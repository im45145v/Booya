//! Screen capture. Each OS backend builds an ffmpeg capture command using the
//! platform's native grabber, while the input logger writes the event file.
//!
//! | OS      | Video                     | Mic          | System audio                     |
//! |---------|---------------------------|--------------|----------------------------------|
//! | Linux   | x11grab (X11)             | PulseAudio   | Pulse/PipeWire monitor source    |
//! | macOS   | ScreenCaptureKit (frames  | avfoundation | TODO: SCStream audio             |
//! |         | piped to ffmpeg)          | (audio only) |                                  |
//! | Windows | gdigrab                   | dshow        | TODO: WASAPI loopback            |
//!
//! TODO(wayland): xdg-desktop-portal ScreenCast + PipeWire stream.
//! TODO(macos): window picker UI and system audio through SCStream.
//! TODO(windows): Windows.Graphics.Capture backend for per-window capture.

use crate::events::EventLog;
use crate::input::InputLogger;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod mics;
pub mod sources;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_sck;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) mod winapi;

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
    /// Key capture. Off by default (no keyboard hook at all). When on, only
    /// key classes are kept: shortcuts by name, typed characters as `printable`.
    #[serde(default)]
    pub keys: crate::input::KeyLogging,
    /// Also record the webcam to `webcam.mkv` for a picture-in-picture overlay.
    #[serde(default)]
    pub webcam: bool,
    /// Name of the microphone to use (as listed by `list_mics`). None picks
    /// the built-in one, so a Bluetooth headset is not switched to its low
    /// quality call mode by accident.
    #[serde(default)]
    pub mic_device: Option<String>,
    /// Tallest picture to record, in pixels. A 4K screen pushes 33 MB per
    /// frame into the encoder, which is more than a laptop can do at 30 fps,
    /// so frames get dropped and motion stutters. None is automatic (screens
    /// taller than 1600 px are recorded at 1440 px high), Some(0) is native
    /// resolution, Some(n) caps the height at n.
    #[serde(default)]
    pub max_height: Option<u32>,
}

/// Stream pixels per screen pixel for a screen `height` pixels tall (1.0 when
/// no scaling is needed).
pub fn capture_scale(height: u32, max_height: Option<u32>) -> f64 {
    let cap = match max_height {
        Some(0) => return 1.0,
        Some(n) => n.max(240),
        None if height > 1600 => 1440,
        None => return 1.0,
    };
    if height <= cap {
        1.0
    } else {
        cap as f64 / height as f64
    }
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
    /// ffmpeg input arguments.
    pub args: Vec<String>,
    /// ffmpeg output arguments (filters, frame rate) placed before the encoder.
    pub out_args: Vec<String>,
    pub warnings: Vec<String>,
    /// Backends that push frames into ffmpeg's stdin (macOS) provide this.
    pub feed: Option<Feed>,
    /// Input arguments of a separate ffmpeg that records the microphone to
    /// its own file (`recording.mic.mkv`) while the screen is encoded. The two
    /// are joined when the recording stops, so a busy video encoder can never
    /// starve the microphone.
    pub mic_sidecar: Option<Vec<String>>,
}

/// Starts feeding frames into ffmpeg's stdin. The returned guard stops the
/// capture when dropped.
pub(crate) type Feed = Box<dyn FnOnce(std::process::ChildStdin) -> Box<dyn Send> + Send>;

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
    /// Frame source for backends that feed ffmpeg (stopped first on `stop`).
    feed: Option<Box<dyn Send>>,
    webcam: Option<Child>,
    /// Separate microphone recorder (see `CapturePlan::mic_sidecar`).
    mic: Option<Child>,
    logger: InputLogger,
    started: Instant,
    stderr: Arc<Mutex<String>>,
    pub video: PathBuf,
    pub events: PathBuf,
    region: Region,
    pub warnings: Vec<String>,
    dir: PathBuf,
    /// The folder did not exist before this recording (so an empty result may delete it).
    created: bool,
}

pub struct Recording {
    pub video: PathBuf,
    pub events: PathBuf,
    pub duration: Duration,
}

/// How long ffmpeg may run without writing anything before we give up. On
/// macOS this is what happens while the Screen Recording permission is
/// missing or its prompt is still open.
const FIRST_OUTPUT_TIMEOUT: Duration = Duration::from_secs(8);
/// How long `stop` waits for ffmpeg to finish the file after "q".
const STOP_TIMEOUT: Duration = Duration::from_secs(6);

fn permission_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        " On macOS, allow DemoLens in System Settings > Privacy & Security > Screen & System Audio Recording (and Microphone if the mic is on), then quit and reopen DemoLens."
    } else {
        ""
    }
}

/// Reads ffmpeg's stderr on a thread, so a chatty device can never fill the
/// pipe and freeze ffmpeg, and keeps the text for error messages.
fn drain_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let buf = Arc::new(Mutex::new(String::new()));
    if let Some(mut e) = child.stderr.take() {
        let b = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut e, &mut chunk) {
                if n == 0 {
                    break;
                }
                let mut s = b.lock().unwrap();
                if s.len() < 64 * 1024 {
                    s.push_str(&String::from_utf8_lossy(&chunk[..n]));
                }
            }
        });
    }
    buf
}

/// Waits until ffmpeg is writing the file. Err(text) when it exits or
/// produces nothing in time (the child is killed in that case).
fn wait_until_writing(child: &mut Child, video: &Path, err: &Arc<Mutex<String>>, timeout: Duration) -> std::result::Result<(), String> {
    let t = Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            std::thread::sleep(Duration::from_millis(100));
            let e = err.lock().unwrap().clone();
            return Err(if e.trim().is_empty() { "ffmpeg exited".into() } else { e });
        }
        let size = std::fs::metadata(video).map(|m| m.len()).unwrap_or(0);
        if size > 0 && t.elapsed() > Duration::from_millis(600) {
            return Ok(());
        }
        if t.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("TIMEOUT {}", err.lock().unwrap()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn start_error(err: &str) -> anyhow::Error {
    if let Some(rest) = err.strip_prefix("TIMEOUT") {
        let detail = first_line(rest);
        let detail = if rest.trim().is_empty() { String::new() } else { format!(" ({detail})") };
        anyhow::anyhow!("Screen capture did not start within {} seconds{detail}.{}", FIRST_OUTPUT_TIMEOUT.as_secs(), permission_hint())
    } else {
        anyhow::anyhow!("Could not start screen capture: {}.{}", first_line(err), permission_hint())
    }
}

/// One PNG of the whole screen (display 0), for the visual area picker.
/// Returns (png bytes, width, height) in capture pixels, the same space
/// `CaptureTarget::Area` uses.
pub fn screenshot() -> Result<(Vec<u8>, u32, u32)> {
    #[cfg(target_os = "macos")]
    {
        macos::screenshot()
    }
    #[cfg(not(target_os = "macos"))]
    {
        screenshot_ffmpeg()
    }
}

#[cfg(not(target_os = "macos"))]
fn screenshot_ffmpeg() -> Result<(Vec<u8>, u32, u32)> {
    let opts = RecordOptions { target: CaptureTarget::Screen { display: 0 }, fps: 1, mic: false, system_audio: false, out_dir: std::env::temp_dir(), keys: Default::default(), webcam: false, mic_device: None, max_height: None };
    let plan = plan(&opts)?;
    let out = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-v", "error", "-nostdin"])
        .args(&plan.args)
        .args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .output()
        .context("failed to start ffmpeg")?;
    if out.stdout.len() < 100 {
        bail!("could not take a screenshot: {}", first_line(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok((out.stdout, plan.region.w, plan.region.h))
}

/// Removes a recording folder this process created, when starting failed.
/// A folder that already existed is never touched.
struct FreshDir {
    path: PathBuf,
    created: bool,
    keep: bool,
}

impl FreshDir {
    fn create(path: &Path) -> Result<Self> {
        let created = !path.exists();
        std::fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            // Recordings and the event log are private to the user.
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self { path: path.to_path_buf(), created, keep: false })
    }
}

impl Drop for FreshDir {
    fn drop(&mut self) {
        if self.created && !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Placeholder in a plan's input arguments for the start of the recording
/// clock (as a negative `-itsoffset`), filled in when ffmpeg is launched.
pub(crate) const CLOCK_START: &str = "@clock-start@";

/// Moves everything logged on the recording clock earlier by `shift_ms`, for
/// a video whose first frame was stamped that long after the clock started.
/// Times never go below zero.
pub(crate) fn shift_timeline(events: &mut [crate::events::Event], pauses: &mut [(f64, f64)], shift_ms: f64) {
    for e in events.iter_mut() {
        e.t = (e.t - shift_ms).max(0.0);
    }
    for p in pauses.iter_mut() {
        p.0 = (p.0 - shift_ms).max(0.0);
        p.1 = (p.1 - shift_ms).max(0.0);
    }
}

pub fn start(opts: &RecordOptions) -> Result<RecordingSession> {
    // Work out the capture plan first (it needs the permissions and can
    // fail), and only then create the folder, so a failed start leaves nothing
    // behind.
    let mut plan = plan(opts)?;
    let mut fresh = FreshDir::create(&opts.out_dir)?;
    let video = opts.out_dir.join("recording.mkv");
    let events = opts.out_dir.join("recording.events.jsonl");
    let mic_file = opts.out_dir.join("recording.mic.mkv");
    let spawn = |plan: &mut CapturePlan| -> Result<(Child, Arc<Mutex<String>>, Option<Box<dyn Send>>, f64)> {
        let _ = std::fs::remove_file(&video);
        let _ = std::fs::remove_file(&mic_file);
        let mut cmd = crate::proc::command(crate::ffmpeg::ffmpeg_bin());
        // The recording clock starts now, as ffmpeg is launched, not while the
        // plan was being made (opening the screen and listing microphones can
        // take a second). The event log starts at the same moment.
        let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        let args: Vec<String> = plan.args.iter().map(|a| if a == CLOCK_START { format!("-{epoch:.3}") } else { a.clone() }).collect();
        cmd.args(["-hide_banner", "-nostats", "-v", "error", "-y"]).args(&args);
        cmd.args(&plan.out_args);
        cmd.args(crate::ffmpeg::best_encoder().capture_args()).args(["-pix_fmt", "yuv420p", "-c:a", "aac"]);
        // Matroska is written as it goes, so even a killed recording keeps
        // what it has; flush packets so the file grows from the first second.
        cmd.args(["-flush_packets", "1"]);
        cmd.arg(&video);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // gdigrab reads the screen in the pixels the process is told
            // about. Without this a 125% or 150% display is captured at its
            // scaled size and the pointer lands in the wrong place.
            cmd.env("__COMPAT_LAYER", "HIGHDPIAWARE");
        }
        let mut child = cmd.spawn().context("failed to start ffmpeg. Is ffmpeg installed?")?;
        let err = drain_stderr(&mut child);
        // Only a frame-feeding backend gives up stdin; the others keep it for "q".
        let feed = match plan.feed.take() {
            Some(f) => child.stdin.take().map(f),
            None => None,
        };
        Ok((child, err, feed, epoch))
    };
    let mut started = Instant::now();
    let (mut child, mut err, mut feed, mut epoch) = spawn(&mut plan)?;
    if let Err(e) = wait_until_writing(&mut child, &video, &err, FIRST_OUTPUT_TIMEOUT) {
        // A missing or blocked microphone should not cost the user the
        // recording, so retry once with video only and say so.
        if opts.mic || opts.system_audio {
            let mut o = opts.clone();
            o.mic = false;
            o.system_audio = false;
            drop(feed.take()); // stop the first capture before starting another
            let mut retry = self::plan(&o)?;
            retry.warnings.retain(|w| w != "recording without audio");
            let why = if e.starts_with("TIMEOUT") { "it did not respond".to_string() } else { first_line(&e) };
            retry.warnings.push(format!("Could not open the {}, so this recording has no sound ({why}).", if opts.mic { "microphone" } else { "system audio" }));
            plan = retry;
            started = Instant::now();
            (child, err, feed, epoch) = spawn(&mut plan)?;
            if let Err(e2) = wait_until_writing(&mut child, &video, &err, FIRST_OUTPUT_TIMEOUT) {
                return Err(start_error(&e2));
            }
        } else {
            return Err(start_error(&e));
        }
    }
    let mut warnings = plan.warnings;
    // The microphone is recorded by its own ffmpeg, stamped with the same
    // clock as the screen, and joined to it when the recording stops.
    let mic = match plan.mic_sidecar.take() {
        Some(input) => start_mic_sidecar(&input, epoch, &mic_file, &mut warnings),
        None => None,
    };
    if opts.keys == crate::input::KeyLogging::On && crate::input::key_permission_missing(true) {
        warnings.push("Key capture is on, but macOS has not allowed DemoLens to monitor the keyboard, so shortcuts such as Cmd+Tab will not be shown. Open System Settings > Privacy & Security > Input Monitoring, switch DemoLens on, then quit and reopen DemoLens.".into());
    }
    let logger = InputLogger::start(plan.region, opts.keys)?;
    logger.set_epoch(started);
    let webcam = if opts.webcam { start_webcam(&opts.out_dir, &mut warnings) } else { None };
    if webcam.is_some() {
        // When the camera's first frame landed, relative to the screen's.
        let _ = std::fs::write(opts.out_dir.join("webcam.offset"), format!("{:.0}", started.elapsed().as_secs_f64() * 1000.0));
    }
    fresh.keep = true;
    Ok(RecordingSession { child, feed, webcam, mic, logger, started, stderr: err, video, events, region: plan.region, warnings, dir: opts.out_dir.clone(), created: fresh.created })
}

/// Records the microphone on its own, as uncompressed samples, so nothing
/// that happens to the video encoder (a 4K frame backlog, a slow disk) can
/// make it drop or repeat audio. Every packet is stamped with the wall clock
/// since `epoch` (the start of the shared recording clock) so the start time
/// of the first samples is known; no timestamp based resampling is applied,
/// so every sample the device delivered ends up in the file.
fn start_mic_sidecar(input: &[String], epoch: f64, out: &Path, warnings: &mut Vec<String>) -> Option<Child> {
    let mut cmd = crate::proc::command(crate::ffmpeg::ffmpeg_bin());
    cmd.args(["-hide_banner", "-nostats", "-v", "error", "-y"]);
    cmd.args(["-copyts", "-thread_queue_size", "2048", "-use_wallclock_as_timestamps", "1", "-itsoffset", &format!("-{epoch:.3}")]);
    cmd.args(input);
    cmd.args(["-vn", "-c:a", "pcm_f32le", "-flush_packets", "1"]).arg(out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut c = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            warnings.push("Could not start the microphone recorder, so this recording has no sound.".into());
            return None;
        }
    };
    let err = drain_stderr(&mut c);
    match wait_until_writing(&mut c, out, &err, FIRST_OUTPUT_TIMEOUT) {
        Ok(()) => Some(c),
        Err(e) => {
            let why = if e.starts_with("TIMEOUT") { "it did not respond".to_string() } else { first_line(&e) };
            warnings.push(format!("Could not open the microphone, so this recording has no sound ({why})."));
            None
        }
    }
}

/// Webcam input arguments for this OS, or None when no camera can be found.
fn webcam_input() -> Option<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        let dev = (0..4).map(|i| format!("/dev/video{i}")).find(|p| Path::new(p).exists())?;
        Some(vec![s("-f"), s("v4l2"), s("-framerate"), s("30"), s("-video_size"), s("640x480"), s("-i"), dev])
    }
    #[cfg(target_os = "macos")]
    {
        let idx = macos::camera_index()?;
        Some(vec![s("-f"), s("avfoundation"), s("-framerate"), s("30"), s("-video_size"), s("1280x720"), s("-i"), format!("{idx}:none")])
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn start_webcam(dir: &Path, warnings: &mut Vec<String>) -> Option<Child> {
    let Some(input) = webcam_input() else {
        warnings.push("No webcam found (webcam capture is supported on Linux and macOS), so there is no camera overlay.".into());
        return None;
    };
    let out = dir.join("webcam.mkv");
    let mut cmd = crate::proc::command(crate::ffmpeg::ffmpeg_bin());
    cmd.args(["-hide_banner", "-nostats", "-v", "error", "-y"]).args(&input);
    cmd.args(crate::ffmpeg::best_encoder().capture_args()).args(["-pix_fmt", "yuv420p", "-an", "-flush_packets", "1"]).arg(&out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    match cmd.spawn() {
        Ok(mut c) => {
            let err = Arc::new(Mutex::new(String::new()));
            if wait_until_writing(&mut c, &out, &err, FIRST_OUTPUT_TIMEOUT).is_ok() {
                Some(c)
            } else {
                warnings.push("The webcam did not start (check the Camera permission), so there is no camera overlay.".into());
                None
            }
        }
        Err(_) => None,
    }
}

/// Ask ffmpeg to finish with "q", then kill it after `STOP_TIMEOUT`.
/// Returns true when it had to be killed.
fn finish(child: &mut Child, fed: bool) -> bool {
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q\n");
        let _ = stdin.flush();
    }
    let t = Instant::now();
    if fed {
        // ffmpeg's stdin carries the video, so there is no "q". The feeder
        // closing it ends the video; with a live microphone input ffmpeg
        // would keep waiting, so interrupt it (it finishes the file on SIGINT).
        while t.elapsed() < Duration::from_millis(400) {
            if let Ok(Some(_)) = child.try_wait() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        #[cfg(unix)]
        {
            let _ = crate::proc::command("kill").args(["-INT", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return false,
            Ok(None) if t.elapsed() < STOP_TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

impl Drop for RecordingSession {
    /// A session dropped without `stop` (app quit, panic) must not leave
    /// ffmpeg recording the screen in the background.
    fn drop(&mut self) {
        for c in std::iter::once(&mut self.child).chain(self.webcam.as_mut()).chain(self.mic.as_mut()) {
            if let Ok(None) = c.try_wait() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

fn first_line(s: &str) -> String {
    let l = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("ffmpeg exited");
    // Drop ffmpeg's "[in#1 @ 0x55...]" prefix.
    let l = if l.starts_with('[') { l.split_once("] ").map(|x| x.1).unwrap_or(l) } else { l };
    l.chars().take(160).collect()
}

impl RecordingSession {
    /// Privacy pause: stops cursor and key logging (and kills the keyboard
    /// hook). The screen keeps recording, but the paused stretch is cut out
    /// of the project, so it never reaches an export.
    pub fn set_paused(&mut self, paused: bool) {
        self.logger.set_paused(paused);
    }

    pub fn is_paused(&self) -> bool {
        self.logger.is_paused()
    }

    /// Drop a marker at the cursor. False while paused.
    pub fn mark(&self) -> bool {
        self.logger.mark()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Time spent paused so far.
    pub fn paused(&self) -> Duration {
        self.logger.paused_total()
    }

    /// Stop and throw the recording away: the folder is deleted if this
    /// recording created it (never a folder that already existed).
    pub fn discard(mut self) -> Result<()> {
        let fed = self.feed.take().is_some();
        finish(&mut self.child, fed);
        if let Some(c) = self.webcam.as_mut() {
            finish(c, false);
        }
        if let Some(c) = self.mic.as_mut() {
            finish(c, false);
        }
        let _ = std::mem::replace(&mut self.logger, InputLogger::idle()).stop();
        if self.created {
            std::fs::remove_dir_all(&self.dir)?;
        } else {
            for f in [&self.video, &self.events, &self.video.with_extension("mic.mkv")] {
                let _ = std::fs::remove_file(f);
            }
        }
        Ok(())
    }

    pub fn stop(mut self) -> Result<Recording> {
        // "q" makes ffmpeg finish the file cleanly. If it does not exit in
        // time (a stuck device), kill it: Matroska keeps what was written.
        let fed = self.feed.take().is_some(); // stops the capture, closes ffmpeg's stdin
        // The microphone stops first, so the screen's flush does not leave it
        // running past the end of the video by much.
        if let Some(c) = self.mic.as_mut() {
            finish(c, false);
        }
        let killed = finish(&mut self.child, fed);
        if let Some(c) = self.webcam.as_mut() {
            finish(c, false);
        }
        let duration = self.started.elapsed();
        // Stop logging input now, before the remux takes its time.
        let logger = std::mem::replace(&mut self.logger, InputLogger::idle());
        let logged = logger.stop();
        let size = std::fs::metadata(&self.video).map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            let e = self.stderr.lock().unwrap().clone();
            if self.created {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
            bail!("The recording is empty: {}{}", first_line(&e), permission_hint());
        }
        if killed {
            eprintln!("demolens: ffmpeg did not stop on q, it was killed");
        }
        let remuxed = crate::finalize::remux_to_mp4_info(&self.video);
        let (video, video_start_s) = match remuxed {
            Ok(r) => (r.path, r.video_start_s),
            Err(e) => {
                eprintln!("demolens: could not remux to MP4, keeping the MKV: {e:#}");
                (self.video.clone(), 0.0)
            }
        };
        // Every frame is stamped with the wall clock since ffmpeg
        // started (when the microphone is recorded on the side), and the MP4 starts its video at 0. Input events were logged
        // on that same clock, so they move earlier by the video's lead-in.
        let shift_ms = if video_start_s > 0.15 && video_start_s < 10.0 { video_start_s * 1000.0 } else { 0.0 };
        let mut log = EventLog::new(self.region.w, self.region.h);
        log.header.origin_x = self.region.x;
        log.header.origin_y = self.region.y;
        let mut pauses = logged.pauses;
        log.events = logged.events;
        if shift_ms > 0.0 {
            shift_timeline(&mut log.events, &mut pauses, shift_ms);
            // The webcam offset was measured on the same clock.
            let wc = self.dir.join("webcam.offset");
            if let Some(ms) = std::fs::read_to_string(&wc).ok().and_then(|t| t.trim().parse::<f64>().ok()) {
                let _ = std::fs::write(&wc, format!("{:.0}", ms - shift_ms));
            }
        }
        // A screen recorded below its own resolution (see `max_height`): the
        // pointer was logged in screen pixels, the video is smaller.
        if let Ok(info) = crate::ffmpeg::probe(&video) {
            if self.region.w > 0 && info.width > 0 {
                let k = info.width as f64 / self.region.w as f64;
                if k < 0.98 && k > 0.1 {
                    crate::events::scale_coords(&mut log.events, k);
                    log.header.width = info.width;
                    log.header.height = info.height;
                }
            }
        }
        log.save(&self.events)?;
        // Paused stretches are cut out when the project is created.
        if !pauses.is_empty() {
            let _ = std::fs::write(self.video.with_extension("pauses.json"), serde_json::to_string(&pauses)?);
        }

        Ok(Recording { video, events: self.events.clone(), duration })
    }
}

/// One avfoundation device from `ffmpeg -f avfoundation -list_devices true -i ""`.
#[derive(Debug, Clone, PartialEq)]
pub struct AvDevice {
    pub index: u32,
    pub name: String,
}

/// Parses the device list ffmpeg prints on stderr into (video, audio) devices.
pub fn parse_avfoundation_devices(text: &str) -> (Vec<AvDevice>, Vec<AvDevice>) {
    let (mut video, mut audio) = (Vec::new(), Vec::new());
    let mut in_audio = false;
    for line in text.lines() {
        if line.contains("AVFoundation video devices") {
            in_audio = false;
            continue;
        }
        if line.contains("AVFoundation audio devices") {
            in_audio = true;
            continue;
        }
        // "[AVFoundation indev @ 0x7f...] [3] Capture screen 0"
        let Some(rest) = line.split_once("] [").map(|x| x.1) else { continue };
        let Some((idx, name)) = rest.split_once("] ") else { continue };
        let Ok(index) = idx.trim().parse() else { continue };
        let d = AvDevice { index, name: name.trim().to_string() };
        if in_audio { audio.push(d) } else { video.push(d) }
    }
    (video, audio)
}

/// Index of "Capture screen N" for display N, if listed.
pub fn screen_index(video: &[AvDevice], display: u32) -> Option<u32> {
    let want = format!("Capture screen {display}");
    video.iter().find(|d| d.name == want).map(|d| d.index)
}

#[cfg(test)]
mod tests {
    #[test]
    fn events_follow_a_video_that_was_recorded_smaller() {
        use crate::events::{scale_coords, Event, EventKind};
        let mut ev = vec![Event { t: 1.0, kind: EventKind::Move { x: 3000.0, y: 1500.0 } }, Event { t: 2.0, kind: EventKind::Key { key: "S".into(), modifiers: vec![] } }];
        scale_coords(&mut ev, 0.5);
        assert_eq!(ev[0].kind, EventKind::Move { x: 1500.0, y: 750.0 });
    }

    #[test]
    fn tall_screens_are_scaled_down_while_capturing() {
        assert_eq!(capture_scale(2160, None), 1440.0 / 2160.0);
        assert_eq!(capture_scale(1912, None), 1440.0 / 1912.0);
        assert_eq!(capture_scale(1080, None), 1.0);
        assert_eq!(capture_scale(1440, None), 1.0);
        assert_eq!(capture_scale(2160, Some(0)), 1.0);
        assert_eq!(capture_scale(2160, Some(1080)), 0.5);
        assert_eq!(capture_scale(900, Some(1080)), 1.0);
    }

    #[test]
    fn the_timeline_shift_moves_events_and_pauses_and_stops_at_zero() {
        use crate::events::{Event, EventKind};
        let mv = |t| Event { t, kind: EventKind::Move { x: 1.0, y: 2.0 } };
        let mut ev = vec![mv(100.0), mv(1500.0)];
        let mut pauses = vec![(900.0, 2000.0)];
        shift_timeline(&mut ev, &mut pauses, 800.0);
        assert_eq!((ev[0].t, ev[1].t), (0.0, 700.0));
        assert_eq!(pauses[0], (100.0, 1200.0));
    }

    use super::*;

    const LIST: &str = "[AVFoundation indev @ 0x7fa] AVFoundation video devices:
[AVFoundation indev @ 0x7fa] [0] FaceTime HD Camera
[AVFoundation indev @ 0x7fa] [1] Ashish's iPhone Camera
[AVFoundation indev @ 0x7fa] [2] Capture screen 0
[AVFoundation indev @ 0x7fa] [3] Capture screen 1
[AVFoundation indev @ 0x7fa] AVFoundation audio devices:
[AVFoundation indev @ 0x7fa] [0] MacBook Pro Microphone
[AVFoundation indev @ 0x7fa] [1] Microsoft Teams Audio
[in#0 @ 0x7fb] Error opening input: Input/output error
";

    #[test]
    fn parses_devices_and_finds_screen() {
        let (v, a) = parse_avfoundation_devices(LIST);
        assert_eq!(v.len(), 4);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], AvDevice { index: 0, name: "MacBook Pro Microphone".into() });
        assert_eq!(screen_index(&v, 0), Some(2));
        assert_eq!(screen_index(&v, 1), Some(3));
        assert_eq!(screen_index(&v, 2), None);
    }

    #[test]
    fn first_line_strips_prefix() {
        assert_eq!(first_line("[in#1 @ 0x55] Device busy\nmore"), "Device busy");
    }

    #[cfg(unix)]
    #[test]
    fn silent_ffmpeg_times_out_instead_of_hanging() {
        let dir = std::env::temp_dir().join(format!("dl-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("v.mkv");
        // stands in for ffmpeg stuck on a permission prompt: alive, no output
        let mut child = crate::proc::command("sleep").arg("30").stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let err = drain_stderr(&mut child);
        let t = Instant::now();
        let r = wait_until_writing(&mut child, &video, &err, Duration::from_millis(700));
        assert!(r.unwrap_err().starts_with("TIMEOUT"));
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(start_error("TIMEOUT").to_string().contains("did not start"));
    }

    #[cfg(unix)]
    #[test]
    fn early_exit_is_reported() {
        let video = std::env::temp_dir().join("dl-none.mkv");
        let mut child = crate::proc::command("sh").args(["-c", "echo 'Input/output error' >&2; exit 1"]).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let err = drain_stderr(&mut child);
        let r = wait_until_writing(&mut child, &video, &err, Duration::from_secs(5));
        assert!(r.unwrap_err().contains("Input/output error"));
    }
}

/// Helper used by backends.
pub(crate) fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

#[allow(dead_code)]
pub(crate) fn which(bin: &str) -> bool {
    crate::proc::command(bin).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

#[allow(dead_code)]
pub(crate) fn exists(p: &Path) -> bool {
    p.exists()
}

fn frame_count(path: &Path) -> Option<u64> {
    let out = crate::proc::command(crate::ffmpeg::ffprobe_bin())
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
        .arg(crate::ffmpeg::file_url(path))
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Self test for `demolens doctor`: permissions, displays, ffmpeg and a short
/// test capture, as JSON. Never panics and never needs the desktop app.
pub fn doctor(test_seconds: f64) -> serde_json::Value {
    use serde_json::{json, Value};
    let mut v = json!({
        "demolens": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "capture_backend": if cfg!(target_os = "macos") { "ScreenCaptureKit" } else if cfg!(target_os = "windows") { "gdigrab" } else { "x11grab" },
    });
    let mut next: Vec<String> = Vec::new();

    let ffmpeg = crate::ffmpeg::ffmpeg_bin();
    let version = crate::proc::command(&ffmpeg).arg("-version").output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string));
    v["ffmpeg"] = json!({ "path": ffmpeg, "version": version, "encoder": crate::ffmpeg::best_encoder().name() });
    if version.is_none() {
        next.push(format!("ffmpeg was not found or does not run. {}", crate::ffmpeg::install_hint()));
    }

    #[cfg(target_os = "macos")]
    {
        v["screen_recording_permission"] = json!(macos_sck::has_permission());
        let keys_ok = !crate::input::key_permission_missing(false);
        v["input_monitoring_permission"] = json!(keys_ok);
        if !keys_ok {
            next.push("Input Monitoring is not allowed, so shortcuts and typing cannot be shown in videos. Run `demolens doctor --prompt-keys` or turn on Keys in the app to get the macOS prompt, then enable DemoLens in System Settings > Privacy & Security > Input Monitoring.".into());
        }
        match macos_sck::content_info() {
            Ok(c) => v["displays"] = serde_json::to_value(&c).unwrap_or(Value::Null),
            Err(e) => {
                v["displays_error"] = json!(e);
                next.push(e);
            }
        }
        let (_, audio) = macos::list_devices();
        v["microphones"] = json!(audio.iter().map(|d| d.name.clone()).collect::<Vec<_>>());
    }

    let dir = std::env::temp_dir().join(format!("demolens-doctor-{}", std::process::id()));
    let opts = RecordOptions { target: CaptureTarget::Screen { display: 0 }, fps: 30, mic: false, system_audio: false, out_dir: dir.clone(), keys: Default::default(), webcam: false, mic_device: None, max_height: None };
    let t0 = Instant::now();
    let mut test = serde_json::Map::new();
    match start(&opts) {
        Err(e) => {
            test.insert("ok".into(), json!(false));
            test.insert("error".into(), json!(format!("{e:#}")));
            next.push(format!("{e:#}"));
        }
        Ok(session) => {
            test.insert("start_ms".into(), json!(t0.elapsed().as_millis() as u64));
            std::thread::sleep(Duration::from_secs_f64(test_seconds.clamp(1.0, 30.0)));
            match session.stop() {
                Err(e) => {
                    test.insert("ok".into(), json!(false));
                    test.insert("error".into(), json!(format!("{e:#}")));
                    next.push(format!("{e:#}"));
                }
                Ok(rec) => {
                    let size = std::fs::metadata(&rec.video).map(|m| m.len()).unwrap_or(0);
                    let info = crate::ffmpeg::probe(&rec.video).ok();
                    let frames = frame_count(&rec.video);
                    let ok = size > 0 && frames.unwrap_or(0) > 0;
                    test.insert("ok".into(), json!(ok));
                    test.insert("seconds".into(), json!((rec.duration.as_secs_f64() * 10.0).round() / 10.0));
                    test.insert("file_bytes".into(), json!(size));
                    test.insert("frames".into(), json!(frames));
                    if let Some(i) = info {
                        test.insert("width".into(), json!(i.width));
                        test.insert("height".into(), json!(i.height));
                        test.insert("fps".into(), json!((i.fps * 10.0).round() / 10.0));
                    }
                    if !ok {
                        next.push("The test recording is empty.".into());
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let ok = test.get("ok").and_then(Value::as_bool).unwrap_or(false);
    v["test_capture"] = Value::Object(test);
    v["ok"] = json!(ok);
    next.dedup();
    v["next_steps"] = json!(next);
    v
}
