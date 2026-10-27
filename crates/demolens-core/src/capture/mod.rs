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
    let video = opts.out_dir.join("recording.mkv");
    let events = opts.out_dir.join("recording.events.jsonl");
    let spawn = |plan: &CapturePlan| -> Result<Child> {
        let mut cmd = Command::new(crate::ffmpeg::ffmpeg_bin());
        cmd.args(["-v", "error", "-y"]).args(&plan.args);
        cmd.args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "16", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
        cmd.arg(&video);
        cmd.stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("failed to start ffmpeg. Is ffmpeg installed and on PATH?")
    };
    // ffmpeg fails within a moment when a device cannot be opened. A missing
    // or busy microphone should not cost the user the recording, so retry
    // once with video only and say so.
    let early_failure = |child: &mut Child| -> Option<String> {
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(900) {
            if let Ok(Some(_)) = child.try_wait() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut e, &mut err);
                }
                return Some(err);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    };
    let mut plan = plan(opts)?;
    let mut started = Instant::now();
    let mut child = spawn(&plan)?;
    if let Some(err) = early_failure(&mut child) {
        if opts.mic || opts.system_audio {
            let mut o = opts.clone();
            o.mic = false;
            o.system_audio = false;
            let mut retry = self::plan(&o)?;
            retry.warnings.retain(|w| w != "recording without audio");
            retry.warnings.push(format!("Could not open the {}, so this recording has no sound. ({})", if opts.mic { "microphone" } else { "system audio" }, first_line(&err)));
            plan = retry;
            started = Instant::now();
            child = spawn(&plan)?;
            if let Some(err) = early_failure(&mut child) {
                bail!("Could not start screen capture: {}", first_line(&err));
            }
        } else {
            bail!("Could not start screen capture: {}", first_line(&err));
        }
    }
    let logger = InputLogger::start(plan.region)?;
    logger.set_epoch(started);
    Ok(RecordingSession { child, logger, started, video, events, region: plan.region, warnings: plan.warnings })
}

fn first_line(s: &str) -> String {
    let l = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("ffmpeg exited");
    // Drop ffmpeg's "[in#1 @ 0x55...]" prefix.
    let l = if l.starts_with('[') { l.split_once("] ").map(|x| x.1).unwrap_or(l) } else { l };
    l.chars().take(160).collect()
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
