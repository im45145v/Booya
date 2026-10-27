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
        Self { enabled: true, shortcuts_only: true, hold_ms: 1200.0 }
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

/// One timed word inside a caption (source ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
}

/// One caption line, usually produced by local speech recognition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
    #[serde(default)]
    pub words: Vec<CaptionWord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionStyle {
    /// Burn captions into the exported video.
    pub burn_in: bool,
    /// Text size as a fraction of the shorter output side.
    pub size: f64,
    pub color: [u8; 3],
    /// Colour of the word being spoken right now.
    pub highlight: [u8; 3],
    pub background_opacity: f64,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self { burn_in: true, size: 0.042, color: [255, 255, 255], highlight: [250, 204, 21], background_opacity: 0.72 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Run RNNoise (nnnoiseless) on the audio track during export.
    pub denoise: bool,
    /// Mute the original audio (for example when a voiceover replaces it).
    pub mute: bool,
    /// Bring the voice to -16 LUFS (two pass loudnorm plus a limiter). On by
    /// default because laptop microphones record very quietly.
    pub normalize: bool,
    /// Extra gain in dB applied before normalising (0 = none).
    pub gain_db: f64,
    /// Final volume, 0.0 to 1.5.
    pub volume: f64,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
    /// Delay the audio by this many ms (negative = earlier). Fine tuning for a
    /// microphone that lags the picture, such as a Bluetooth headset.
    pub offset_ms: f64,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { denoise: false, mute: false, normalize: true, gain_db: 0.0, volume: 1.0, fade_in_ms: 0.0, fade_out_ms: 0.0, offset_ms: 0.0 }
    }
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
    /// VP9 + Opus, for the web.
    Webm,
}

impl Format {
    pub fn from_path(p: &Path) -> Option<Self> {
        match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "mp4" | "m4v" | "mov" => Some(Format::Mp4),
            "gif" => Some(Format::Gif),
            "webm" => Some(Format::Webm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub aspect: Aspect,
    /// Short side in pixels. 2160 = 4K.
    pub quality: u32,
    pub fps: u32,
    pub format: Format,
    /// Cross-fade length at every cut, ms. 0 = hard cuts.
    #[serde(default = "default_crossfade")]
    pub crossfade_ms: f64,
}

fn default_crossfade() -> f64 {
    180.0
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { aspect: Aspect::Landscape, quality: 1080, fps: 30, format: Format::Mp4, crossfade_ms: default_crossfade() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebcamLayout {
    /// Floating over a corner of the screen.
    #[default]
    Bubble,
    /// A column beside the screen, nothing covered.
    Side,
}

/// Picture-in-picture webcam recorded alongside the screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Webcam {
    pub path: PathBuf,
    pub visible: bool,
    pub corner: Corner,
    /// Diameter as a fraction of the shorter output side.
    pub size: f64,
    /// Circle (true) or rounded square.
    pub circle: bool,
    /// The camera started this many ms after the screen recording.
    #[serde(default)]
    pub offset_ms: f64,
    #[serde(default)]
    pub layout: WebcamLayout,
    /// Flip left to right, like a mirror (what video calls show).
    #[serde(default)]
    pub mirror: bool,
}

impl Webcam {
    pub fn new(path: PathBuf) -> Self {
        Self { path, visible: true, corner: Corner::BottomRight, size: 0.24, circle: true, offset_ms: 0.0, layout: WebcamLayout::Bubble, mirror: false }
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
    #[serde(default)]
    pub captions: Vec<Caption>,
    #[serde(default)]
    pub caption_style: CaptionStyle,
    #[serde(default)]
    pub audio: AudioSettings,
    #[serde(default)]
    pub webcam: Option<Webcam>,
}

/// Zoom settings stored in a project. Old project files only had
/// `transition_ms`; every field has a default so they still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoomConfigSer {
    /// Legacy single transition length. Kept so old files round trip.
    pub transition_ms: f64,
    pub ease_in_ms: f64,
    pub ease_out_ms: f64,
    /// Zoom level used when auto zoom is generated.
    pub level: f64,
}

impl Default for ZoomConfigSer {
    fn default() -> Self {
        let c = ZoomConfig::default();
        Self { transition_ms: c.ease_in_ms, ease_in_ms: c.ease_in_ms, ease_out_ms: c.ease_out_ms, level: c.click_scale }
    }
}

impl ZoomConfigSer {
    /// The generator config that matches these settings.
    pub fn to_config(&self) -> ZoomConfig {
        ZoomConfig::default().with_level(self.level).with_speed(self.ease_in_ms, self.ease_out_ms)
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
            captions: Vec::new(),
            caption_style: CaptionStyle::default(),
            audio: AudioSettings::default(),
            webcam: None,
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let mut p: Project = serde_json::from_str(&s)?;
        // Relative media paths are resolved against the project file, so a
        // project works no matter which folder it is opened from.
        let base = path.parent().map(|d| if d.as_os_str().is_empty() { Path::new(".") } else { d }).unwrap_or(Path::new("."));
        let fix = |f: &mut PathBuf| {
            if f.is_relative() && !f.exists() && base.join(&*f).exists() {
                *f = base.join(&*f);
            }
        };
        fix(&mut p.source);
        if let Some(e) = p.events.as_mut() {
            fix(e);
        }
        if let Some(w) = p.webcam.as_mut() {
            fix(&mut w.path);
        }
        Ok(p)
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

    /// Map source time to output time. None if the moment was cut out.
    pub fn output_time(&self, src_ms: f64) -> Option<f64> {
        let mut acc = 0.0;
        for c in &self.clips {
            if src_ms >= c.src_start_ms && src_ms < c.src_end_ms {
                return Some(acc + (src_ms - c.src_start_ms) / c.speed);
            }
            acc += c.out_len();
        }
        None
    }

    /// Remove the given source ranges from the clip list (used by auto cut).
    pub fn cut_ranges(&mut self, ranges: &[(f64, f64)]) {
        let mut clips = self.clips.clone();
        for &(a, b) in ranges {
            let mut next = Vec::new();
            for c in clips {
                if b <= c.src_start_ms || a >= c.src_end_ms {
                    next.push(c);
                    continue;
                }
                if a > c.src_start_ms {
                    next.push(Clip { src_start_ms: c.src_start_ms, src_end_ms: a, speed: c.speed });
                }
                if b < c.src_end_ms {
                    next.push(Clip { src_start_ms: b, src_end_ms: c.src_end_ms, speed: c.speed });
                }
            }
            clips = next;
        }
        clips.retain(|c| c.src_end_ms - c.src_start_ms >= 1.0);
        if !clips.is_empty() {
            self.clips = clips;
        }
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
    fn cut_ranges_and_output_time() {
        let mut p = Project::new("a.mp4".into(), None, 100, 100, 10_000.0);
        p.cut_ranges(&[(1000.0, 2000.0), (5000.0, 6000.0)]);
        assert_eq!(p.clips.len(), 3);
        assert_eq!(p.output_duration_ms(), 8000.0);
        assert_eq!(p.output_time(1500.0), None);
        assert_eq!(p.output_time(2500.0), Some(1500.0));
        assert_eq!(p.output_time(6000.0), Some(4000.0));
        // Old project files without the new fields still load.
        let mut v: serde_json::Value = serde_json::to_value(&p).unwrap();
        v.as_object_mut().unwrap().remove("captions");
        v.as_object_mut().unwrap().remove("audio");
        let back: Project = serde_json::from_value(v).unwrap();
        assert!(back.captions.is_empty());
    }

    #[test]
    fn aspect_sizes() {
        assert_eq!(Aspect::Landscape.size(2160), (3840, 2160));
        assert_eq!(Aspect::Portrait.size(1080), (1080, 1920));
        assert_eq!(Aspect::Square.size(720), (720, 720));
    }
}
