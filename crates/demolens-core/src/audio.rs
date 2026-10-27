//! Audio helpers: decoding through ffmpeg, RNNoise noise cleanup
//! (nnnoiseless, pure Rust, no model download) and silence detection.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Decode the first audio stream to mono f32 samples at `rate` Hz.
pub fn decode_mono(path: &Path, rate: u32) -> Result<Vec<f32>> {
    let out = Command::new(crate::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(crate::ffmpeg::file_url(path))
        .args(["-vn", "-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "-"])
        .output()
        .context("failed to run ffmpeg")?;
    if !out.status.success() {
        bail!("audio decode failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

/// Write mono f32 samples as a 32-bit float WAV file.
pub fn write_wav_f32(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data_len = (samples.len() * 4) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?; // IEEE float
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

/// RNNoise noise suppression on 48 kHz mono audio.
pub fn denoise(samples: &[f32]) -> Vec<f32> {
    use nnnoiseless::DenoiseState;
    const N: usize = DenoiseState::FRAME_SIZE;
    let mut st = DenoiseState::new();
    let mut out = Vec::with_capacity(samples.len() + N);
    let mut inb = [0f32; N];
    let mut outb = [0f32; N];
    let mut first = true;
    for chunk in samples.chunks(N) {
        for (i, v) in inb.iter_mut().enumerate() {
            // RNNoise expects 16-bit range samples.
            *v = chunk.get(i).copied().unwrap_or(0.0) * 32768.0;
        }
        st.process_frame(&mut outb, &inb);
        if !first {
            out.extend(outb.iter().map(|v| (v / 32768.0).clamp(-1.0, 1.0)));
        } else {
            // The first frame contains a fade-in artifact; keep it silent.
            out.extend(std::iter::repeat(0.0).take(N));
            first = false;
        }
    }
    out.truncate(samples.len());
    out
}

/// Denoise the audio of `video` into a WAV file usable by the exporter.
pub fn denoise_file(video: &Path, out_wav: &Path) -> Result<()> {
    let s = decode_mono(video, 48_000)?;
    write_wav_f32(out_wav, &denoise(&s), 48_000)
}

/// Find silent stretches: windows of 20 ms whose RMS is below
/// `threshold_db` dBFS, at least `min_ms` long. Returns (start_ms, end_ms).
pub fn detect_silences(samples: &[f32], rate: u32, threshold_db: f32, min_ms: f64) -> Vec<(f64, f64)> {
    let win = (rate as usize / 50).max(1);
    let thr = 10f32.powf(threshold_db / 20.0);
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let n_windows = samples.len().div_ceil(win);
    for w in 0..=n_windows {
        let silent = if w < n_windows {
            let c = &samples[w * win..((w + 1) * win).min(samples.len())];
            let rms = (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
            rms < thr
        } else {
            false
        };
        match (silent, start) {
            (true, None) => start = Some(w),
            (false, Some(s)) => {
                let a = s as f64 * win as f64 * 1000.0 / rate as f64;
                let b = (w * win).min(samples.len()) as f64 * 1000.0 / rate as f64;
                if b - a >= min_ms {
                    out.push((a, b));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: usize, rate: u32, amp: f32) -> Vec<f32> {
        (0..ms * rate as usize / 1000).map(|i| amp * (i as f32 * 0.05).sin()).collect()
    }

    #[test]
    fn finds_silence_between_tones() {
        let rate = 16_000;
        let mut s = tone(500, rate, 0.5);
        s.extend(vec![0.0; rate as usize]); // 1 s silence
        s.extend(tone(500, rate, 0.5));
        let sil = detect_silences(&s, rate, -40.0, 300.0);
        assert_eq!(sil.len(), 1);
        assert!((sil[0].0 - 500.0).abs() <= 20.0);
        assert!((sil[0].1 - 1500.0).abs() <= 20.0);
        assert!(detect_silences(&s, rate, -40.0, 1500.0).is_empty());
    }

    #[test]
    fn denoise_reduces_noise_energy() {
        // Deterministic white-ish noise.
        let mut x: u32 = 1;
        let noise: Vec<f32> = (0..48_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * 0.1
            })
            .collect();
        let out = denoise(&noise);
        assert_eq!(out.len(), noise.len());
        let e = |v: &[f32]| v[4800..].iter().map(|s| s * s).sum::<f32>();
        assert!(e(&out) < e(&noise) * 0.5);
    }
}
