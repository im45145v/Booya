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

/// PowerShell that lists visible top level windows front to back with their
/// exact frame (DPI aware, shadow free): process, left, top, right, bottom, title.
const LIST_WINDOWS_PS: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public class DLWin {
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int n);
  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int n);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static string[] List() {
    SetProcessDPIAware();
    var o = new List<string>();
    EnumWindows((h, l) => {
      if (!IsWindowVisible(h) || IsIconic(h)) return true;
      int cloaked; if (DwmGetWindowAttribute(h, 14, out cloaked, 4) == 0 && cloaked != 0) return true;
      var sb = new StringBuilder(512); GetWindowText(h, sb, 512);
      if (sb.Length == 0) return true;
      RECT r; if (DwmGetWindowAttribute(h, 9, out r, 16) != 0 && !GetWindowRect(h, out r)) return true;
      uint pid; GetWindowThreadProcessId(h, out pid);
      o.Add(pid + "\t" + r.L + "\t" + r.T + "\t" + r.R + "\t" + r.B + "\t" + sb.ToString());
      return true;
    }, IntPtr.Zero);
    return o.ToArray();
  }
}
"@
foreach ($l in [DLWin]::List()) {
  $p = $l.Split([char]9, 2)
  $name = (Get-Process -Id ([int]$p[0])).ProcessName
  if ($name) { $name + [char]9 + $p[1] }
}
"#;

/// Windows with a title and their frames, for the picker. Recording a window is by title.
pub(crate) fn list_sources() -> Result<Vec<super::sources::SourceInfo>> {
    use super::sources::{parse_win_lines, sort_sources, SourceInfo};
    use std::os::windows::process::CommandExt;
    let mut v = vec![SourceInfo { kind: "display".into(), id: "0".into(), title: "Entire screen".into(), app: String::new(), bundle: String::new(), x: 0, y: 0, w: 0, h: 0, z: 0 }];
    // A script file keeps the here-string and its quotes intact.
    let script = std::env::temp_dir().join(format!("demolens-windows-{}.ps1", std::process::id()));
    let out = std::fs::write(&script, LIST_WINDOWS_PS).ok().and_then(|_| {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .ok()
    });
    let _ = std::fs::remove_file(&script);
    if let Some(o) = out {
        let me = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase())).unwrap_or_default();
        let mut wins = parse_win_lines(&String::from_utf8_lossy(&o.stdout));
        wins.retain(|w| w.app.to_lowercase() != me && !w.app.to_lowercase().contains("demolens"));
        for (i, w) in wins.iter_mut().enumerate() {
            w.z = i as u32;
        }
        v.extend(wins);
    }
    sort_sources(&mut v);
    Ok(v)
}
