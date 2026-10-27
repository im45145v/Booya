//! macOS capture via ffmpeg's avfoundation input.
//!
//! Devices are looked up by listing them first (`-list_devices true`), because
//! the index of "Capture screen 0" depends on how many cameras are attached
//! (FaceTime camera, iPhone Continuity Camera, virtual cameras).
//!
//! TODO(macos): replace with a native ScreenCaptureKit backend
//! (screencapturekit crate) to get per-window capture and system audio.

use super::{parse_avfoundation_devices, s, screen_index, AvDevice, CapturePlan, CaptureTarget, RecordOptions, Region};
use anyhow::{bail, Result};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs the device listing with a timeout, so a stuck permission prompt can
/// not freeze the app.
fn list_devices() -> (Vec<AvDevice>, Vec<AvDevice>) {
    let child = Command::new(crate::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else { return (vec![], vec![]) };
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return (vec![], vec![]);
            }
        }
    }
    let mut text = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut e, &mut text);
    }
    parse_avfoundation_devices(&text)
}

pub(crate) fn plan(o: &RecordOptions) -> Result<CapturePlan> {
    let mut warnings = Vec::new();
    let (display, crop) = match &o.target {
        CaptureTarget::Screen { display } => (*display, None),
        CaptureTarget::Area { x, y, w, h } => (0, Some(Region { x: *x, y: *y, w: w & !1, h: h & !1 })),
        CaptureTarget::Window { .. } => bail!("Window capture on macOS is not supported yet. Use Full screen or Area."),
    };
    if o.system_audio {
        warnings.push("System audio on macOS needs a loopback device such as BlackHole for now.".into());
    }
    let (video, audio) = list_devices();
    let screen = match screen_index(&video, display) {
        Some(i) => i.to_string(),
        None if video.is_empty() => format!("Capture screen {display}"),
        None => bail!(
            "No screen to capture was found. ffmpeg lists: {}. Allow DemoLens in System Settings > Privacy & Security > Screen & System Audio Recording, then reopen it.",
            video.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")
        ),
    };
    let mic = if !o.mic {
        "none".to_string()
    } else if let Some(d) = audio.first() {
        d.index.to_string()
    } else if audio.is_empty() && !video.is_empty() {
        warnings.push("No microphone was found, so this recording has no sound.".into());
        "none".to_string()
    } else {
        "default".to_string()
    };
    let mut args = vec![
        s("-thread_queue_size"), s("1024"),
        s("-f"), s("avfoundation"),
        s("-capture_cursor"), s("0"),
        s("-capture_mouse_clicks"), s("0"),
        s("-pixel_format"), s("uyvy422"),
        s("-framerate"), s(o.fps),
        s("-i"), format!("{screen}:{mic}"),
    ];
    let region = crop.unwrap_or(Region { x: 0, y: 0, w: 0, h: 0 });
    if let Some(r) = crop {
        args.extend([s("-vf"), format!("crop={}:{}:{}:{}", r.w, r.h, r.x, r.y)]);
    }
    Ok(CapturePlan { region, args, warnings })
}

/// avfoundation index of the first camera (not a screen), if any.
pub fn camera_index() -> Option<u32> {
    let (video, _) = list_devices();
    video.iter().find(|d| !d.name.starts_with("Capture screen")).map(|d| d.index)
}
