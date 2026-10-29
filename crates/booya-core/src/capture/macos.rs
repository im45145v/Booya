//! macOS capture. Video comes from ScreenCaptureKit (`macos_sck`) and is piped
//! to ffmpeg as raw frames. The microphone is an audio-only avfoundation input.
//!
//! The old avfoundation screen input (AVCaptureScreenInput) is deprecated and
//! produces no frames on recent macOS. It is kept behind
//! `BOOYA_MAC_CAPTURE=avfoundation` for machines where ScreenCaptureKit is
//! not available.
//!
//! Devices are looked up by listing them first (`-list_devices true`), because
//! the index of a device depends on what is attached.
//!
//! TODO(macos): record the microphone through SCStream on macOS 15+.

use super::{macos_sck, parse_avfoundation_devices, s, screen_index, AvDevice, CapturePlan, CaptureTarget, Feed, RecordOptions, Region};
use anyhow::{anyhow, bail, Result};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Runs the device listing with a timeout, so a stuck permission prompt can
/// not freeze the app.
pub(crate) fn list_devices() -> (Vec<AvDevice>, Vec<AvDevice>) {
    let child = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
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

/// The built in microphone when there is one: a Bluetooth headset that is
/// listed first would drop its sound quality as soon as it is used as an input.
pub(crate) fn pick_mic<'a>(audio: &'a [AvDevice], wanted: Option<&str>) -> Option<&'a AvDevice> {
    if let Some(w) = wanted {
        if let Some(d) = audio.iter().find(|d| d.name == w).or_else(|| audio.iter().find(|d| d.name.to_lowercase().contains(&w.to_lowercase()))) {
            return Some(d);
        }
    }
    audio
        .iter()
        .find(|d| super::mics::is_builtin(&d.name))
        .or_else(|| audio.iter().find(|d| d.name.to_lowercase().contains("microphone")))
        .or_else(|| audio.first())
}

pub(crate) fn plan(o: &RecordOptions) -> Result<CapturePlan> {
    if std::env::var("BOOYA_MAC_CAPTURE").as_deref() == Ok("avfoundation") {
        return plan_avfoundation(o);
    }
    // System audio comes from a ScreenCaptureKit audio stream, recorded on the side.
    let mut warnings = Vec::new();
    let fps = o.fps.clamp(1, 60);
    let cap = macos_sck::open(&o.target, fps, o.max_height, std::time::Duration::from_secs(8)).map_err(|e| anyhow!(e))?;
    // (w, h) is the size of the screen in capture pixels, which is what events
    // and area picks use; the stream may be smaller (see `scale`).
    let scale = cap.scale.clamp(0.1, 1.0);
    let (sw, sh) = (cap.width, cap.height);
    let (w, h) = if scale < 1.0 { ((sw as f64 / scale).round() as u32, (sh as f64 / scale).round() as u32) } else { (sw, sh) };
    // Every input is stamped with the wall clock minus the moment recording
    // began, and the timestamps are kept (`-copyts`), so the microphone lands
    // where it really happened relative to the first frame instead of both
    // tracks being shifted to start at zero. The event log uses the same
    // origin, so clicks, zoom and sound all line up. The screen is raw frames
    // with no clock of their own, so this wall clock plus a constant frame
    // rate is its timeline. The camera and the microphone keep only the first
    // packet's wall-clock time and then run at their own rate: spacing every
    // packet by when it was read drops camera frames and stacks the audio.
    let t0 = super::CLOCK_START;
    let mut out_args = vec![s("-fps_mode"), s("cfr"), s("-r"), s(fps)];
    let region = match &o.target {
        CaptureTarget::Area { x, y, w: aw, h: ah } => {
            let (aw, ah) = (aw & !1, ah & !1);
            if aw < 2 || ah < 2 || *x < 0 || *y < 0 || *x as u32 + aw > w || *y as u32 + ah > h {
                bail!("The selected area ({x},{y} {aw}x{ah}) is outside the screen ({w}x{h}). Pick the area again.");
            }
            let (cx, cy, cw, ch) = if scale < 1.0 {
                (((*x as f64 * scale).round() as u32) & !1, ((*y as f64 * scale).round() as u32) & !1, ((aw as f64 * scale).round() as u32 & !1).max(2), ((ah as f64 * scale).round() as u32 & !1).max(2))
            } else {
                (*x as u32, *y as u32, aw, ah)
            };
            let (cw, ch) = (cw.min(sw.saturating_sub(cx)).max(2) & !1, ch.min(sh.saturating_sub(cy)).max(2) & !1);
            out_args.extend([s("-vf"), format!("crop={cw}:{ch}:{cx}:{cy}")]);
            Region { x: *x, y: *y, w: aw, h: ah }
        }
        _ => Region { x: cap.origin.0, y: cap.origin.1, w, h },
    };
    let mut args = vec![
        s("-copyts"),
        s("-thread_queue_size"), s("8"),
        s("-use_wallclock_as_timestamps"), s("1"),
        s("-itsoffset"), s(t0),
        s("-f"), s("rawvideo"),
        s("-pixel_format"), s("bgra"),
        s("-video_size"), format!("{sw}x{sh}"),
        s("-framerate"), s(fps),
        s("-i"), s("pipe:0"),
    ];
    // The microphone is NOT an input of this ffmpeg. Timestamp based
    // resampling of a wall clock stamped device (aresample async) inserted
    // runs of digital silence and cut samples whenever the 4K video path was
    // busy. It is recorded by its own ffmpeg and joined at the end.
    let mut mic_sidecar = None;
    if o.mic {
        let (_, audio) = list_devices();
        match pick_mic(&audio, o.mic_device.as_deref()) {
            Some(d) => mic_sidecar = Some(vec![s("-f"), s("avfoundation"), s("-i"), format!(":{}", d.index)]),
            None => warnings.push("No microphone was found, so this recording has no sound.".into()),
        }
    }
    let feed: Feed = Box::new(move |stdin| Box::new(macos_sck::feed(cap, stdin, fps)));
    Ok(CapturePlan { region, args, out_args, warnings, feed: Some(feed), mic_sidecar })
}

/// Fallback: ffmpeg's avfoundation screen input (does not work on recent macOS).
fn plan_avfoundation(o: &RecordOptions) -> Result<CapturePlan> {
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
            "No screen to capture was found. ffmpeg lists: {}. Allow Booya in System Settings > Privacy & Security > Screen & System Audio Recording, then reopen it.",
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
    Ok(CapturePlan { region, args, out_args: Vec::new(), warnings, feed: None, mic_sidecar: None })
}

/// avfoundation index of the first camera (not a screen), if any.
pub fn camera_index() -> Option<u32> {
    let (video, _) = list_devices();
    video.iter().find(|d| !d.name.starts_with("Capture screen")).map(|d| d.index)
}

/// One PNG of the screen for the area picker.
pub(crate) fn screenshot() -> Result<(Vec<u8>, u32, u32)> {
    macos_sck::screenshot_png().map_err(|e| anyhow!(e))
}
