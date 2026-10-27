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
    use std::process::{Command, Stdio};
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
    let out = Command::new(crate::ffmpeg::ffmpeg_bin()).args(&args).stdin(Stdio::null()).output().context("failed to start ffmpeg")?;
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
        SourceInfo { kind: "window".into(), id: "1".into(), title: title.into(), app: app.into(), bundle: String::new(), x: 0, y: 0, w: 800, h: 600 }
    }

    #[test]
    fn displays_come_first_and_windows_are_grouped_by_app() {
        let mut v = vec![win("Safari", "b"), win("Notes", "z"), SourceInfo { kind: "display".into(), ..win("", "Screen 1") }, win("Notes", "a")];
        sort_sources(&mut v);
        let names: Vec<_> = v.iter().map(|s| format!("{}:{}", s.app, s.title)).collect();
        assert_eq!(names, [":Screen 1", "Notes:a", "Notes:z", "Safari:b"]);
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
