//! What can be recorded: displays and windows, for the visual picker.
//!
//! The list is cheap (names and sizes only). Thumbnails and app icons are
//! fetched one at a time afterwards (`thumbnail`), so the picker opens at
//! once and fills in as items scroll into view.

use super::CaptureTarget;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SourceInfo {
    /// "display" or "window".
    pub kind: String,
    /// Display index, or the window id (macOS window number, X11 id, Windows title).
    pub id: String,
    pub title: String,
    /// Owning application, empty when unknown.
    pub app: String,
    /// macOS bundle identifier (used to look up the app icon), else empty.
    pub bundle: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Stacking order among windows, 0 is frontmost. Displays use 0.
    pub z: u32,
}

impl SourceInfo {
    /// The capture target that records this source.
    pub fn target(&self) -> CaptureTarget {
        if self.kind == "display" {
            CaptureTarget::Screen { display: self.id.parse().unwrap_or(0) }
        } else {
            CaptureTarget::Window { id: self.id.clone() }
        }
    }
}

/// A preview image and, when known, the app icon (both PNG bytes).
#[derive(Debug, Default, Clone)]
pub struct Thumb {
    pub image: Option<Vec<u8>>,
    pub icon: Option<Vec<u8>>,
}

/// Width of a picker thumbnail in pixels (shown at half size on retina).
pub const THUMB_W: u32 = 360;

pub fn list_sources() -> Result<Vec<SourceInfo>> {
    #[cfg(target_os = "macos")]
    {
        super::macos_sck::list_sources().map_err(|e| anyhow::anyhow!(e))
    }
    #[cfg(target_os = "linux")]
    {
        super::linux::list_sources()
    }
    #[cfg(target_os = "windows")]
    {
        super::windows::list_sources()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Ok(Vec::new())
    }
}

/// Every window the system knows about and whether the picker lists it (and
/// why not). Only macOS keeps such a report for now.
pub fn window_report() -> Result<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        super::macos_sck::window_report().map_err(|e| anyhow::anyhow!(e))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(vec!["The window report is only available on macOS. `booya sources` shows what the picker lists.".to_string()])
    }
}

pub fn thumbnail(src: &SourceInfo) -> Result<Thumb> {
    #[cfg(target_os = "macos")]
    {
        super::macos_sck::thumbnail(src).map_err(|e| anyhow::anyhow!(e))
    }
    #[cfg(not(target_os = "macos"))]
    {
        grab_with_ffmpeg(src).map(|image| Thumb { image: Some(image), icon: None })
    }
}

/// One scaled frame of a screen region through ffmpeg (Linux and Windows).
#[cfg(not(target_os = "macos"))]
fn grab_with_ffmpeg(src: &SourceInfo) -> Result<Vec<u8>> {
    use anyhow::{bail, Context};
    use std::process::Stdio;
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-v".into(), "error".into(), "-nostdin".into()];
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        args.extend(["-f".into(), "x11grab".into(), "-draw_mouse".into(), "0".into()]);
        if src.w > 0 && src.h > 0 {
            args.extend(["-video_size".into(), format!("{}x{}", src.w & !1, src.h & !1)]);
        }
        args.extend(["-i".into(), format!("{display}+{},{}", src.x.max(0), src.y.max(0))]);
    }
    #[cfg(target_os = "windows")]
    {
        args.extend(["-f".into(), "gdigrab".into(), "-draw_mouse".into(), "0".into()]);
        if src.kind == "display" {
            args.extend(["-i".into(), "desktop".into()]);
        } else {
            args.extend(["-i".into(), format!("title={}", src.id)]);
        }
    }
    args.extend(["-frames:v".into(), "1".into(), "-vf".into(), format!("scale={THUMB_W}:-2"), "-f".into(), "image2pipe".into(), "-c:v".into(), "png".into(), "-".into()]);
    let out = crate::proc::command(crate::ffmpeg::ffmpeg_bin()).args(&args).stdin(Stdio::null()).output().context("failed to start ffmpeg")?;
    if out.stdout.len() < 100 {
        bail!("no preview: {}", String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or(""));
    }
    Ok(out.stdout)
}

/// RGBA pixels to PNG bytes.
pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header()?;
        wr.write_image_data(rgba)?;
    }
    Ok(out)
}

/// Parses the Windows window script output into window sources. Each line is
/// `process<TAB>left<TAB>top<TAB>right<TAB>bottom<TAB>title`, frontmost first.
/// Tiny, empty and off screen windows are skipped.
pub fn parse_win_lines(text: &str) -> Vec<SourceInfo> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.trim_end_matches('\r').splitn(6, '\t').collect();
        if f.len() < 6 {
            continue;
        }
        let (Ok(l), Ok(t), Ok(r), Ok(b)) = (f[1].trim().parse::<i32>(), f[2].trim().parse::<i32>(), f[3].trim().parse::<i32>(), f[4].trim().parse::<i32>()) else { continue };
        let title = f[5].trim();
        let (w, h) = (r - l, b - t);
        if title.is_empty() || w < 120 || h < 80 || r <= 0 || b <= 0 {
            continue;
        }
        let z = out.len() as u32;
        out.push(SourceInfo { kind: "window".into(), id: title.to_string(), title: title.to_string(), app: f[0].trim().to_string(), bundle: String::new(), x: l, y: t, w: w as u32, h: h as u32, z });
    }
    out
}

/// Stacking rank (0 = frontmost) for each id, given the window ids of the
/// system's own front to back list. Ids missing from that list go behind all
/// listed ones, in their input order. Ranks are consecutive, so they stay
/// small and stable whatever the system's numbering looks like.
pub fn stacking_ranks(ids: &[u32], front_to_back: &[u32]) -> Vec<u32> {
    let key = |i: usize| (front_to_back.iter().position(|w| *w == ids[i]).unwrap_or(usize::MAX), i);
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|i| key(*i));
    let mut ranks = vec![0u32; ids.len()];
    for (rank, i) in order.into_iter().enumerate() {
        ranks[i] = rank as u32;
    }
    ranks
}

/// Sorts a picker list: displays first, then windows by app and title.
pub fn sort_sources(v: &mut [SourceInfo]) {
    v.sort_by(|a, b| {
        (a.kind != "display", a.app.to_lowercase(), a.title.to_lowercase()).cmp(&(b.kind != "display", b.app.to_lowercase(), b.title.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(app: &str, title: &str) -> SourceInfo {
        SourceInfo { kind: "window".into(), id: "1".into(), title: title.into(), app: app.into(), bundle: String::new(), x: 0, y: 0, w: 800, h: 600, z: 0 }
    }

    #[test]
    fn displays_come_first_and_windows_are_grouped_by_app() {
        let mut v = vec![win("Safari", "b"), win("Notes", "z"), SourceInfo { kind: "display".into(), ..win("", "Screen 1") }, win("Notes", "a")];
        sort_sources(&mut v);
        let names: Vec<_> = v.iter().map(|s| format!("{}:{}", s.app, s.title)).collect();
        assert_eq!(names, [":Screen 1", "Notes:a", "Notes:z", "Safari:b"]);
    }

    #[test]
    fn windows_script_lines_become_stacked_windows() {
        let text = "chrome\t100\t50\t1380\t850\tInbox - Chrome\r\nExplorer\t10\t10\t60\t40\tTiny\nnotepad\t-5\t0\t600\t400\tnotes.txt - Notepad\nbroken line\ncode\t0\t0\t900\t700\t\n";
        let v = parse_win_lines(text);
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!((v[0].app.as_str(), v[0].x, v[0].y, v[0].w, v[0].h, v[0].z), ("chrome", 100, 50, 1280, 800, 0));
        assert_eq!((v[1].title.as_str(), v[1].x, v[1].w, v[1].z), ("notes.txt - Notepad", -5, 605, 1));
    }

    #[test]
    fn the_frontmost_window_gets_the_lowest_rank() {
        // The system lists 90 first (front), then 7. Our own list order is the reverse.
        let ranks = stacking_ranks(&[7, 90, 55], &[90, 31, 7]);
        assert_eq!(ranks, [1, 0, 2], "unlisted windows go last");
        // Without any system order the input order is kept.
        assert_eq!(stacking_ranks(&[5, 6, 7], &[]), [0, 1, 2]);
    }

    #[test]
    fn a_source_maps_to_its_capture_target() {
        let d = SourceInfo { kind: "display".into(), id: "1".into(), ..win("", "") };
        assert_eq!(d.target(), CaptureTarget::Screen { display: 1 });
        assert_eq!(win("A", "t").target(), CaptureTarget::Window { id: "1".into() });
    }

    #[test]
    fn png_encoding_round_trips_the_size() {
        let png = encode_png(&[255u8; 4 * 4 * 2], 4, 2).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
