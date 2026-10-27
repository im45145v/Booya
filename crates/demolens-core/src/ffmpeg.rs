//! Thin wrappers around the ffmpeg / ffprobe command line tools.
//! The binaries are found by `candidates()`; DEMOLENS_FFMPEG / DEMOLENS_FFPROBE override.

use crate::render::Frame;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

pub fn ffmpeg_bin() -> String {
    resolve("DEMOLENS_FFMPEG", "ffmpeg")
}
pub fn ffprobe_bin() -> String {
    resolve("DEMOLENS_FFPROBE", "ffprobe")
}

/// True when a working ffmpeg was found.
pub fn ffmpeg_found() -> bool {
    runs(Path::new(&ffmpeg_bin()))
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn runs(p: &Path) -> bool {
    let mut c = Command::new(p);
    c.arg("-version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.status().map(|s| s.success()).unwrap_or(false)
}

/// Every place we look, in order: the env override, the copy bundled next to
/// the app (`demolens-ffmpeg`), PATH, then common install folders, because GUI
/// apps on macOS do not inherit the shell PATH.
pub fn candidates(env: &str, name: &str) -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut v = Vec::new();
    if let Ok(p) = std::env::var(env) {
        v.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        v.push(dir.join(exe_name(&format!("demolens-{name}"))));
        v.push(dir.join(exe_name(name)));
        // macOS bundle: Contents/MacOS/<app>, Linux AppImage/deb: usr/bin
        v.push(dir.join("../Resources").join(exe_name(name)));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            v.push(d.join(exe_name(name)));
        }
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/opt/local/bin", "/snap/bin"] {
        v.push(Path::new(d).join(name));
    }
    if cfg!(windows) {
        for d in [r"C:\ffmpeg\bin", r"C:\Program Files\ffmpeg\bin", r"C:\ProgramData\chocolatey\bin"] {
            v.push(Path::new(d).join(exe_name(name)));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            v.push(Path::new(&local).join(r"Microsoft\WinGet\Links").join(exe_name(name)));
        }
    }
    v
}

fn resolve(env: &'static str, name: &'static str) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<&'static str, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(p) = cache.lock().unwrap().get(name) {
        return p.clone();
    }
    let found = candidates(env, name).into_iter().find(|p| p.is_file() && runs(p));
    match found {
        Some(p) => {
            let s = p.to_string_lossy().into_owned();
            cache.lock().unwrap().insert(name, s.clone());
            s
        }
        // not cached, so installing ffmpeg while the app runs starts working
        None => name.to_string(),
    }
}

/// One line install command for this OS, shown when ffmpeg is missing.
pub fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "brew install ffmpeg"
    } else if cfg!(windows) {
        "winget install Gyan.FFmpeg"
    } else {
        "sudo apt install ffmpeg   (Fedora: sudo dnf install ffmpeg)"
    }
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

/// Which H.264 encoder to use. Hardware encoders (VideoToolbox on macOS,
/// NVENC on NVIDIA) move the work off the CPU, which matters most for
/// battery life on laptops. Software x264 is the fallback everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    VideoToolbox,
    Nvenc,
    X264,
}

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Encoder::VideoToolbox => "h264_videotoolbox",
            Encoder::Nvenc => "h264_nvenc",
            Encoder::X264 => "libx264",
        }
    }
    pub fn is_hardware(self) -> bool {
        self != Encoder::X264
    }

    /// Encoder arguments for a finished export (quality first).
    pub fn export_args(self, w: u32, h: u32, fps: f64) -> Vec<String> {
        let v: Vec<&str> = match self {
            // Bitrate sized to the frame: about 12 Mbit/s at 1080p30.
            Encoder::VideoToolbox => vec!["-c:v", "h264_videotoolbox", "-profile:v", "high", "-allow_sw", "1"],
            Encoder::Nvenc => vec!["-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "19", "-b:v", "0"],
            // veryfast at CRF 18 measured about half the CPU of medium with no
            // visible loss for screen content (see docs/EFFICIENCY.md).
            Encoder::X264 => vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"],
        };
        let mut a: Vec<String> = v.into_iter().map(String::from).collect();
        if self == Encoder::VideoToolbox {
            let bits = (w as f64 * h as f64 * fps * 0.19).max(2e6) as u64;
            a.extend(["-b:v".into(), bits.to_string()]);
        }
        a
    }

    /// Encoder arguments while recording (speed first, near lossless).
    pub fn capture_args(self) -> Vec<String> {
        let v: Vec<&str> = match self {
            Encoder::VideoToolbox => vec!["-c:v", "h264_videotoolbox", "-realtime", "1", "-allow_sw", "1", "-b:v", "40M"],
            Encoder::Nvenc => vec!["-c:v", "h264_nvenc", "-preset", "p1", "-tune", "ll", "-rc", "constqp", "-qp", "18"],
            Encoder::X264 => vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "16"],
        };
        v.into_iter().map(String::from).collect()
    }
}

/// Parse DEMOLENS_ENCODER: "software"/"x264", "videotoolbox", "nvenc" or auto.
pub fn encoder_override(v: Option<&str>) -> Option<Encoder> {
    match v?.trim().to_ascii_lowercase().as_str() {
        "software" | "x264" | "libx264" | "cpu" => Some(Encoder::X264),
        "videotoolbox" | "h264_videotoolbox" => Some(Encoder::VideoToolbox),
        "nvenc" | "h264_nvenc" => Some(Encoder::Nvenc),
        _ => None,
    }
}

/// True when this ffmpeg can actually open `name` on this machine (being
/// compiled in is not enough: NVENC needs a GPU and driver).
pub fn encoder_works(name: &str) -> bool {
    let mut c = Command::new(ffmpeg_bin());
    c.args(["-v", "error", "-f", "lavfi", "-i", "color=c=black:s=256x256:r=30:d=0.2", "-pix_fmt", "yuv420p", "-c:v", name, "-f", "null", "-"]);
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.status().map(|s| s.success()).unwrap_or(false)
}

/// The best working H.264 encoder, probed once per process.
pub fn best_encoder() -> Encoder {
    static BEST: std::sync::OnceLock<Encoder> = std::sync::OnceLock::new();
    if let Some(e) = encoder_override(std::env::var("DEMOLENS_ENCODER").ok().as_deref()) {
        return e;
    }
    *BEST.get_or_init(|| {
        let order: &[Encoder] = if cfg!(target_os = "macos") { &[Encoder::VideoToolbox] } else { &[Encoder::Nvenc] };
        order.iter().copied().find(|e| encoder_works(e.name())).unwrap_or(Encoder::X264)
    })
}

#[cfg(test)]
mod encoder_tests {
    use super::*;
    #[test]
    fn override_parsing() {
        assert_eq!(encoder_override(Some("software")), Some(Encoder::X264));
        assert_eq!(encoder_override(Some(" VideoToolbox ")), Some(Encoder::VideoToolbox));
        assert_eq!(encoder_override(Some("auto")), None);
        assert_eq!(encoder_override(None), None);
    }
    #[test]
    fn export_args_are_sane() {
        let a = Encoder::X264.export_args(1920, 1080, 30.0);
        assert!(a.windows(2).any(|w| w[0] == "-preset" && w[1] == "veryfast"));
        let v = Encoder::VideoToolbox.export_args(1920, 1080, 30.0);
        let i = v.iter().position(|s| s == "-b:v").unwrap();
        let bits: u64 = v[i + 1].parse().unwrap();
        assert!((10_000_000..14_000_000).contains(&bits));
    }
    #[test]
    fn missing_encoder_is_rejected() {
        if !crate::ffmpeg::ffmpeg_found() {
            return;
        }
        assert!(!encoder_works("definitely_not_an_encoder"));
    }
}
