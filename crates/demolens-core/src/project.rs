//! Project model: everything the editor changes, saved as one JSON file.

use crate::zoom::{ZoomConfig, ZoomSegment};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Background {
    Solid { color: [u8; 3] },
    /// Linear gradient, `angle_deg` 0 = left to right, 90 = top to bottom.
    Gradient { from: [u8; 3], to: [u8; 3], angle_deg: f64 },
    Image { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub background: Background,
    /// Padding as a fraction of the shorter output side (0.0 to 0.4).
    pub padding: f64,
    /// Corner radius as a fraction of the shorter output side.
    pub corner_radius: f64,
    /// Shadow blur size as a fraction of the shorter output side. 0 = off.
    pub shadow: f64,
    /// Shadow opacity 0..1.
    pub shadow_opacity: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: Background::Gradient { from: [99, 102, 241], to: [236, 72, 153], angle_deg: 35.0 },
            padding: 0.07,
            corner_radius: 0.018,
            shadow: 0.035,
            shadow_opacity: 0.45,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorStyle {
    pub visible: bool,
    /// 1.0 = normal, 2.0 = double size.
    pub size: f64,
    /// Smoothing window in ms. 0 = raw path.
    pub smoothing_ms: f64,
    pub click_ripples: bool,
    pub ripple_color: [u8; 3],
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self { visible: true, size: 1.5, smoothing_ms: 90.0, click_ripples: true, ripple_color: [255, 255, 255] }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyOverlay {
    pub enabled: bool,
    /// Only show combos with Ctrl/Alt/Meta, not plain typing.
    pub shortcuts_only: bool,
    /// How long a label stays on screen, ms.
    pub hold_ms: f64,
}

impl Default for KeyOverlay {
    fn default() -> Self {
        Self { enabled: true, shortcuts_only: false, hold_ms: 1200.0 }
    }
}

/// One piece of the source kept in the output, played at `speed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub src_start_ms: f64,
    pub src_end_ms: f64,
    pub speed: f64,
}

impl Clip {
    pub fn out_len(&self) -> f64 {
        (self.src_end_ms - self.src_start_ms) / self.speed
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlurBox {
    /// Source pixels.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Source time range.
    pub start_ms: f64,
    pub end_ms: f64,
    /// Blur radius in source pixels.
    #[serde(default = "default_blur")]
    pub radius: u32,
}

fn default_blur() -> u32 {
    16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    #[serde(rename = "16:9")]
    Landscape,
    #[serde(rename = "9:16")]
    Portrait,
    #[serde(rename = "1:1")]
    Square,
}

impl Aspect {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "16:9" => Aspect::Landscape,
            "9:16" => Aspect::Portrait,
            "1:1" => Aspect::Square,
            _ => bail!("aspect must be 16:9, 9:16 or 1:1"),
        })
    }

    /// Output size for a quality level given as the short side
    /// (720, 1080, 1440 or 2160 for 4K).
    pub fn size(self, short_side: u32) -> (u32, u32) {
        let s = short_side.clamp(240, 2160) & !1;
        let long = ((s as f64 * 16.0 / 9.0).round() as u32) & !1;
        match self {
            Aspect::Landscape => (long, s),
            Aspect::Portrait => (s, long),
            Aspect::Square => (s, s),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mp4,
    Gif,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub aspect: Aspect,
    /// Short side in pixels. 2160 = 4K.
    pub quality: u32,
    pub fps: u32,
    pub format: Format,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { aspect: Aspect::Landscape, quality: 1080, fps: 30, format: Format::Mp4 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub source: PathBuf,
    pub events: Option<PathBuf>,
    pub source_width: u32,
    pub source_height: u32,
    pub duration_ms: f64,
    pub clips: Vec<Clip>,
    pub zoom: Vec<ZoomSegment>,
    #[serde(default)]
    pub zoom_config: ZoomConfigSer,
    pub style: Style,
    pub cursor: CursorStyle,
    pub keys: KeyOverlay,
    pub blur: Vec<BlurBox>,
    pub export: ExportSettings,
}

/// Serializable wrapper so old project files without zoom config still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomConfigSer {
    pub transition_ms: f64,
}

impl Default for ZoomConfigSer {
    fn default() -> Self {
        Self { transition_ms: ZoomConfig::default().transition_ms }
    }
}

impl Project {
    pub fn new(source: PathBuf, events: Option<PathBuf>, w: u32, h: u32, duration_ms: f64) -> Self {
        Self {
            version: 1,
            source,
            events,
            source_width: w,
            source_height: h,
            duration_ms,
            clips: vec![Clip { src_start_ms: 0.0, src_end_ms: duration_ms, speed: 1.0 }],
            zoom: Vec::new(),
            zoom_config: ZoomConfigSer::default(),
            style: Style::default(),
            cursor: CursorStyle::default(),
            keys: KeyOverlay::default(),
            blur: Vec::new(),
            export: ExportSettings::default(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn output_duration_ms(&self) -> f64 {
        self.clips.iter().map(Clip::out_len).sum()
    }

    /// Map output time to source time. Returns None past the end.
    pub fn source_time(&self, out_ms: f64) -> Option<f64> {
        let mut acc = 0.0;
        for c in &self.clips {
            let len = c.out_len();
            if out_ms < acc + len {
                return Some(c.src_start_ms + (out_ms - acc) * c.speed);
            }
            acc += len;
        }
        None
    }

    /// Trim the source to [start, end] (source ms), keeping existing speeds.
    pub fn trim(&mut self, start: f64, end: f64) {
        let mut out = Vec::new();
        for c in &self.clips {
            let s = c.src_start_ms.max(start);
            let e = c.src_end_ms.min(end);
            if e > s {
                out.push(Clip { src_start_ms: s, src_end_ms: e, speed: c.speed });
            }
        }
        self.clips = out;
    }

    /// Split the clip containing source time `at` into two.
    pub fn split(&mut self, at: f64) -> bool {
        if let Some(i) = self.clips.iter().position(|c| at > c.src_start_ms && at < c.src_end_ms) {
            let c = self.clips[i].clone();
            self.clips[i].src_end_ms = at;
            self.clips.insert(i + 1, Clip { src_start_ms: at, src_end_ms: c.src_end_ms, speed: c.speed });
            true
        } else {
            false
        }
    }

    /// Remove clip `index` (cut a piece out).
    pub fn delete_clip(&mut self, index: usize) {
        if index < self.clips.len() && self.clips.len() > 1 {
            self.clips.remove(index);
        }
    }

    pub fn set_speed(&mut self, index: usize, speed: f64) {
        if let Some(c) = self.clips.get_mut(index) {
            c.speed = speed.clamp(0.25, 16.0);
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut prev = f64::NEG_INFINITY;
        for c in &self.clips {
            if c.src_end_ms <= c.src_start_ms || c.speed <= 0.0 {
                bail!("invalid clip {:?}", c);
            }
            if c.src_start_ms < prev {
                bail!("clips must be in source order");
            }
            prev = c.src_end_ms;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_split_speed_mapping() {
        let mut p = Project::new("a.mp4".into(), None, 100, 100, 10_000.0);
        p.trim(1000.0, 9000.0);
        assert!(p.split(5000.0));
        p.set_speed(1, 2.0);
        assert_eq!(p.output_duration_ms(), 4000.0 + 2000.0);
        assert_eq!(p.source_time(0.0), Some(1000.0));
        assert_eq!(p.source_time(4000.0), Some(5000.0));
        assert_eq!(p.source_time(5000.0), Some(7000.0));
        assert_eq!(p.source_time(6000.0), None);
        p.validate().unwrap();
    }

    #[test]
    fn aspect_sizes() {
        assert_eq!(Aspect::Landscape.size(2160), (3840, 2160));
        assert_eq!(Aspect::Portrait.size(1080), (1080, 1920));
        assert_eq!(Aspect::Square.size(720), (720, 720));
    }
}
