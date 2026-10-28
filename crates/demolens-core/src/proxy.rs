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
