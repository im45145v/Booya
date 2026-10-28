//! One microphone recording, whatever is doing the capturing, with the checks
//! that keep a silent or failed microphone from going unnoticed:
//!
//! * Windows records in process through WASAPI and falls back to DirectShow
//!   (an ffmpeg sidecar); macOS and Linux use an ffmpeg sidecar.
//! * `state()` is asked 1 to 2 seconds into a recording and says whether
//!   sound is arriving, so the editor can warn at once.
//! * Everything that happens is written to `audio.log` in the project folder.

use super::{drain_stderr, finish, first_line, wait_until_writing, FIRST_OUTPUT_TIMEOUT};
use crate::capture::audiosink::SILENT_DB;
use anyhow::Result;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum MicState {
    /// The microphone is off for this recording.
    Off,
    /// Opened, nothing measured yet.
    Starting,
    Ok { peak_db: f32 },
    /// Opened, but only digital silence arrives.
    Silent { peak_db: f32 },
    Failed { reason: String },
}

impl MicState {
    /// The sentence for the editor, when something is wrong.
    pub fn message(&self) -> Option<String> {
        match self {
            MicState::Silent { .. } => Some(format!("Mic not recorded: no sound is coming from the microphone. Check that it is not muted. {}", privacy_hint())),
            MicState::Failed { reason } => Some(format!("Mic not recorded: {reason}")),
            _ => None,
        }
    }
}

pub(crate) fn privacy_hint() -> &'static str {
    if cfg!(windows) {
        "On Windows also check Settings > Privacy & security > Microphone, and that \"Let desktop apps access your microphone\" is on."
    } else if cfg!(target_os = "macos") {
        "Also check System Settings > Privacy & Security > Microphone for DemoLens."
    } else {
        ""
    }
}

/// A short plain reason from what ffmpeg or the OS said while opening a
/// microphone.
pub fn explain_error(text: &str) -> String {
    let t = text.to_lowercase();
    if t.contains("access is denied") || t.contains("0x80070005") || t.contains("permission denied") || t.contains("not permitted") || t.contains("privacy") || t.contains("not authorized") {
        format!("Windows or macOS is blocking this app from using the microphone. {}", privacy_hint())
    } else if t.contains("could not find audio") || t.contains("no such device") || t.contains("no microphone") || t.contains("not found") {
        "the selected microphone was not found. It may be unplugged or disabled.".into()
    } else if t.contains("could not run graph") || t.contains("device or resource busy") || t.contains("already in use") || t.contains("exclusive") || t.contains("in use") {
        "another program is using the microphone exclusively. Close apps that hold it, then record again.".into()
    } else if t.contains("did not respond") || t.starts_with("timeout") {
        "the microphone did not respond.".into()
    } else {
        first_line(text)
    }
}

/// The device to open from a list of names (the default first when known).
/// An exact match (ignoring case) wins, then a partial one; with no choice
/// the default, then the first.
pub fn pick_device_name(names: &[String], wanted: Option<&str>, default: Option<&str>) -> Option<String> {
    if let Some(w) = wanted.map(str::trim).filter(|w| !w.is_empty()) {
        let wl = w.to_lowercase();
        if let Some(n) = names.iter().find(|n| n.to_lowercase() == wl) {
            return Some(n.clone());
        }
        if let Some(n) = names.iter().find(|n| n.to_lowercase().contains(&wl) || wl.contains(&n.to_lowercase())) {
            return Some(n.clone());
        }
    }
    default.map(str::to_string).filter(|d| names.iter().any(|n| n == d) || names.is_empty()).or_else(|| names.first().cloned())
}

/// Appends a line to the project's `audio.log`.
pub fn log_line(log: &Path, text: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
        let _ = writeln!(f, "[{}] {}", chrono_like_now(), text);
    }
}

fn chrono_like_now() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    format!("{s:.3}")
}

/// Loudest sample in an audio file (from ffmpeg's volumedetect), looking at
/// the first `secs` seconds. None when nothing could be read.
pub fn file_peak_db(path: &Path, secs: f64) -> Option<f32> {
    let out = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-t", &format!("{secs}"), "-i"])
        .arg(path)
        .args(["-vn", "-af", "volumedetect", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    parse_max_volume(&String::from_utf8_lossy(&out.stderr))
}

pub(crate) fn parse_max_volume(text: &str) -> Option<f32> {
    let l = text.lines().find(|l| l.contains("max_volume:"))?;
    let v = l.split("max_volume:").nth(1)?.trim().trim_end_matches("dB").trim();
    v.parse::<f32>().ok()
}

enum Backend {
    Ffmpeg { child: Child, err: Arc<Mutex<String>> },
    #[cfg(windows)]
    Wasapi(super::wasapi::Recorder),
}

pub(crate) struct MicRec {
    backend: Option<Backend>,
    out: PathBuf,
    log: PathBuf,
    started: Instant,
    pub name: &'static str,
    last: MicState,
}

impl MicRec {
    /// Starts recording to `out` (Matroska, float samples). `epoch` is the
    /// unix time of the shared recording clock. Problems are logged and
    /// returned as the reason in `Err`.
    pub fn start(device: Option<&str>, epoch: f64, out: &Path, log: &Path) -> Result<MicRec, String> {
        let _ = std::fs::remove_file(out);
        let mut reasons: Vec<String> = Vec::new();
        #[cfg(windows)]
        {
            let wav = out.with_extension("wav");
            match super::wasapi::start(super::wasapi::Source::Mic(device.map(str::to_string)), epoch, &wav) {
                Ok(r) => {
                    log_line(log, &format!("microphone: WASAPI, device \"{}\"", r.device));
                    return Ok(MicRec { backend: Some(Backend::Wasapi(r)), out: out.to_path_buf(), log: log.to_path_buf(), started: Instant::now(), name: "WASAPI", last: MicState::Starting });
                }
                Err(e) => {
                    log_line(log, &format!("WASAPI could not open the microphone: {e:#}; trying DirectShow"));
                    reasons.push(explain_error(&format!("{e:#}")));
                }
            }
        }
        match start_ffmpeg(device, epoch, out, log) {
            Ok(r) => Ok(r),
            Err(e) => {
                reasons.push(e);
                reasons.dedup();
                Err(reasons.join(" Then: "))
            }
        }
    }

    pub fn backend_name(&self) -> &'static str {
        self.name
    }

    /// What the microphone is doing now. Cheap enough to call every second.
    pub fn state(&mut self) -> MicState {
        let s = self.state_now();
        if s != self.last {
            log_line(&self.log, &format!("mic state: {s:?}"));
            self.last = s.clone();
        }
        s
    }

    fn state_now(&mut self) -> MicState {
        match self.backend.as_mut() {
            None => MicState::Off,
            Some(Backend::Ffmpeg { child, err }) => {
                if let Ok(Some(st)) = child.try_wait() {
                    return MicState::Failed { reason: format!("the microphone recorder stopped ({st}): {}", explain_error(&err.lock().unwrap())) };
                }
                let size = std::fs::metadata(&self.out).map(|m| m.len()).unwrap_or(0);
                if size < 20_000 || self.started.elapsed() < Duration::from_millis(1200) {
                    return if self.started.elapsed() > Duration::from_secs(4) { MicState::Failed { reason: "no audio arrived from the microphone".into() } } else { MicState::Starting };
                }
                match file_peak_db(&self.out, 3.0) {
                    Some(db) if db < SILENT_DB => MicState::Silent { peak_db: db },
                    Some(db) => MicState::Ok { peak_db: db },
                    None => MicState::Starting,
                }
            }
            #[cfg(windows)]
            Some(Backend::Wasapi(r)) => {
                if let Some(e) = r.error() {
                    return MicState::Failed { reason: explain_error(&e) };
                }
                if r.probe.frames() == 0 {
                    return if r.age() > Duration::from_secs(3) { MicState::Failed { reason: "Windows opened the microphone but delivered no audio. It may be blocked by the privacy setting. ".to_string() + privacy_hint() } } else { MicState::Starting };
                }
                if r.age() < Duration::from_millis(1200) {
                    return MicState::Starting;
                }
                let db = r.probe.peak_db();
                if db < SILENT_DB { MicState::Silent { peak_db: db } } else { MicState::Ok { peak_db: db } }
            }
        }
    }

    /// Stops and finishes `out`. The returned state is the verdict for the
    /// whole recording (a silent or failed microphone is reported).
    pub fn stop(mut self) -> MicState {
        let mid = self.state_now();
        let verdict = match self.backend.take() {
            None => MicState::Off,
            Some(Backend::Ffmpeg { mut child, err }) => {
                let died = matches!(child.try_wait(), Ok(Some(_)));
                finish(&mut child, false);
                let e = err.lock().unwrap().clone();
                if !e.trim().is_empty() {
                    log_line(&self.log, &format!("ffmpeg microphone stderr: {}", e.trim()));
                }
                if died {
                    mid
                } else {
                    match file_peak_db(&self.out, 3600.0) {
                        Some(db) if db < SILENT_DB => MicState::Silent { peak_db: db },
                        Some(db) => MicState::Ok { peak_db: db },
                        None => MicState::Failed { reason: "the microphone file could not be read".into() },
                    }
                }
            }
            #[cfg(windows)]
            Some(Backend::Wasapi(r)) => {
                let db = r.probe.peak_db();
                let frames = r.probe.frames();
                let device = r.device.clone();
                match r.finish(&self.out) {
                    Ok(()) => {
                        log_line(&self.log, &format!("WASAPI finished: {frames} frames from \"{device}\", peak {db:.1} dBFS"));
                        if db < SILENT_DB { MicState::Silent { peak_db: db } } else { MicState::Ok { peak_db: db } }
                    }
                    Err(e) => MicState::Failed { reason: explain_error(&format!("{e:#}")) },
                }
            }
        };
        log_line(&self.log, &format!("mic result: {verdict:?}"));
        verdict
    }
}

fn start_ffmpeg(device: Option<&str>, epoch: f64, out: &Path, log: &Path) -> Result<MicRec, String> {
    let input = super::mics::input_args(device).map_err(|e| e.to_string())?;
    log_line(log, &format!("microphone: ffmpeg input {}", input.join(" ")));
    let mut cmd = crate::proc::command(crate::ffmpeg::ffmpeg_bin());
    cmd.args(["-hide_banner", "-nostats", "-v", "error", "-y"]);
    cmd.args(["-copyts", "-thread_queue_size", "2048", "-use_wallclock_as_timestamps", "1", "-itsoffset", &format!("-{epoch:.3}")]);
    cmd.args(&input);
    cmd.args(["-vn", "-c:a", "pcm_f32le", "-flush_packets", "1"]).arg(out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut c = cmd.spawn().map_err(|e| format!("could not start ffmpeg for the microphone: {e}"))?;
    let err = drain_stderr(&mut c);
    match wait_until_writing(&mut c, out, &err, FIRST_OUTPUT_TIMEOUT) {
        Ok(()) => Ok(MicRec { backend: Some(Backend::Ffmpeg { child: c, err }), out: out.to_path_buf(), log: log.to_path_buf(), started: Instant::now(), name: if cfg!(windows) { "DirectShow" } else { "ffmpeg" }, last: MicState::Starting }),
        Err(e) => {
            let text = if e.starts_with("TIMEOUT") { "the microphone did not respond".to_string() } else { e.clone() };
            log_line(log, &format!("ffmpeg could not open the microphone: {}", err.lock().unwrap().trim()));
            let _ = c.kill();
            let _ = c.wait();
            Err(explain_error(&text))
        }
    }
}

/// Result of `demolens doctor --audio`.
#[derive(Debug, Clone, Serialize)]
pub struct AudioCheck {
    pub backend: String,
    pub devices: Vec<String>,
    pub state: MicState,
    pub peak_db: Option<f32>,
    pub log: PathBuf,
}

/// Records the microphone for `secs` seconds into a scratch folder and says
/// what arrived. This is the same code path as a real recording.
pub fn check_audio(device: Option<&str>, secs: f64) -> AudioCheck {
    let dir = std::env::temp_dir().join(format!("demolens-audio-check-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("audio.log");
    let out = dir.join("check.mic.mkv");
    let devices = super::mics::list_mics();
    log_line(&log, &format!("devices: {devices:?}"));
    let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    match MicRec::start(device, epoch, &out, &log) {
        Err(reason) => AudioCheck { backend: "none".into(), devices, state: MicState::Failed { reason }, peak_db: None, log },
        Ok(rec) => {
            let backend = rec.backend_name().to_string();
            std::thread::sleep(Duration::from_secs_f64(secs));
            let state = rec.stop();
            let peak_db = match &state {
                MicState::Ok { peak_db } | MicState::Silent { peak_db } => Some(*peak_db),
                _ => None,
            };
            AudioCheck { backend, devices, state, peak_db, log }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_explained_in_plain_words() {
        assert!(explain_error("[dshow @ 0x1] Could not run graph (sometimes caused by a device already in use)").contains("another program"));
        assert!(explain_error("Could not find audio only device with name [Mic] among source devices of type audio").contains("not found"));
        assert!(explain_error("Error: Access is denied. (0x80070005)").contains("blocking"));
        assert!(explain_error("something odd happened").contains("something odd"));
    }

    #[test]
    fn the_microphone_is_picked_by_name_then_default() {
        let names = vec!["Microphone (Realtek Audio)".to_string(), "Headset (USB)".to_string()];
        assert_eq!(pick_device_name(&names, Some("headset (usb)"), None).as_deref(), Some("Headset (USB)"));
        assert_eq!(pick_device_name(&names, Some("Headset"), None).as_deref(), Some("Headset (USB)"));
        assert_eq!(pick_device_name(&names, None, Some("Headset (USB)")).as_deref(), Some("Headset (USB)"));
        assert_eq!(pick_device_name(&names, Some("gone"), None).as_deref(), Some("Microphone (Realtek Audio)"));
        assert_eq!(pick_device_name(&[], None, None), None);
    }

    #[test]
    fn a_silent_state_has_a_message_and_a_good_one_does_not() {
        assert!(MicState::Silent { peak_db: -100.0 }.message().unwrap().starts_with("Mic not recorded"));
        assert!(MicState::Failed { reason: "x".into() }.message().unwrap().ends_with('x'));
        assert!(MicState::Ok { peak_db: -20.0 }.message().is_none());
    }

    #[test]
    fn volumedetect_output_is_parsed() {
        assert_eq!(parse_max_volume("[Parsed_volumedetect_0 @ 0x1] max_volume: -23.4 dB\n"), Some(-23.4));
        assert_eq!(parse_max_volume("nothing"), None);
    }

    #[test]
    fn a_silent_file_and_a_loud_one_are_told_apart() {
        if !crate::ffmpeg::ffmpeg_found() {
            return;
        }
        let d = std::env::temp_dir().join(format!("dl-micpeak-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let make = |src: &str, name: &str| {
            let p = d.join(name);
            let ok = crate::proc::command(crate::ffmpeg::ffmpeg_bin()).args(["-v", "error", "-y", "-f", "lavfi", "-i", src, "-t", "1", "-c:a", "pcm_f32le"]).arg(&p).status().unwrap().success();
            assert!(ok);
            p
        };
        let quiet = make("anullsrc=r=48000:cl=mono", "q.mkv");
        let loud = make("sine=frequency=440:sample_rate=48000", "l.mkv");
        assert!(file_peak_db(&quiet, 2.0).unwrap() < SILENT_DB);
        assert!(file_peak_db(&loud, 2.0).unwrap() > -20.0);
        let _ = std::fs::remove_dir_all(&d);
    }
}

impl Drop for MicRec {
    fn drop(&mut self) {
        if let Some(Backend::Ffmpeg { child, .. }) = self.backend.as_mut() {
            if let Ok(None) = child.try_wait() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
