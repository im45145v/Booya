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

/// Target loudness for exports and the editor preview (online video).
pub const TARGET_LUFS: f64 = -16.0;
pub const TARGET_TP: f64 = -1.5;
/// Below this the track is treated as silence (mic muted or not working) and
/// is left alone instead of boosting noise by 40 dB.
pub const SILENT_LUFS: f64 = -65.0;

/// Result of a loudnorm measuring pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loudness {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

/// Pull the JSON object loudnorm prints at the end of its stderr.
pub fn parse_loudnorm(stderr: &str) -> Option<Loudness> {
    let end = stderr.rfind('}')?;
    let start = stderr[..end].rfind('{')?;
    let v: serde_json::Value = serde_json::from_str(&stderr[start..=end]).ok()?;
    let num = |k: &str| v[k].as_str().and_then(|s| s.trim().parse::<f64>().ok()).or_else(|| v[k].as_f64());
    let m = Loudness {
        input_i: num("input_i")?,
        input_tp: num("input_tp")?,
        input_lra: num("input_lra")?,
        input_thresh: num("input_thresh")?,
        target_offset: num("target_offset").unwrap_or(0.0),
    };
    // "-inf" parses as -inf: no usable measurement.
    [m.input_i, m.input_tp, m.input_lra, m.input_thresh].iter().all(|x| x.is_finite()).then_some(m)
}

/// Measure the loudness of the audio produced by `graph` (which must end in
/// the label `[aout]`) for the input file `input`.
pub fn measure_graph(input: &Path, graph: &str) -> Result<Option<Loudness>> {
    let full = format!("{graph};[aout]loudnorm=I={TARGET_LUFS}:TP={TARGET_TP}:LRA=11:print_format=json[m]");
    let out = Command::new(crate::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(crate::ffmpeg::file_url(input))
        .args(["-vn", "-filter_complex", &full, "-map", "[m]", "-f", "null", "-"])
        .output()
        .context("failed to run ffmpeg")?;
    if !out.status.success() {
        bail!("loudness measurement failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
    }
    Ok(parse_loudnorm(&String::from_utf8_lossy(&out.stderr)))
}

/// Measure a whole file.
pub fn measure_file(input: &Path) -> Result<Option<Loudness>> {
    measure_graph(input, "[0:a]anull[aout]")
}

/// The normalising filters for a track measured as `m` (empty when the track
/// is silent). Two pass loudnorm is exact and linear (no pumping); the limiter
/// after it catches any peak that still pokes out.
pub fn normalize_chain(m: Option<&Loudness>) -> String {
    match m {
        Some(m) if m.input_i < SILENT_LUFS => String::new(),
        Some(m) => format!(
            "loudnorm=I={TARGET_LUFS}:TP={TARGET_TP}:LRA=11:measured_I={:.2}:measured_TP={:.2}:measured_LRA={:.2}:measured_thresh={:.2}:offset={:.2}:linear=true,alimiter=limit=0.84:level=disabled",
            m.input_i, m.input_tp, m.input_lra, m.input_thresh, m.target_offset
        ),
        None => format!("loudnorm=I={TARGET_LUFS}:TP={TARGET_TP}:LRA=11,alimiter=limit=0.84:level=disabled"),
    }
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

/// Runs of exact digital silence in the middle of sound, as (start_ms,
/// length_ms). Real speech and room noise are never exactly zero, so a run
/// like this is a dropout: samples the recorder lost or replaced with silence.
/// Silence at the very start or end (padding, a muted start) is not reported,
/// and neither is a stretch where the track is quiet on both sides for longer
/// than `quiet_ms` (a muted microphone).
pub fn dropouts(samples: &[f32], rate: u32, min_ms: f64) -> Vec<(f64, f64)> {
    let ms = |n: usize| n as f64 * 1000.0 / rate as f64;
    let min_run = ((min_ms / 1000.0) * rate as f64).ceil() as usize;
    let eps = 1e-6f32;
    let first = samples.iter().position(|v| v.abs() > eps);
    let last = samples.iter().rposition(|v| v.abs() > eps);
    let (Some(first), Some(last)) = (first, last) else { return Vec::new() };
    let mut out = Vec::new();
    let mut i = first;
    while i < last {
        if samples[i].abs() <= eps {
            let mut j = i;
            while j < last && samples[j].abs() <= eps {
                j += 1;
            }
            // Ignore the single zero crossing samples of a clean tone.
            if j - i >= min_run.max(1) {
                out.push((ms(i), ms(j - i)));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
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
mod loudness_tests {
    use super::*;

    #[test]
    fn parses_loudnorm_json() {
        let text = "[Parsed_loudnorm_0 @ 0x1] \n{\n\t\"input_i\" : \"-42.10\",\n\t\"input_tp\" : \"-26.00\",\n\t\"input_lra\" : \"3.20\",\n\t\"input_thresh\" : \"-52.00\",\n\t\"output_i\" : \"-23.0\",\n\t\"target_offset\" : \"0.50\"\n}\n";
        let m = parse_loudnorm(text).unwrap();
        assert_eq!(m.input_i, -42.10);
        assert_eq!(m.target_offset, 0.5);
        assert!(normalize_chain(Some(&m)).contains("measured_I=-42.10"));
        let silent = "{\"input_i\":\"-inf\",\"input_tp\":\"-inf\",\"input_lra\":\"0\",\"input_thresh\":\"-70\"}";
        assert!(parse_loudnorm(silent).is_none());
    }

    #[test]
    fn silent_tracks_are_not_boosted() {
        let m = Loudness { input_i: -70.0, input_tp: -55.0, input_lra: 0.0, input_thresh: -75.0, target_offset: 0.0 };
        assert_eq!(normalize_chain(Some(&m)), "");
    }

    #[test]
    fn a_quiet_track_is_brought_to_the_target() {
        if !crate::ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-loud-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let quiet = dir.join("quiet.wav");
        let ok = Command::new(crate::ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=300:d=4", "-af", "volume=-25dB"])
            .arg(&quiet)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let m = measure_file(&quiet).unwrap().expect("measurable");
        assert!(m.input_i < -35.0, "{}", m.input_i);
        let loud = dir.join("loud.wav");
        let ok = Command::new(crate::ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-i"])
            .arg(&quiet)
            .args(["-af", &normalize_chain(Some(&m))])
            .arg(&loud)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let after = measure_file(&loud).unwrap().expect("measurable");
        assert!((after.input_i - TARGET_LUFS).abs() < 2.0, "{}", after.input_i);
        let _ = std::fs::remove_dir_all(&dir);
    }
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

#[cfg(test)]
mod dropout_tests {
    use super::dropouts;

    #[test]
    fn digital_silence_inside_sound_is_a_dropout_but_padding_is_not() {
        let rate = 16_000;
        let tone = |n: usize| (0..n).map(|i| 0.3 * (i as f32 * 0.3).sin() + 0.05).collect::<Vec<_>>();
        let mut x = vec![0.0; 2 * rate as usize]; // leading silence: not a dropout
        x.extend(tone(rate as usize));
        x.extend(vec![0.0; (0.12 * rate as f64) as usize]); // 120 ms hole
        x.extend(tone(rate as usize));
        x.extend(vec![0.0; rate as usize]); // trailing padding
        let d = dropouts(&x, rate, 20.0);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!((d[0].0 - 3000.0).abs() < 5.0 && (d[0].1 - 120.0).abs() < 5.0, "{d:?}");
        assert!(dropouts(&tone(rate as usize), rate, 20.0).is_empty());
    }
}
