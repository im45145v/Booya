//! Thin wrappers around the ffmpeg / ffprobe command line tools.
//! Set DEMOLENS_FFMPEG / DEMOLENS_FFPROBE to use a bundled copy.

use crate::render::Frame;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

pub fn ffmpeg_bin() -> String {
    std::env::var("DEMOLENS_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}
pub fn ffprobe_bin() -> String {
    std::env::var("DEMOLENS_FFPROBE").unwrap_or_else(|_| "ffprobe".into())
}

#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: f64,
    pub fps: f64,
    pub has_audio: bool,
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let out = Command::new(ffprobe_bin())
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .context("failed to run ffprobe (is ffmpeg installed?)")?;
    if !out.status.success() {
        bail!("ffprobe failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video").context("no video stream")?;
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let fps = video["avg_frame_rate"]
        .as_str()
        .and_then(|r| {
            let (a, b) = r.split_once('/')?;
            let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
            (b > 0.0).then(|| a / b)
        })
        .filter(|f| *f > 0.0)
        .unwrap_or(30.0);
    let duration = v["format"]["duration"]
        .as_str()
        .or(video["duration"].as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    Ok(MediaInfo {
        width: video["width"].as_u64().unwrap_or(0) as u32,
        height: video["height"].as_u64().unwrap_or(0) as u32,
        duration_ms: duration * 1000.0,
        fps,
        has_audio,
    })
}

/// Decode a single frame at `t_ms` (used by the editor preview).
pub fn frame_at(path: &Path, t_ms: f64, w: u32, h: u32) -> Result<Frame> {
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-ss", &format!("{:.3}", t_ms / 1000.0), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-"])
        .output()?;
    let need = (w * h * 4) as usize;
    if out.stdout.len() < need {
        bail!("could not decode frame at {t_ms} ms: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(Frame { w, h, data: out.stdout[..need].to_vec() })
}

/// Load an image scaled to cover (w, h), for image backgrounds.
pub fn load_image_cover(path: &Path, w: u32, h: u32) -> Result<Frame> {
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output()?;
    let need = (w * h * 4) as usize;
    if out.stdout.len() < need {
        bail!("could not load background image: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(Frame { w, h, data: out.stdout[..need].to_vec() })
}

/// Sequential decoder producing RGBA frames at a fixed rate.
pub struct FrameReader {
    child: Child,
    stdout: ChildStdout,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub index: u64,
}

impl FrameReader {
    pub fn open(path: &Path, start_ms: f64, fps: f64, w: u32, h: u32) -> Result<Self> {
        let mut child = Command::new(ffmpeg_bin())
            .args(["-v", "error", "-ss", &format!("{:.3}", start_ms / 1000.0), "-i"])
            .arg(path)
            .args(["-an", "-vf", &format!("fps={fps},scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start ffmpeg decoder")?;
        let stdout = child.stdout.take().unwrap();
        Ok(Self { child, stdout, w, h, fps, index: 0 })
    }

    pub fn next(&mut self) -> Result<Option<Frame>> {
        let mut f = Frame::new(self.w, self.h);
        match self.stdout.read_exact(&mut f.data) {
            Ok(()) => {
                self.index += 1;
                Ok(Some(f))
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for FrameReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
