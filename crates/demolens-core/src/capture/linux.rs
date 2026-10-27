//! Linux capture: X11 via ffmpeg x11grab, audio via PulseAudio (also served
//! by PipeWire's pulse shim).
//!
//! TODO(wayland): request a ScreenCast session from xdg-desktop-portal
//! (org.freedesktop.portal.ScreenCast), then read the PipeWire node with
//! ffmpeg's pipewire input or a GStreamer pipewiresrc. Until then Wayland
//! sessions must run under XWayland or use the X11 session.

use super::{s, CapturePlan, CaptureTarget, RecordOptions, Region};
use anyhow::{bail, Context, Result};
use std::process::Command;

fn screen_size(display: &str) -> Result<(u32, u32)> {
    let out = Command::new("xdpyinfo").env("DISPLAY", display).output().context("xdpyinfo not found (install x11-utils)")?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("dimensions:") {
            let dims = rest.split_whitespace().next().unwrap_or("");
            if let Some((w, h)) = dims.split_once('x') {
                return Ok((w.parse()?, h.parse()?));
            }
        }
    }
    bail!("could not read screen size from xdpyinfo")
}

fn window_geometry(display: &str, id: &str) -> Result<Region> {
    let out = Command::new("xwininfo").env("DISPLAY", display).args(["-id", id]).output().context("xwininfo not found (install x11-utils)")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let get = |k: &str| -> Option<i64> {
        text.lines().find_map(|l| l.trim().strip_prefix(k).and_then(|v| v.trim().parse().ok()))
    };
    match (get("Absolute upper-left X:"), get("Absolute upper-left Y:"), get("Width:"), get("Height:")) {
        (Some(x), Some(y), Some(w), Some(h)) => Ok(Region { x: x as i32, y: y as i32, w: w as u32 & !1, h: h as u32 & !1 }),
        _ => bail!("window {id} not found"),
    }
}

pub(crate) fn plan(o: &RecordOptions) -> Result<CapturePlan> {
    let mut warnings = Vec::new();
    if std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("DISPLAY").is_err() {
        bail!("pure Wayland capture is not implemented yet (TODO: xdg-desktop-portal + PipeWire). Run under X11 or XWayland.");
    }
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    let region = match &o.target {
        CaptureTarget::Screen { .. } => {
            let (w, h) = screen_size(&display)?;
            Region { x: 0, y: 0, w: w & !1, h: h & !1 }
        }
        CaptureTarget::Window { id } => window_geometry(&display, id)?,
        CaptureTarget::Area { x, y, w, h } => Region { x: *x, y: *y, w: w & !1, h: h & !1 },
    };
    let mut args = vec![
        s("-f"), s("x11grab"),
        s("-draw_mouse"), s("0"), // DemoLens draws its own cursor
        s("-framerate"), s(o.fps),
        s("-video_size"), format!("{}x{}", region.w, region.h),
        s("-i"), format!("{display}+{},{}", region.x, region.y),
    ];
    let mut audio_inputs = 0;
    if o.mic {
        args.extend([s("-f"), s("pulse"), s("-i"), o.mic_device.clone().unwrap_or_else(|| s("default"))]);
        audio_inputs += 1;
    }
    if o.system_audio {
        args.extend([s("-f"), s("pulse"), s("-i"), s("@DEFAULT_MONITOR@")]);
        audio_inputs += 1;
    }
    if audio_inputs == 2 {
        args.extend([s("-filter_complex"), s("[1:a][2:a]amix=inputs=2:duration=longest[a]"), s("-map"), s("0:v"), s("-map"), s("[a]")]);
    }
    if audio_inputs == 0 {
        warnings.push("recording without audio".into());
    }
    Ok(CapturePlan { region, args, out_args: Vec::new(), warnings, feed: None })
}
