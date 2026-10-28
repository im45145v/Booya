//! Editing proxies: a small (720p) copy of a recording that the editor decodes
//! and plays instead of the full resolution original.
//!
//! A 2940x1912 recording is expensive to decode on every preview frame. The
//! proxy is built once in the background (VideoToolbox/NVENC when available),
//! keeps a keyframe every half second so seeking is cheap, and carries the
//! audio already normalised to -16 LUFS so the editor can play it with the
//! browser's own audio element. Exports always read the original.

use crate::ffmpeg::{self, Encoder};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

/// Height of the proxy in pixels.
pub const PROXY_HEIGHT: u32 = 720;

/// Stable cache key for a source file: path, size and modified time.
pub fn cache_key(src: &Path) -> String {
    let meta = std::fs::metadata(src).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // FNV-1a: tiny and stable across Rust versions.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in src.to_string_lossy().bytes().chain(size.to_le_bytes()).chain(mtime.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn proxy_path(cache_dir: &Path, src: &Path) -> PathBuf {
    cache_dir.join("proxies").join(format!("{}.mp4", cache_key(src)))
}

/// The finished proxy for `src`, if one exists.
pub fn existing(cache_dir: &Path, src: &Path) -> Option<PathBuf> {
    let p = proxy_path(cache_dir, src);
    p.metadata().ok().filter(|m| m.len() > 1024).map(|_| p)
}

/// Size of a proxy for a source of `sw`x`sh`, even numbers, height capped at
/// [`PROXY_HEIGHT`] (never upscaled).
pub fn proxy_size(sw: u32, sh: u32) -> (u32, u32) {
    if sh <= PROXY_HEIGHT {
        return (sw & !1, sh & !1);
    }
    let w = (sw as f64 * PROXY_HEIGHT as f64 / sh as f64).round() as u32;
    ((w + 1) & !1, PROXY_HEIGHT)
}

/// True when a proxy of `proxy` pixels can stand in for the source at a
/// decode size of `want` (it must not be smaller than what the renderer wants).
pub fn fits(proxy: (u32, u32), want: (u32, u32)) -> bool {
    proxy.0 >= want.0 && proxy.1 >= want.1
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyStats {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
    pub encoder: String,
    pub hwaccel_decode: bool,
    /// True when it was already there and nothing was done.
    pub cached: bool,
}

fn encode_args(enc: Encoder) -> Vec<&'static str> {
    match enc {
        Encoder::VideoToolbox => vec!["-c:v", "h264_videotoolbox", "-b:v", "6M", "-allow_sw", "1", "-g", "15"],
        Encoder::Nvenc => vec!["-c:v", "h264_nvenc", "-preset", "p1", "-rc", "vbr", "-cq", "28", "-b:v", "0", "-g", "15"],
        Encoder::X264 => vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "26", "-g", "15"],
    }
}

/// Build the proxy (or return the cached one). `progress` gets 0..=1.
pub fn build(cache_dir: &Path, src: &Path, mut progress: impl FnMut(f64)) -> Result<ProxyStats> {
    let info = ffmpeg::probe(src).context("probe source")?;
    let (w, h) = proxy_size(info.width, info.height);
    if let Some(p) = existing(cache_dir, src) {
        progress(1.0);
        return Ok(ProxyStats { path: p, width: w, height: h, seconds: 0.0, encoder: String::new(), hwaccel_decode: false, cached: true });
    }
    let out = proxy_path(cache_dir, src);
    std::fs::create_dir_all(out.parent().unwrap())?;
    let part = out.with_extension("mp4.part");
    let _ = std::fs::remove_file(&part);

    // Loudness of the original, so the proxy's audio is already at the target.
    let norm = if info.has_audio { crate::audio::normalize_chain(crate::audio::measure_file(src).ok().flatten().as_ref()) } else { String::new() };

    let enc = ffmpeg::best_encoder();
    let hw = ffmpeg::hwaccel_args();
    let t0 = Instant::now();
    let mut run = |enc: Encoder, hw: &[&str]| -> Result<()> {
        let mut c = crate::proc::command(ffmpeg::ffmpeg_bin());
        c.args(["-v", "error", "-nostdin", "-y", "-progress", "pipe:1"]);
        c.args(hw);
        c.arg("-i").arg(ffmpeg::file_url(src));
        let fps = if info.fps > 30.5 { 30.0 } else { info.fps.max(1.0) };
        c.args(["-vf", &format!("scale={w}:{h}:flags=bilinear,fps={fps}"), "-pix_fmt", "yuv420p"]);
        c.args(encode_args(enc));
        if info.has_audio {
            if !norm.is_empty() {
                c.args(["-af", &norm]);
            }
            c.args(["-c:a", "aac", "-b:a", "128k", "-ac", "2"]);
        } else {
            c.arg("-an");
        }
        c.args(["-movflags", "+faststart", "-f", "mp4"]).arg(&part);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = c.spawn().context("spawn ffmpeg")?;
        let stdout = child.stdout.take().unwrap();
        let total = info.duration_ms.max(1.0);
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                progress((us / 1000.0 / total).clamp(0.0, 0.99));
            }
        }
        let o = child.wait_with_output()?;
        if !o.status.success() {
            bail!("proxy encode failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
        }
        Ok(())
    };
    // Hardware first; if that fails for any reason fall back to the safe path.
    let mut used_hw = !hw.is_empty();
    if let Err(first) = run(enc, &hw) {
        eprintln!("proxy: {first}; retrying in software");
        let _ = std::fs::remove_file(&part);
        used_hw = false;
        run(Encoder::X264, &[])?;
    }
    let got = ffmpeg::probe(&part).context("verify proxy")?;
    if got.duration_ms + 1500.0 < info.duration_ms {
        let _ = std::fs::remove_file(&part);
        bail!("proxy is shorter than the source ({:.1}s vs {:.1}s)", got.duration_ms / 1000.0, info.duration_ms / 1000.0);
    }
    std::fs::rename(&part, &out)?;
    progress(1.0);
    Ok(ProxyStats { path: out, width: w, height: h, seconds: t0.elapsed().as_secs_f64(), encoder: enc.name().into(), hwaccel_decode: used_hw, cached: false })
}

// ---------- Preview sound ----------
//
// The editor does not play the sound out of the 720p video proxy: that file
// takes as long to build as the video is long, and until it exists the editor
// would be silent. The sound gets its own small file, built in seconds, with
// the loudness already evened out, 48 kHz stereo AAC so every web view plays it.

/// Length of one waveform bucket in the peaks the editor draws.
pub const PEAK_BUCKET_MS: f64 = 50.0;

#[derive(Debug, Clone, Serialize)]
pub struct AudioPreview {
    /// False when the recording has no sound track at all.
    pub has_audio: bool,
    pub path: Option<PathBuf>,
    pub duration_ms: f64,
    pub bucket_ms: f64,
    /// Waveform: the loudest sample in each bucket, 0 to 255.
    pub peaks: Vec<u8>,
    /// True when the track is only a noise floor (nothing to hear).
    pub faint: bool,
}

pub fn audio_path(cache_dir: &Path, src: &Path) -> PathBuf {
    cache_dir.join("proxies").join(format!("{}.audio.m4a", cache_key(src)))
}

pub fn audio_wav_path(cache_dir: &Path, src: &Path) -> PathBuf {
    cache_dir.join("proxies").join(format!("{}.audio.wav", cache_key(src)))
}

/// The same preview sound as plain 16 bit PCM in a WAV file (mono, 24 kHz,
/// loudness evened out the same way). Every web view plays it, so it is the
/// fallback when a web view cannot decode the AAC file (macOS WKWebView in
/// particular). About 3 MB a minute, built only when the AAC one fails.
pub fn build_audio_wav(cache_dir: &Path, src: &Path) -> Result<PathBuf> {
    let out = audio_wav_path(cache_dir, src);
    if out.metadata().map(|m| m.len() > 512).unwrap_or(false) {
        return Ok(out);
    }
    std::fs::create_dir_all(out.parent().unwrap())?;
    let part = out.with_extension("wav.part");
    let _ = std::fs::remove_file(&part);
    let loud = crate::audio::measure_file(src).ok().flatten();
    let norm = crate::audio::normalize_chain(loud.as_ref());
    let mut c = crate::proc::command(ffmpeg::ffmpeg_bin());
    c.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(ffmpeg::file_url(src)).arg("-vn");
    if !norm.is_empty() {
        c.args(["-af", &norm]);
    }
    c.args(["-c:a", "pcm_s16le", "-ac", "1", "-ar", "24000", "-f", "wav"]).arg(&part);
    let o = c.output().context("run ffmpeg")?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&part);
        bail!("could not build the compatible preview sound: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
    }
    std::fs::rename(&part, &out)?;
    Ok(out)
}

/// Peaks of mono samples at `rate`: the loudest value per bucket, scaled so the
/// loudest bucket is 255 (square rooted, so quiet speech is still visible).
pub fn peaks(samples: &[f32], rate: u32) -> Vec<u8> {
    let n = ((PEAK_BUCKET_MS / 1000.0) * rate as f64).max(1.0) as usize;
    let raw: Vec<f32> = samples.chunks(n).map(|c| c.iter().fold(0.0f32, |m, x| m.max(x.abs()))).collect();
    let top = raw.iter().cloned().fold(0.0f32, f32::max);
    if top <= 1e-5 {
        return vec![0; raw.len()];
    }
    raw.iter().map(|v| ((v / top).sqrt() * 255.0).round().clamp(0.0, 255.0) as u8).collect()
}

/// Build (or reuse) the preview sound for `src`.
pub fn build_audio(cache_dir: &Path, src: &Path) -> Result<AudioPreview> {
    let info = ffmpeg::probe(src).context("probe source")?;
    if !info.has_audio {
        return Ok(AudioPreview { has_audio: false, path: None, duration_ms: info.duration_ms, bucket_ms: PEAK_BUCKET_MS, peaks: vec![], faint: false });
    }
    let samples = crate::audio::decode_mono(src, 8000)?;
    let pk = peaks(&samples, 8000);
    let loud = crate::audio::measure_file(src).ok().flatten();
    let faint = loud.map(|l| l.input_i < crate::audio::SILENT_LUFS).unwrap_or(true);
    let out = audio_path(cache_dir, src);
    if out.metadata().map(|m| m.len() > 512).unwrap_or(false) {
        return Ok(AudioPreview { has_audio: true, path: Some(out), duration_ms: info.duration_ms, bucket_ms: PEAK_BUCKET_MS, peaks: pk, faint });
    }
    std::fs::create_dir_all(out.parent().unwrap())?;
    let part = out.with_extension("m4a.part");
    let _ = std::fs::remove_file(&part);
    let norm = crate::audio::normalize_chain(loud.as_ref());
    let mut c = crate::proc::command(ffmpeg::ffmpeg_bin());
    c.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(ffmpeg::file_url(src)).arg("-vn");
    if !norm.is_empty() {
        c.args(["-af", &norm]);
    }
    c.args(["-c:a", "aac", "-b:a", "128k", "-ac", "2", "-ar", "48000", "-movflags", "+faststart", "-f", "mp4"]).arg(&part);
    let o = c.output().context("run ffmpeg")?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&part);
        bail!("could not build the preview sound: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
    }
    std::fs::rename(&part, &out)?;
    Ok(AudioPreview { has_audio: true, path: Some(out), duration_ms: info.duration_ms, bucket_ms: PEAK_BUCKET_MS, peaks: pk, faint })
}

/// Delete proxies not used for `max_age_days`, returning the bytes freed.
pub fn prune(cache_dir: &Path, max_age_days: u64) -> u64 {
    let mut freed = 0;
    let Ok(rd) = std::fs::read_dir(cache_dir.join("proxies")) else { return 0 };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let age = m.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() / 86400).unwrap_or(0);
        if age >= max_age_days && std::fs::remove_file(e.path()).is_ok() {
            freed += m.len();
        }
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_are_scaled_to_the_loudest_bucket() {
        let mut s = vec![0.0f32; 400];
        s[10] = 0.5;
        s[210] = 0.125;
        let p = peaks(&s, 8000);
        assert_eq!(p.len(), 1);
        let mut s2 = vec![0.0f32; 800];
        s2[10] = 0.5;
        s2[410] = 0.125;
        let p = peaks(&s2, 8000);
        assert_eq!(p, vec![255, 128]);
        assert_eq!(peaks(&[0.0; 800], 8000), vec![0, 0]);
    }

    #[test]
    fn preview_sound_is_48k_stereo_aac_and_has_a_waveform() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-asound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.mp4");
        let ok = crate::proc::command(ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=320x240:r=10:d=3", "-f", "lavfi", "-i", "sine=frequency=300:d=3", "-af", "volume=-14dB", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let r = build_audio(&dir, &src).unwrap();
        assert!(r.has_audio && !r.faint);
        let p = r.path.clone().unwrap();
        assert!(p.metadata().unwrap().len() > 1000);
        assert!((r.peaks.len() as i64 - 60).abs() <= 2, "{}", r.peaks.len());
        assert!(r.peaks.iter().filter(|v| **v > 200).count() > 40, "a steady tone fills the waveform");
        let out = crate::proc::command(ffmpeg::ffprobe_bin()).args(["-v", "error", "-show_entries", "stream=codec_name,sample_rate,channels,duration", "-of", "json"]).arg(&p).output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let st = &v["streams"][0];
        assert_eq!(st["codec_name"], "aac");
        assert_eq!(st["sample_rate"], "48000");
        assert_eq!(st["channels"], 2);
        assert!((st["duration"].as_str().unwrap().parse::<f64>().unwrap() - 3.0).abs() < 0.2);
        // Built once, reused the second time.
        let again = build_audio(&dir, &src).unwrap();
        assert_eq!(again.path, r.path);
        // No sound track: no file, not an error.
        let mute = dir.join("m.mp4");
        assert!(crate::proc::command(ffmpeg::ffmpeg_bin()).args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=320x240:r=10:d=2", "-c:v", "libx264", "-pix_fmt", "yuv420p"]).arg(&mute).status().unwrap().success());
        let m = build_audio(&dir, &mute).unwrap();
        assert!(!m.has_audio && m.path.is_none() && m.peaks.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_compatible_preview_sound_is_plain_pcm_wav() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-awav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.mp4");
        assert!(crate::proc::command(ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=320x240:r=10:d=2", "-f", "lavfi", "-i", "sine=frequency=300:d=2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&src)
            .status()
            .unwrap()
            .success());
        let p = build_audio_wav(&dir, &src).unwrap();
        assert!(p.to_string_lossy().ends_with(".audio.wav"));
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(&b[8..12], b"WAVE");
        assert!(b.len() > 24_000 * 2 * 2 - 4000, "{}", b.len());
        // Built once, reused.
        assert_eq!(build_audio_wav(&dir, &src).unwrap(), p);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_are_even_and_never_upscaled() {
        assert_eq!(proxy_size(2940, 1912), (1108, 720));
        assert_eq!(proxy_size(1920, 1080), (1280, 720));
        assert_eq!(proxy_size(640, 360), (640, 360));
        assert!(fits((1280, 720), (1000, 600)));
        assert!(!fits((1280, 720), (1920, 1080)));
    }

    #[test]
    fn builds_and_reuses_a_proxy_with_loud_audio() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("dl-proxy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("big.mp4");
        let ok = crate::proc::command(ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=2940x1912:r=10:d=2", "-f", "lavfi", "-i", "sine=frequency=300:d=2", "-af", "volume=-14dB"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let s = build(&dir, &src, |_| {}).unwrap();
        assert!(!s.cached);
        assert_eq!((s.width, s.height), (1108, 720));
        let info = ffmpeg::probe(&s.path).unwrap();
        assert_eq!((info.width, info.height), (1108, 720));
        assert!(info.has_audio);
        let loud = crate::audio::measure_file(&s.path).unwrap().unwrap().input_i;
        assert!((loud - crate::audio::TARGET_LUFS).abs() < 3.0, "{loud}");
        assert!(build(&dir, &src, |_| {}).unwrap().cached);
        // Seeking stays cheap: a keyframe at least every 15 frames.
        let probe = crate::proc::command("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=flags", "-of", "csv=p=0"])
            .arg(&s.path)
            .output();
        if let Some(o) = probe.ok().filter(|o| o.status.success() && !o.stdout.is_empty()) {
            let (mut gap, mut worst) = (0, 0);
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                if l.starts_with('K') {
                    worst = worst.max(gap);
                    gap = 0;
                }
                gap += 1;
            }
            assert!(worst <= 15, "largest keyframe gap {worst}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
