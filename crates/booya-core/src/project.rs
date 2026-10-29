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
    /// A soft four colour wallpaper: top left, top right, bottom left, bottom right.
    Mesh { colors: [[u8; 3]; 4] },
}

/// How a background photo fills the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BgFit {
    /// Cover the frame, cropping the edges.
    #[default]
    Fill,
    /// Show the whole photo, with black bars where it does not match.
    Fit,
    /// Show the whole photo over a blurred copy of itself.
    Blur,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub background: Background,
    /// How a background photo fills the frame.
    #[serde(default)]
    pub bg_fit: BgFit,
    /// Darken the background photo, 0 (none) to 0.8.
    #[serde(default)]
    pub bg_dim: f64,
    /// Padding as a fraction of the shorter output side (0.0 to 0.4).
    pub padding: f64,
    /// Corner radius as a fraction of the shorter output side.
    pub corner_radius: f64,
    /// Shadow blur size as a fraction of the shorter output side. 0 = off.
    pub shadow: f64,
    /// Shadow opacity 0..1.
    pub shadow_opacity: f64,
    /// Frosted glass frame around the recording and glass overlays.
    #[serde(default)]
    pub glass: Glass,
    /// A simple vector device drawn round the recording. Added in 0.7.0.
    #[serde(default)]
    pub frame: DeviceFrame,
    /// Blur the picture while the camera zooms or pans (a camera shutter
    /// look). Costs a little export time during those moments. Added in 0.7.0.
    #[serde(default)]
    pub motion_blur: bool,
}

/// A device drawn round the recording. Plain shapes and neutral colours, no
/// logos and no one maker's artwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceFrame {
    #[default]
    None,
    /// A laptop: lid, thin bezel, camera dot and a base.
    Laptop,
    /// A browser window: a title bar with three dots and an address field.
    Browser,
    /// A phone: rounded body, speaker slot and camera dot.
    Phone,
}

impl DeviceFrame {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "laptop" | "macbook" => Self::Laptop,
            "browser" | "window" => Self::Browser,
            "phone" | "iphone" => Self::Phone,
            _ => Self::None,
        }
    }
}

/// A frosted, liquid glass look: the recording sits on a blurred, tinted,
/// rounded panel with a light edge, and captions, callouts and the keystroke
/// overlay become small frosted pills. Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Glass {
    pub enabled: bool,
    /// Frost blur as a fraction of the shorter output side (0 to 0.06).
    pub blur: f64,
    /// Tint colour laid over the frost.
    pub tint: [u8; 3],
    /// How strongly the tint shows, 0 to 0.6.
    pub tint_opacity: f64,
    /// Strength of the bright rim light on the edge, 0 to 1.
    pub highlight: f64,
    /// Panel corner radius as a fraction of the shorter output side.
    pub radius: f64,
    /// Space between the panel edge and the recording, fraction of the shorter side.
    pub inset: f64,
    /// Also draw captions, callouts and the keystroke overlay as frosted glass.
    pub overlays: bool,
}

impl Default for Glass {
    fn default() -> Self {
        Self { enabled: false, blur: 0.018, tint: [255, 255, 255], tint_opacity: 0.16, highlight: 0.7, radius: 0.04, inset: 0.022, overlays: true }
    }
}

impl Glass {
    /// Glass limits so a hand edited project cannot produce nonsense.
    pub fn clamped(&self) -> Glass {
        Glass {
            enabled: self.enabled,
            blur: self.blur.clamp(0.0, 0.06),
            tint: self.tint,
            tint_opacity: self.tint_opacity.clamp(0.0, 0.6),
            highlight: self.highlight.clamp(0.0, 1.0),
            radius: self.radius.clamp(0.0, 0.12),
            inset: self.inset.clamp(0.0, 0.08),
            overlays: self.overlays,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: Background::Gradient { from: [99, 102, 241], to: [236, 72, 153], angle_deg: 35.0 },
            bg_fit: BgFit::Fill,
            bg_dim: 0.0,
            padding: 0.07,
            corner_radius: 0.018,
            shadow: 0.035,
            shadow_opacity: 0.45,
            glass: Glass::default(),
            frame: DeviceFrame::None,
            motion_blur: false,
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
    /// Show Mac symbols (⌘ ⌥ ⇧ ⌃) instead of words. Applies to recordings
    /// made on a Mac only; a new project starts with symbols on a Mac.
    #[serde(default = "default_symbols")]
    pub symbols: bool,
}

fn default_symbols() -> bool {
    cfg!(target_os = "macos")
}

impl Default for KeyOverlay {
    fn default() -> Self {
        Self { enabled: true, shortcuts_only: true, hold_ms: 1200.0, symbols: default_symbols() }
    }
}

/// One piece of the source kept in the output, played at `speed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub src_start_ms: f64,
    pub src_end_ms: f64,
    pub speed: f64,
    /// No sound from the recording in this clip (the music still plays).
    /// Added in 0.7.0.
    #[serde(default)]
    pub mute: bool,
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
    /// Set by the exporter, never saved: the source's audio track starts at a
    /// different time than its video (an older recording, or a microphone that
    /// started late), so the audio is re-based to zero and `offset_ms` carries
    /// the difference.
    #[serde(skip)]
    pub rebase_start: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { denoise: false, mute: false, normalize: true, gain_db: 0.0, volume: 1.0, fade_in_ms: 0.0, fade_out_ms: 0.0, offset_ms: 0.0, rebase_start: false }
    }
}

/// A music track under the whole video. Mixed on export, and in the preview
/// by the native player. Added in 0.7.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Music {
    /// The audio file (mp3, m4a, wav, ogg, flac).
    pub path: PathBuf,
    /// 0 to 1.
    pub volume: f64,
    /// Lower the music while someone speaks.
    pub duck: bool,
    /// How far the music drops under the voice, in dB (3 to 24).
    pub duck_db: f64,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
    /// Where in the video the music starts.
    pub start_ms: f64,
    /// Repeat the file when it is shorter than the video.
    pub looped: bool,
}

impl Default for Music {
    fn default() -> Self {
        Self { path: PathBuf::new(), volume: 0.25, duck: true, duck_db: 12.0, fade_in_ms: 1000.0, fade_out_ms: 1500.0, start_ms: 0.0, looped: true }
    }
}

impl Music {
    pub fn new(path: PathBuf) -> Self {
        Self { path, ..Self::default() }
    }
    pub fn clamped(&self) -> Self {
        Self { volume: self.volume.clamp(0.0, 1.0), duck_db: self.duck_db.clamp(3.0, 24.0), fade_in_ms: self.fade_in_ms.clamp(0.0, 10_000.0), fade_out_ms: self.fade_out_ms.clamp(0.0, 10_000.0), start_ms: self.start_ms.max(0.0), ..self.clone() }
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
    /// Middle of the top edge. Edge pins were added in 0.7.0.
    TopCenter,
    BottomCenter,
    /// Middle of the left edge.
    LeftCenter,
    RightCenter,
}

impl Corner {
    /// On the left half (a corner or the middle of the left edge).
    pub fn is_left(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::BottomLeft | Corner::LeftCenter)
    }
    /// On the top (a corner or the middle of the top edge).
    pub fn is_top(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::TopRight | Corner::TopCenter)
    }
    /// Where a box `w` by `h` sits in an `ow` by `oh` frame with margin `m`.
    pub fn place(self, ow: f64, oh: f64, w: f64, h: f64, m: f64) -> (f64, f64) {
        let cx = (ow - w) / 2.0;
        let cy = (oh - h) / 2.0;
        match self {
            Corner::TopLeft => (m, m),
            Corner::TopRight => (ow - w - m, m),
            Corner::BottomLeft => (m, oh - h - m),
            Corner::BottomRight => (ow - w - m, oh - h - m),
            Corner::TopCenter => (cx, m),
            Corner::BottomCenter => (cx, oh - h - m),
            Corner::LeftCenter => (m, cy),
            Corner::RightCenter => (ow - w - m, cy),
        }
    }
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

/// A line of text in a corner of every frame, such as a product name or URL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watermark {
    pub text: String,
    pub corner: Corner,
    /// 0.1 to 1.
    pub opacity: f64,
    /// Text height as a fraction of the shorter output side.
    pub size: f64,
}

impl Default for Watermark {
    fn default() -> Self {
        Self { text: String::new(), corner: Corner::BottomRight, opacity: 0.7, size: 0.028 }
    }
}

/// A short text note shown over the video for a few seconds, like "Click here".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Callout {
    pub text: String,
    /// Source time range.
    pub start_ms: f64,
    pub end_ms: f64,
    /// Centre of the note as a fraction of the output (0 to 1).
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    /// A line from (x1, y1) with the head at (x2, y2).
    Arrow,
    /// An outlined rounded box.
    Box,
    /// An outlined ellipse.
    Ellipse,
    /// A translucent filled box, like a highlighter.
    Highlight,
}

/// A drawn mark over the video for a stretch of time: arrow, box, ellipse or highlight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub kind: ShapeKind,
    /// Source time range.
    pub start_ms: f64,
    pub end_ms: f64,
    /// Corners as fractions of the output (0 to 1). For an arrow, (x1, y1) is the tail and (x2, y2) the head.
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    /// Colour as `#rrggbb`.
    #[serde(default = "default_shape_color")]
    pub color: String,
    /// Line width as a fraction of the shorter output side.
    #[serde(default = "default_shape_width")]
    pub width: f64,
}

fn default_shape_color() -> String {
    "#fbbf24".into()
}
fn default_shape_width() -> f64 {
    0.006
}

impl Shape {
    /// The colour as RGB, falling back to amber for anything unreadable.
    pub fn rgb(&self) -> [f32; 3] {
        let h = self.color.trim_start_matches('#');
        if h.len() == 6 {
            if let Ok(v) = u32::from_str_radix(h, 16) {
                return [((v >> 16) & 255) as f32, ((v >> 8) & 255) as f32, (v & 255) as f32];
            }
        }
        [251.0, 191.0, 36.0]
    }
}

/// A title card shown before or after the recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default = "Card::default_ms")]
    pub duration_ms: f64,
}

impl Card {
    fn default_ms() -> f64 {
        2500.0
    }
    pub fn is_empty(&self) -> bool {
        self.title.trim().is_empty() && self.subtitle.trim().is_empty()
    }
    /// Length actually used: 1 to 6 seconds.
    pub fn length_ms(&self) -> f64 {
        self.duration_ms.clamp(1000.0, 6000.0)
    }
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

/// What the picture is made of at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SceneKind {
    /// The recording alone.
    Screen,
    /// The recording with the camera floating over a corner.
    Bubble,
    /// The recording and the camera next to each other (above and below in
    /// a tall picture).
    Split,
    /// The camera fills the picture.
    Camera,
}

impl SceneKind {
    pub const ALL: [SceneKind; 4] = [SceneKind::Screen, SceneKind::Bubble, SceneKind::Split, SceneKind::Camera];

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "screen" => SceneKind::Screen,
            "bubble" => SceneKind::Bubble,
            "split" | "side" | "side-by-side" => SceneKind::Split,
            "camera" | "cam" | "fullscreen" => SceneKind::Camera,
            _ => bail!("a layout is screen, bubble, split or camera"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            SceneKind::Screen => "screen",
            SceneKind::Bubble => "bubble",
            SceneKind::Split => "split",
            SceneKind::Camera => "camera",
        }
    }

    pub fn needs_camera(self) -> bool {
        self != SceneKind::Screen
    }
}

/// A stretch of the recording (source time) shown in one layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneSegment {
    pub start_ms: f64,
    pub end_ms: f64,
    pub kind: SceneKind,
}

/// Which layout is on screen when, and how the picture changes between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layouts {
    /// The layout outside every segment. When not set it follows the camera
    /// settings (bubble or side by side when there is a camera, else screen).
    #[serde(default)]
    pub base: Option<SceneKind>,
    #[serde(default)]
    pub segments: Vec<SceneSegment>,
    /// Length of the cross-fade between two layouts, ms.
    #[serde(default = "default_scene_transition")]
    pub transition_ms: f64,
    /// In a tall or square picture, show the part of the screen around the
    /// cursor instead of shrinking the whole screen to fit.
    #[serde(default = "yes")]
    pub follow_cursor: bool,
}

fn default_scene_transition() -> f64 {
    450.0
}

fn yes() -> bool {
    true
}

impl Default for Layouts {
    fn default() -> Self {
        Self { base: None, segments: Vec::new(), transition_ms: default_scene_transition(), follow_cursor: true }
    }
}

/// One layout shown from `start_ms` to `end_ms` (source time).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneRun {
    pub start_ms: f64,
    pub end_ms: f64,
    pub kind: SceneKind,
}

/// The layout at one moment, and the one it is fading into or out of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneMix {
    pub kind: SceneKind,
    /// The other layout and how much of it shows (0 to 1).
    pub other: Option<(SceneKind, f64)>,
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
    /// Background music. Added in 0.7.0.
    #[serde(default)]
    pub music: Option<Music>,
    #[serde(default)]
    pub webcam: Option<Webcam>,
    #[serde(default)]
    pub layouts: Layouts,
    #[serde(default)]
    pub watermark: Option<Watermark>,
    #[serde(default)]
    pub callouts: Vec<Callout>,
    #[serde(default)]
    pub shapes: Vec<Shape>,
    #[serde(default)]
    pub intro: Option<Card>,
    #[serde(default)]
    pub outro: Option<Card>,
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
    /// 0 makes the camera follow every cursor move; 1 holds still until the
    /// cursor nears the edge of the view. Added in 0.3.0.
    pub steadiness: f64,
    /// How long a zoom holds after the last click or key of a group. Added in 0.5.0.
    pub hold_ms: f64,
    /// Clicks closer than this share one zoom instead of making a new one. Added in 0.5.0.
    pub merge_gap_ms: f64,
    /// "subtle", "normal" or "lively": how selective auto zoom is. Added in 0.5.2.
    pub intensity: crate::zoom::Intensity,
    /// Clicks in one small area it takes to zoom. Normal is 3. Added in 0.7.0.
    pub min_clicks: usize,
    /// The clicks have to come within this long. Added in 0.7.0.
    pub click_window_ms: f64,
}

impl Default for ZoomConfigSer {
    fn default() -> Self {
        let c = ZoomConfig::default();
        Self { transition_ms: c.ease_in_ms, ease_in_ms: c.ease_in_ms, ease_out_ms: c.ease_out_ms, level: c.click_scale, steadiness: 0.8, hold_ms: c.hold_ms, merge_gap_ms: c.merge_gap_ms, intensity: c.intensity, min_clicks: c.min_clicks, click_window_ms: c.cluster_window_ms }
    }
}

impl ZoomConfigSer {
    /// The generator config that matches these settings.
    pub fn to_config(&self) -> ZoomConfig {
        ZoomConfig::default().with_intensity(self.intensity).with_level(self.level).with_speed(self.ease_in_ms, self.ease_out_ms).with_timing(self.hold_ms, self.merge_gap_ms).with_clicks(self.min_clicks, self.click_window_ms)
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
            clips: vec![Clip { src_start_ms: 0.0, src_end_ms: duration_ms, speed: 1.0, mute: false }],
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
            music: None,
            webcam: None,
            layouts: Layouts::default(),
            watermark: None,
            callouts: Vec::new(),
            shapes: Vec::new(),
            intro: None,
            outro: None,
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

    /// True when the camera can be shown.
    pub fn has_camera(&self) -> bool {
        self.webcam.as_ref().is_some_and(|w| w.visible)
    }

    /// The layout outside every segment.
    pub fn base_scene(&self) -> SceneKind {
        if !self.has_camera() {
            return SceneKind::Screen;
        }
        match self.layouts.base {
            Some(k) => k,
            None => match self.webcam.as_ref().map(|w| w.layout) {
                Some(WebcamLayout::Side) => SceneKind::Split,
                _ => SceneKind::Bubble,
            },
        }
    }

    /// Every moment of the recording in exactly one layout, in order: the
    /// segments (clipped, the later one winning an overlap), with the base
    /// layout between them. Without a camera everything is `Screen`.
    pub fn scene_runs(&self) -> Vec<SceneRun> {
        let dur = self.duration_ms.max(1.0);
        let base = self.base_scene();
        if !self.has_camera() {
            return vec![SceneRun { start_ms: 0.0, end_ms: dur, kind: SceneKind::Screen }];
        }
        let mut segs: Vec<SceneSegment> = self.layouts.segments.iter().filter(|s| s.end_ms > s.start_ms && s.end_ms > 0.0 && s.start_ms < dur).cloned().collect();
        segs.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        let mut runs: Vec<SceneRun> = Vec::new();
        let push = |runs: &mut Vec<SceneRun>, a: f64, b: f64, kind: SceneKind| {
            if b - a < 1.0 {
                return;
            }
            match runs.last_mut() {
                Some(l) if l.kind == kind && (a - l.end_ms).abs() < 1.0 => l.end_ms = b,
                _ => runs.push(SceneRun { start_ms: a, end_ms: b, kind }),
            }
        };
        let mut at = 0.0;
        for s in segs {
            let (a, b) = (s.start_ms.max(at), s.end_ms.min(dur));
            if b <= a {
                continue;
            }
            push(&mut runs, at, a, base);
            push(&mut runs, a, b, s.kind);
            at = b;
        }
        push(&mut runs, at, dur, base);
        if runs.is_empty() {
            runs.push(SceneRun { start_ms: 0.0, end_ms: dur, kind: base });
        }
        runs
    }

    /// Every layout the picture ever uses.
    pub fn scene_kinds_used(&self) -> Vec<SceneKind> {
        let mut v: Vec<SceneKind> = Vec::new();
        for r in self.scene_runs() {
            if !v.contains(&r.kind) {
                v.push(r.kind);
            }
        }
        v
    }

    /// The layout at source time `t`, cross-fading around each change.
    pub fn scene_at(&self, t: f64) -> SceneMix {
        let runs = self.scene_runs();
        let i = runs.iter().position(|r| t < r.end_ms).unwrap_or(runs.len() - 1);
        let here = runs[i];
        let tr = self.layouts.transition_ms.clamp(0.0, 3000.0);
        if tr < 1.0 {
            return SceneMix { kind: here.kind, other: None };
        }
        let smooth = |x: f64| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        // The change at the end of this run: it starts fading before the edge.
        if let Some(next) = runs.get(i + 1) {
            let half = (tr / 2.0).min((here.end_ms - here.start_ms) / 2.0).min((next.end_ms - next.start_ms) / 2.0);
            if half >= 1.0 && t >= here.end_ms - half {
                let k = smooth((t - (here.end_ms - half)) / (2.0 * half));
                return SceneMix { kind: here.kind, other: Some((next.kind, k)) };
            }
        }
        // The change at the start of this run: it is still finishing.
        if i > 0 {
            let prev = runs[i - 1];
            let half = (tr / 2.0).min((here.end_ms - here.start_ms) / 2.0).min((prev.end_ms - prev.start_ms) / 2.0);
            if half >= 1.0 && t < here.start_ms + half {
                let k = smooth((t - (here.start_ms - half)) / (2.0 * half));
                return SceneMix { kind: prev.kind, other: Some((here.kind, k)) };
            }
        }
        SceneMix { kind: here.kind, other: None }
    }

    /// Lengths of the intro and outro cards in ms (0 when absent or empty).
    pub fn card_lengths_ms(&self) -> (f64, f64) {
        let len = |c: &Option<Card>| c.as_ref().filter(|c| !c.is_empty()).map(Card::length_ms).unwrap_or(0.0);
        (len(&self.intro), len(&self.outro))
    }

    /// Where source time `s` lands on the exported timeline. A time inside a
    /// removed stretch lands on the seam where the cut was made.
    pub fn src_to_out(&self, s: f64) -> f64 {
        let mut acc = 0.0;
        for c in &self.clips {
            if s < c.src_start_ms {
                return acc;
            }
            if s <= c.src_end_ms {
                return acc + (s - c.src_start_ms) / c.speed.max(0.01);
            }
            acc += c.out_len();
        }
        acc
    }

    /// Source time shown at output time `o` (the end of the last clip past the end).
    pub fn out_to_src(&self, o: f64) -> f64 {
        let mut acc = 0.0;
        for c in &self.clips {
            let len = c.out_len();
            if o <= acc + len {
                return c.src_start_ms + (o - acc).max(0.0) * c.speed.max(0.01);
            }
            acc += len;
        }
        self.clips.last().map(|c| c.src_end_ms).unwrap_or(0.0)
    }

    /// The zoom segments on the exported timeline. Zoom is easing in and out
    /// of what the viewer sees, so a cut in the middle of a zoom must not make
    /// the camera jump: the part that is cut away simply is not there.
    pub fn zoom_on_output(&self) -> Vec<ZoomSegment> {
        self.zoom
            .iter()
            .filter_map(|z| {
                let (a, b) = (self.src_to_out(z.start_ms), self.src_to_out(z.end_ms));
                (b - a >= 300.0).then(|| ZoomSegment { start_ms: a, end_ms: b, ..z.clone() })
            })
            .collect()
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
                    next.push(Clip { src_start_ms: c.src_start_ms, src_end_ms: a, speed: c.speed, mute: c.mute });
                }
                if b < c.src_end_ms {
                    next.push(Clip { src_start_ms: b, src_end_ms: c.src_end_ms, speed: c.speed, mute: c.mute });
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
                out.push(Clip { src_start_ms: s, src_end_ms: e, speed: c.speed, mute: c.mute });
            }
        }
        self.clips = out;
    }

    /// Split the clip containing source time `at` into two.
    pub fn split(&mut self, at: f64) -> bool {
        if let Some(i) = self.clips.iter().position(|c| at > c.src_start_ms && at < c.src_end_ms) {
            let c = self.clips[i].clone();
            self.clips[i].src_end_ms = at;
            self.clips.insert(i + 1, Clip { src_start_ms: at, src_end_ms: c.src_end_ms, speed: c.speed, mute: c.mute });
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
    fn edge_pins_sit_in_the_middle_of_their_edge() {
        let (ow, oh, d, m) = (1920.0, 1080.0, 200.0, 40.0);
        assert_eq!(Corner::TopCenter.place(ow, oh, d, d, m), (860.0, 40.0));
        assert_eq!(Corner::BottomCenter.place(ow, oh, d, d, m), (860.0, 840.0));
        assert_eq!(Corner::LeftCenter.place(ow, oh, d, d, m), (40.0, 440.0));
        assert_eq!(Corner::RightCenter.place(ow, oh, d, d, m), (1680.0, 440.0));
        assert_eq!(Corner::BottomRight.place(ow, oh, d, d, m), (1680.0, 840.0));
        assert_eq!(serde_json::to_string(&Corner::LeftCenter).unwrap(), "\"left-center\"");
        assert!(Corner::LeftCenter.is_left() && !Corner::TopCenter.is_left() && Corner::TopCenter.is_top());
    }

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
    fn captions_and_their_words_survive_save_and_open() {
        let dir = std::env::temp_dir().join(format!("dl-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Project::new("a.mp4".into(), None, 100, 100, 10_000.0);
        p.captions = crate::captions::words_to_captions(
            &[CaptionWord { start_ms: 1000.0, end_ms: 1300.0, text: "Hello".into() }, CaptionWord { start_ms: 1400.0, end_ms: 1800.0, text: "world".into() }],
            42,
            700.0,
        );
        p.caption_style.burn_in = true;
        let f = dir.join("p.booya");
        p.save(&f).unwrap();
        let back = Project::load(&f).unwrap();
        assert_eq!(back.captions.len(), 1);
        assert_eq!(back.captions[0].words[1].text, "world");
        assert_eq!(back.captions[0].words[1].start_ms, 1400.0);
        assert!(back.caption_style.burn_in);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn aspect_sizes() {
        assert_eq!(Aspect::Landscape.size(2160), (3840, 2160));
        assert_eq!(Aspect::Portrait.size(1080), (1080, 1920));
        assert_eq!(Aspect::Square.size(720), (720, 720));
    }
}
