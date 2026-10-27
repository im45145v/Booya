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
        let dev = o.mic_device.clone().or_else(|| std::env::var("DEMOLENS_MIC").ok()).unwrap_or_else(|| "Microphone".into());
        args.extend([s("-f"), s("dshow"), s("-i"), format!("audio={dev}")]);
    }
    if o.system_audio {
        warnings.push("system audio on Windows needs WASAPI loopback (TODO)".into());
    }
    Ok(CapturePlan { region, args, out_args: Vec::new(), warnings, feed: None })
}

/// Windows with a title, for the picker. Recording a window is by title.
pub(crate) fn list_sources() -> Result<Vec<super::sources::SourceInfo>> {
    use super::sources::{sort_sources, SourceInfo};
    use std::os::windows::process::CommandExt;
    let mut v = vec![SourceInfo { kind: "display".into(), id: "0".into(), title: "Entire screen".into(), app: String::new(), bundle: String::new(), x: 0, y: 0, w: 0, h: 0 }];
    let script = "Get-Process | Where-Object { $_.MainWindowTitle } | ForEach-Object { $_.ProcessName + [char]9 + $_.MainWindowTitle }";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
    if let Ok(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some((app, title)) = line.split_once('\t') {
                let title = title.trim();
                if !title.is_empty() {
                    v.push(SourceInfo { kind: "window".into(), id: title.to_string(), title: title.to_string(), app: app.to_string(), bundle: String::new(), x: 0, y: 0, w: 0, h: 0 });
                }
            }
        }
    }
    sort_sources(&mut v);
    Ok(v)
}
