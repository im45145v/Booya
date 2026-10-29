//! The parts of in-process audio capture that need no audio device, so they
//! can be tested anywhere: a float WAV writer, silence padding for devices
//! that stop delivering (a loopback with nothing playing), and the live level
//! that the editor uses to warn when the microphone is silent.

use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Interleaved 32 bit float PCM in a WAV file. The sizes are patched in by
/// `finish`, so a crash leaves a file ffmpeg can still read (it ignores a
/// wrong size and reads to the end).
pub struct WavWriter<W: Write + Seek> {
    w: W,
    channels: u16,
    rate: u32,
    frames: u64,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut w: W, rate: u32, channels: u16) -> std::io::Result<Self> {
        w.write_all(&header(rate, channels, 0))?;
        Ok(Self { w, channels, rate, frames: 0 })
    }

    pub fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.w.write_all(&bytes)?;
        self.frames += (samples.len() / self.channels.max(1) as usize) as u64;
        Ok(())
    }

    pub fn write_silence(&mut self, frames: usize) -> std::io::Result<()> {
        let chunk = vec![0u8; 4096 * self.channels as usize * 4];
        let mut left = frames as u64 * self.channels as u64 * 4;
        while left > 0 {
            let n = left.min(chunk.len() as u64) as usize;
            self.w.write_all(&chunk[..n])?;
            left -= n as u64;
        }
        self.frames += frames as u64;
        Ok(())
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn finish(mut self) -> std::io::Result<u64> {
        self.w.seek(SeekFrom::Start(0))?;
        self.w.write_all(&header(self.rate, self.channels, self.frames))?;
        self.w.flush()?;
        Ok(self.frames)
    }
}

fn header(rate: u32, channels: u16, frames: u64) -> [u8; 44] {
    let data = (frames * channels as u64 * 4).min(u32::MAX as u64 - 40) as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&3u16.to_le_bytes()); // IEEE float
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * channels as u32 * 4).to_le_bytes());
    h[32..34].copy_from_slice(&(channels * 4).to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// Keeps a file the length of the wall clock. A device that delivers nothing
/// for a while (a loopback while nothing plays, a mic that stalls) would
/// otherwise make the file shorter than the video and shift everything after
/// the gap.
#[derive(Debug, Default)]
pub struct GapFiller {
    rate: u32,
    first_ms: Option<f64>,
    written: u64,
}

/// A gap shorter than this is jitter, not a stall.
const GAP_S: f64 = 0.15;

impl GapFiller {
    pub fn new(rate: u32) -> Self {
        Self { rate, first_ms: None, written: 0 }
    }

    /// `now_ms` is when a buffer of `frames` frames has just arrived. Returns
    /// the frames of silence to write before it.
    pub fn before(&mut self, now_ms: f64, frames: usize) -> usize {
        let rate = self.rate as f64;
        // The first buffer started `frames` ago.
        let first = *self.first_ms.get_or_insert(now_ms - frames as f64 * 1000.0 / rate);
        let expected = ((now_ms - first) / 1000.0 * rate).round() as i64;
        let behind = expected - self.written as i64 - frames as i64;
        let pad = if behind as f64 > GAP_S * rate { behind.min((600.0 * rate) as i64) as usize } else { 0 };
        self.written += (pad + frames) as u64;
        pad
    }

    /// When the first buffer started, on the caller's clock.
    pub fn first_ms(&self) -> Option<f64> {
        self.first_ms
    }
}

/// Live level and data count, shared between the audio thread and the rest.
#[derive(Debug, Default)]
pub struct Probe {
    peak_bits: AtomicU32,
    frames: AtomicU64,
}

impl Probe {
    pub fn feed(&self, samples: &[f32], frames: usize) {
        let mut p = f32::from_bits(self.peak_bits.load(Ordering::Relaxed));
        for s in samples {
            p = p.max(s.abs());
        }
        self.peak_bits.store(p.to_bits(), Ordering::Relaxed);
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
    }

    /// Highest sample so far, 0..1.
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.load(Ordering::Relaxed))
    }

    pub fn peak_db(&self) -> f32 {
        let p = self.peak();
        if p <= 1e-5 { -100.0 } else { 20.0 * p.log10() }
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

/// Peak below this, after a second or two, is digital silence: no real
/// microphone is that quiet, a blocked or muted one is exactly zero.
pub const SILENT_DB: f32 = -80.0;

/// Turns what ScreenCaptureKit hands over into interleaved stereo f32. Each
/// item is one buffer's bytes (little endian f32) and its channel count.
/// ScreenCaptureKit gives either one buffer per channel (planar) or a single
/// interleaved buffer; mono is copied to both sides. Anything else is skipped.
pub fn interleave_stereo(buffers: &[(&[u8], usize)]) -> Vec<f32> {
    let f = |b: &[u8]| -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() };
    match buffers {
        [] => Vec::new(),
        [(b, ch)] => {
            let v = f(b);
            match ch {
                2 => v,
                1 => v.iter().flat_map(|x| [*x, *x]).collect(),
                _ => Vec::new(),
            }
        }
        [(l, 1), (r, 1), ..] => {
            let (l, r) = (f(l), f(r));
            l.iter().zip(r.iter()).flat_map(|(a, b)| [*a, *b]).collect()
        }
        _ => Vec::new(),
    }
}

/// Converts a float WAV into the Matroska sidecar the recording is joined
/// with: the samples are stamped `offset_s` seconds after the recording clock
/// started (when the first sample was taken).
pub fn wav_to_mkv(wav: &std::path::Path, out: &std::path::Path, offset_s: f64) -> anyhow::Result<()> {
    let st = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-v", "error", "-y", "-itsoffset", &format!("{:.3}", offset_s), "-i"])
        .arg(wav)
        .args(["-c:a", "pcm_f32le", "-f", "matroska"])
        .arg(out)
        .output()?;
    if !st.status.success() {
        anyhow::bail!("ffmpeg could not convert the audio: {}", String::from_utf8_lossy(&st.stderr).trim());
    }
    let _ = std::fs::remove_file(wav);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn the_wav_header_is_patched_and_ffmpeg_sized() {
        let mut w = WavWriter::new(Cursor::new(Vec::new()), 48_000, 2).unwrap();
        w.write(&[0.5, -0.5, 0.25, -0.25]).unwrap();
        w.write_silence(3).unwrap();
        assert_eq!(w.frames(), 5);
        let cur = {
            let WavWriter { mut w, rate, channels, frames } = w;
            w.seek(SeekFrom::Start(0)).unwrap();
            w.write_all(&header(rate, channels, frames)).unwrap();
            w
        };
        let b = cur.into_inner();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 5 * 2 * 4);
        assert_eq!(b.len(), 44 + 40);
        assert_eq!(f32::from_le_bytes(b[44..48].try_into().unwrap()), 0.5);
        assert_eq!(u16::from_le_bytes(b[20..22].try_into().unwrap()), 3);
    }

    #[test]
    fn a_stall_is_filled_with_silence_and_jitter_is_not() {
        let mut g = GapFiller::new(48_000);
        // 10 ms buffers arriving on time.
        assert_eq!(g.before(10.0, 480), 0);
        assert_eq!(g.before(20.0, 480), 0);
        assert_eq!(g.before(31.0, 480), 0, "one ms of jitter");
        // Nothing for a second, then a buffer.
        let pad = g.before(1041.0, 480);
        assert!((pad as i64 - 47_520).abs() < 600, "padded {pad}");
        // Back to normal.
        assert_eq!(g.before(1051.0, 480), 0);
    }

    #[test]
    fn the_probe_sees_silence_and_sound() {
        let p = Probe::default();
        p.feed(&[0.0; 480], 480);
        assert!(p.peak_db() < SILENT_DB && p.frames() == 480);
        p.feed(&[0.0, 0.1, -0.2], 3);
        assert!((p.peak_db() + 13.98).abs() < 0.1, "{}", p.peak_db());
    }

    #[test]
    fn screencapturekit_buffers_become_stereo() {
        let bytes = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let (l, r) = (bytes(&[0.1, 0.2]), bytes(&[-0.1, -0.2]));
        assert_eq!(interleave_stereo(&[(&l, 1), (&r, 1)]), vec![0.1, -0.1, 0.2, -0.2], "planar");
        let both = bytes(&[0.1, -0.1, 0.2, -0.2]);
        assert_eq!(interleave_stereo(&[(&both, 2)]), vec![0.1, -0.1, 0.2, -0.2], "interleaved");
        assert_eq!(interleave_stereo(&[(&l, 1)]), vec![0.1, 0.1, 0.2, 0.2], "mono");
        assert!(interleave_stereo(&[]).is_empty() && interleave_stereo(&[(&l, 6)]).is_empty());
        assert_eq!(interleave_stereo(&[(&[1u8, 2, 3][..], 1), (&[1u8][..], 1)]), Vec::<f32>::new(), "a torn buffer yields nothing");
    }
}
