//! macOS capture via ffmpeg's avfoundation input.
//!
//! TODO(macos): replace with a native ScreenCaptureKit backend
//! (screencapturekit crate) to get per-window capture, system audio and
//! Retina-aware scaling without ffmpeg's device index guessing.

use super::{s, CapturePlan, CaptureTarget, RecordOptions, Region};
use anyhow::{bail, Result};

pub(crate) fn plan(o: &RecordOptions) -> Result<CapturePlan> {
    let mut warnings = Vec::new();
    // avfoundation lists screens after cameras; "Capture screen N" is
    // addressed by name to avoid index guessing.
    let (display, crop) = match &o.target {
        CaptureTarget::Screen { display } => (*display, None),
        CaptureTarget::Area { x, y, w, h } => (0, Some(Region { x: *x, y: *y, w: w & !1, h: h & !1 })),
        CaptureTarget::Window { .. } => bail!("window capture on macOS needs the ScreenCaptureKit backend (TODO). Use area capture for now."),
    };
    if o.system_audio {
        warnings.push("system audio on macOS needs ScreenCaptureKit (TODO) or a loopback device such as BlackHole".into());
    }
    let audio = if o.mic { ":default" } else { ":none" };
    let mut args = vec![
        s("-f"), s("avfoundation"),
        s("-capture_cursor"), s("0"),
        s("-framerate"), s(o.fps),
        s("-i"), format!("Capture screen {display}{audio}"),
    ];
    // TODO(macos): query the real display size (CGDisplayBounds) for the
    // event log header. Full screen falls back to 0x0 and the editor reads
    // the true size from the video.
    let region = crop.unwrap_or(Region { x: 0, y: 0, w: 0, h: 0 });
    if let Some(r) = crop {
        args.extend([s("-vf"), format!("crop={}:{}:{}:{}", r.w, r.h, r.x, r.y)]);
    }
    Ok(CapturePlan { region, args, warnings })
}
