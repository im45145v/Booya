//! Screen capture. Each OS backend builds an ffmpeg capture command using the
//! platform's native grabber, while the input logger writes the event file.
//!
//! | OS      | Video                     | Mic          | System audio                     |
//! |---------|---------------------------|--------------|----------------------------------|
//! | Linux   | x11grab (X11)             | PulseAudio   | Pulse/PipeWire monitor source    |
//! | macOS   | avfoundation              | avfoundation | TODO: ScreenCaptureKit           |
//! | Windows | gdigrab                   | dshow        | TODO: WASAPI loopback            |
//!
//! TODO(wayland): xdg-desktop-portal ScreenCast + PipeWire stream.
//! TODO(macos): native ScreenCaptureKit backend (window picking, system audio).
//! TODO(windows): Windows.Graphics.Capture backend for per-window capture.

use crate::events::EventLog;
use crate::input::InputLogger;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CaptureTarget {
    /// Whole display (index into the OS display list).
    Screen { display: u32 },
    /// One window, by native id (X11 window id, HWND title on Windows).
    Window { id: String },
    /// A rectangle in screen pixels.
    Area { x: i32, y: i32, w: u32, h: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordOptions {
    pub target: CaptureTarget,
    pub fps: u32,
    pub mic: bool,
    pub system_audio: bool,
    /// Output directory. The recorder writes `recording.mkv` and
    /// `recording.events.jsonl` there.
    pub out_dir: PathBuf,
}

/// Geometry of what will be captured, used to make event coordinates relative.
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// What a backend must provide: the ffmpeg arguments for its grabber.
pub(crate) struct CapturePlan {
    pub region: Region,
    pub args: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) fn plan(opts: &RecordOptions) -> Result<CapturePlan> {
    #[cfg(target_os = "linux")]
    return linux::plan(opts);
    #[cfg(target_os = "macos")]
    return macos::plan(opts);
    #[cfg(target_os = "windows")]
    return windows::plan(opts);
    #[allow(unreachable_code)]
    {
        let _ = opts;
        bail!("screen capture is not supported on this platform yet")
    }
}

pub struct RecordingSession {
    child: Child,
    logger: InputLogger,
    started: Instant,
    pub video: PathBuf,
    pub events: PathBuf,
    region: Region,
    pub warnings: Vec<String>,
}

pub struct Recording {
    pub video: PathBuf,
    pub events: PathBuf,
    pub duration: Duration,
}

pub fn start(opts: &RecordOptions) -> Result<RecordingSession> {
    std::fs::create_dir_all(&opts.out_dir)?;
    let plan = plan(opts)?;
    let video = opts.out_dir.join("recording.mkv");
    let events = opts.out_dir.join("recording.events.jsonl");
    let mut cmd = Command::new(crate::ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-y"]).args(&plan.args);
    cmd.args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "16", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
    cmd.arg(&video);
    // The logger starts first so no early input is lost; timestamps are
    // re-based to the moment ffmpeg was launched.
    let logger = InputLogger::start(plan.region)?;
    let child = cmd.stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("failed to start ffmpeg")?;
    let started = Instant::now();
    logger.set_epoch(started);
    Ok(RecordingSession { child, logger, started, video, events, region: plan.region, warnings: plan.warnings })
}

impl RecordingSession {
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn stop(mut self) -> Result<Recording> {
        // "q" makes ffmpeg finish the file cleanly.
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = stdin.write_all(b"q");
        }
        let out = self.child.wait_with_output()?;
        let duration = self.started.elapsed();
        let mut log = EventLog::new(self.region.w, self.region.h);
        log.header.origin_x = self.region.x;
        log.header.origin_y = self.region.y;
        log.events = self.logger.stop();
        log.save(&self.events)?;
        if !self.video.exists() {
            bail!("recording failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(Recording { video: self.video, events: self.events, duration })
    }
}

/// Helper used by backends.
pub(crate) fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

#[allow(dead_code)]
pub(crate) fn which(bin: &str) -> bool {
    Command::new(bin).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

#[allow(dead_code)]
pub(crate) fn exists(p: &Path) -> bool {
    p.exists()
}
