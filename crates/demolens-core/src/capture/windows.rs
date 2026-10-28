//! Windows capture via ffmpeg's gdigrab input.
//!
//! TODO(windows): native Windows.Graphics.Capture backend (windows-capture
//! crate) for GPU capture, and WASAPI loopback for system audio.

use super::{s, CapturePlan, CaptureTarget, RecordOptions, Region};
use anyhow::Result;

pub(crate) fn plan(o: &RecordOptions) -> Result<CapturePlan> {
    let warnings: Vec<String> = Vec::new();
    let mut args = vec![s("-f"), s("gdigrab"), s("-draw_mouse"), s("0"), s("-framerate"), s(o.fps)];
    let region = match &o.target {
        CaptureTarget::Screen { .. } => {
            args.extend([s("-i"), s("desktop")]);
            // gdigrab's "desktop" is the whole virtual desktop, which starts
            // at a negative x or y when a monitor sits left of or above the
            // primary one. Pointer positions are logged relative to this.
            let (x, y, w, h) = super::winapi::virtual_screen();
            Region { x, y, w: w & !1, h: h & !1 }
        }
        CaptureTarget::Window { id } => {
            args.extend([s("-i"), format!("title={id}")]);
            // The pointer is logged relative to the window's corner.
            match super::winapi::window_rect_by_title(id) {
                Some((x, y, w, h)) => Region { x, y, w, h },
                None => Region { x: 0, y: 0, w: 0, h: 0 },
            }
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
    // The microphone is recorded in process through WASAPI (DirectShow is the
    // fallback); see `capture::mic`. The marker only says "record one".
    let mic_sidecar = if o.mic { Some(Vec::new()) } else { None };
    // System audio is also recorded on the side (WASAPI loopback), on the same clock.
    if mic_sidecar.is_some() || o.system_audio {
        // Keep the screen's own (wall clock) timestamps, relative to the start
        // of the recording, so the sound recorded on the side lands where it
        // really happened relative to the first frame.
        args.splice(0..0, [s("-copyts"), s("-itsoffset"), s(super::CLOCK_START)]);
    }
    Ok(CapturePlan { region, args, out_args: Vec::new(), warnings, feed: None, mic_sidecar })
}

/// Windows with a title and their frames, for the picker. Recording a window is by title.
pub(crate) fn list_sources() -> Result<Vec<super::sources::SourceInfo>> {
    use super::sources::{parse_win_lines, sort_sources, SourceInfo};
    let (vx, vy, vw, vh) = super::winapi::virtual_screen();
    let mut v = vec![SourceInfo { kind: "display".into(), id: "0".into(), title: "Entire screen".into(), app: String::new(), bundle: String::new(), x: vx, y: vy, w: vw, h: vh, z: 0 }];
    let me = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase())).unwrap_or_default();
    let mut wins = parse_win_lines(&super::winapi::window_lines());
    wins.retain(|w| w.app.to_lowercase() != me && !w.app.to_lowercase().contains("demolens"));
    for (i, w) in wins.iter_mut().enumerate() {
        w.z = i as u32;
    }
    v.extend(wins);
    sort_sources(&mut v);
    Ok(v)
}
