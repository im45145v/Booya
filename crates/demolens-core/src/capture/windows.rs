//! Windows capture via ffmpeg's gdigrab input.
//!
//! TODO(windows): native Windows.Graphics.Capture backend (windows-capture
//! crate) for GPU capture, and WASAPI loopback for system audio.

use super::{s, CapturePlan, CaptureTarget, RecordOptions, Region};
use anyhow::Result;

pub(crate) fn plan(o: &RecordOptions) -> Result<CapturePlan> {
    let mut warnings = Vec::new();
    let mut args = vec![s("-f"), s("gdigrab"), s("-draw_mouse"), s("0"), s("-framerate"), s(o.fps)];
    let region = match &o.target {
        CaptureTarget::Screen { .. } => {
            args.extend([s("-i"), s("desktop")]);
            // TODO(windows): GetSystemMetrics for exact size; editor reads it from the video.
            Region { x: 0, y: 0, w: 0, h: 0 }
        }
        CaptureTarget::Window { id } => {
            args.extend([s("-i"), format!("title={id}")]);
            Region { x: 0, y: 0, w: 0, h: 0 }
        }
        CaptureTarget::Area { x, y, w, h } => {
            args.extend([
                s("-offset_x"), s(x), s("-offset_y"), s(y),
                s("-video_size"), format!("{}x{}", w & !1, h & !1),
                s("-i"), s("desktop"),
            ]);
            Region { x: *x, y: *y, w: w & !1, h: h & !1 }
        }
    };
    if o.mic {
        // TODO(windows): enumerate dshow devices and let the user pick.
        let dev = std::env::var("DEMOLENS_MIC").unwrap_or_else(|_| "Microphone".into());
        args.extend([s("-f"), s("dshow"), s("-i"), format!("audio={dev}")]);
    }
    if o.system_audio {
        warnings.push("system audio on Windows needs WASAPI loopback (TODO)".into());
    }
    Ok(CapturePlan { region, args, warnings })
}
