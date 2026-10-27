//! The compositor. One function, `Renderer::render`, turns a source frame plus
//! a source timestamp into a finished output frame. The editor preview and the
//! exporter both call it, so what you see is exactly what you export.

use crate::events::{CursorTrack, EventLog};
use crate::text;
use crate::project::{Background, BlurBox, Project};
use crate::zoom::{camera_at, Camera, CameraTiming};
use rayon::prelude::*;

/// RGBA8 frame.
#[derive(Clone)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, data: vec![0; (w * h * 4) as usize] }
    }

    #[inline]
    fn px(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        let i = (y * self.w as usize + x) * 4;
        let d = &self.data[i..i + 4];
        [d[0] as f32, d[1] as f32, d[2] as f32, d[3] as f32]
    }

    /// Bilinear sample at continuous pixel coordinates.
    #[inline]
    fn sample(&self, x: f64, y: f64) -> [f32; 3] {
        let x = x - 0.5;
        let y = y - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = (x - x0) as f32;
        let fy = (y - y0) as f32;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.px(x0, y0);
        let b = self.px(x0 + 1, y0);
        let c = self.px(x0, y0 + 1);
        let d = self.px(x0 + 1, y0 + 1);
        let mut out = [0f32; 3];
        for k in 0..3 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bot = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bot - top) * fy;
        }
        out
    }

    pub fn encode_png(&self) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.w, self.h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            // Previews are thrown away after display, so favour speed over size.
            enc.set_compression(png::Compression::Fast);
            let mut wr = enc.write_header()?;
            wr.write_image_data(&self.data)?;
        }
        Ok(out)
    }

    /// Alpha blend one pixel (used by the text renderer).
    #[inline]
    /// Blend `other` over this frame with weight `a` (same size only).
    pub fn mix_from(&mut self, other: &Frame, a: f32) {
        if other.w != self.w || other.h != self.h || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        self.data.par_chunks_mut(4096).zip(other.data.par_chunks(4096)).for_each(|(d, o)| {
            for (x, y) in d.iter_mut().zip(o) {
                *x = (*x as f32 + (*y as f32 - *x as f32) * a).round() as u8;
            }
        });
    }

    pub fn blend_pub(&mut self, x: i64, y: i64, rgb: [f32; 3], a: f32) {
        self.blend(x, y, rgb, a)
    }

    #[inline]
    fn blend(&mut self, x: i64, y: i64, rgb: [f32; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 || a <= 0.0 {
            return;
        }
        let i = ((y as usize) * self.w as usize + x as usize) * 4;
        let a = a.min(1.0);
        for k in 0..3 {
            let d = self.data[i + k] as f32;
            self.data[i + k] = (d + (rgb[k] - d) * a).round() as u8;
        }
        self.data[i + 3] = 255;
    }
}

/// Signed distance to a rounded rectangle centred at (cx, cy) with half size
/// (hw, hh) and radius r. Negative inside.
#[inline]
fn sd_round_rect(px: f64, py: f64, cx: f64, cy: f64, hw: f64, hh: f64, r: f64) -> f64 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// In-place box blur of a region (three passes approximates a Gaussian).
pub fn blur_region(f: &mut Frame, b: &BlurBox) {
    let x0 = b.x.max(0.0) as usize;
    let y0 = b.y.max(0.0) as usize;
    let x1 = ((b.x + b.w) as usize).min(f.w as usize);
    let y1 = ((b.y + b.h) as usize).min(f.h as usize);
    if x1 <= x0 + 1 || y1 <= y0 + 1 {
        return;
    }
    let (rw, rh) = (x1 - x0, y1 - y0);
    let mut buf: Vec<[f32; 3]> = Vec::with_capacity(rw * rh);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * f.w as usize + x) * 4;
            buf.push([f.data[i] as f32, f.data[i + 1] as f32, f.data[i + 2] as f32]);
        }
    }
    let r = (b.radius.max(1) as usize / 2).max(1);
    let mut tmp = buf.clone();
    // Running-sum box blur, three passes, O(1) per pixel regardless of radius.
    let pass = |src: &[[f32; 3]], dst: &mut [[f32; 3]], len: usize, lines: usize, idx: &dyn Fn(usize, usize) -> usize| {
        for l in 0..lines {
            let mut s = [0f32; 3];
            let mut n = 0f32;
            for i in 0..=r.min(len - 1) {
                let p = src[idx(l, i)];
                s = [s[0] + p[0], s[1] + p[1], s[2] + p[2]];
                n += 1.0;
            }
            for i in 0..len {
                dst[idx(l, i)] = [s[0] / n, s[1] / n, s[2] / n];
                if i + r + 1 < len {
                    let p = src[idx(l, i + r + 1)];
                    s = [s[0] + p[0], s[1] + p[1], s[2] + p[2]];
                    n += 1.0;
                }
                if i >= r {
                    let p = src[idx(l, i - r)];
                    s = [s[0] - p[0], s[1] - p[1], s[2] - p[2]];
                    n -= 1.0;
                }
            }
        }
    };
    for _ in 0..3 {
        pass(&buf, &mut tmp, rw, rh, &|l, i| l * rw + i);
        pass(&tmp, &mut buf, rh, rw, &|l, i| i * rw + l);
    }
    for y in 0..rh {
        for x in 0..rw {
            let i = ((y + y0) * f.w as usize + x + x0) * 4;
            let p = buf[y * rw + x];
            f.data[i] = p[0] as u8;
            f.data[i + 1] = p[1] as u8;
            f.data[i + 2] = p[2] as u8;
        }
    }
}

/// Label shown while the presenter types (characters are never logged).
pub const TYPING: &str = "Typing...";

/// One label shown by the keystroke overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyLabel {
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
}

fn is_modifier_combo(mods: &[String]) -> bool {
    mods.iter().any(|m| !m.eq_ignore_ascii_case("shift"))
}

/// Turn key events into overlay labels. Typing is grouped into one growing
/// label; shortcuts like Ctrl+S get their own label.
pub fn build_key_labels(log: &EventLog, hold_ms: f64, shortcuts_only: bool) -> Vec<KeyLabel> {
    let mut out: Vec<KeyLabel> = Vec::new();
    let mut typing_open = false;
    for (t, key, mods) in log.keys() {
        if is_modifier_combo(mods) {
            let mut parts: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
            parts.push(key.to_string());
            out.push(KeyLabel { start_ms: t, end_ms: t + hold_ms, text: parts.join(" + ") });
            typing_open = false;
            continue;
        }
        if shortcuts_only {
            continue;
        }
        // The log holds no characters (only that a printable key was
        // pressed), so typing shows as one "Typing..." label.
        let typing = key == crate::events::PRINTABLE || key.chars().count() == 1 || key.eq_ignore_ascii_case("space");
        if typing {
            if let Some(last) = out.last_mut() {
                if typing_open && t - (last.end_ms - hold_ms) < 1200.0 {
                    last.end_ms = t + hold_ms;
                    continue;
                }
            }
            out.push(KeyLabel { start_ms: t, end_ms: t + hold_ms, text: TYPING.into() });
            typing_open = true;
        } else {
            out.push(KeyLabel { start_ms: t, end_ms: t + hold_ms, text: key.to_string() });
            typing_open = false;
        }
    }
    // Earlier labels end when the next one starts.
    for i in 1..out.len() {
        let next = out[i].start_ms;
        if out[i - 1].end_ms > next {
            out[i - 1].end_ms = next;
        }
    }
    out
}

const ARROW: [(f64, f64); 7] = [(0.0, 0.0), (0.0, 17.0), (4.2, 13.2), (7.0, 19.5), (9.6, 18.4), (6.9, 12.2), (12.2, 12.2)];

fn point_in_poly(x: f64, y: f64, poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn fill_poly(f: &mut Frame, poly: &[(f64, f64)], rgb: [f32; 3], alpha: f32) {
    let minx = poly.iter().map(|p| p.0).fold(f64::MAX, f64::min).floor() as i64;
    let maxx = poly.iter().map(|p| p.0).fold(f64::MIN, f64::max).ceil() as i64;
    let miny = poly.iter().map(|p| p.1).fold(f64::MAX, f64::min).floor() as i64;
    let maxy = poly.iter().map(|p| p.1).fold(f64::MIN, f64::max).ceil() as i64;
    for y in miny..=maxy {
        for x in minx..=maxx {
            let mut cov = 0;
            for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                if point_in_poly(x as f64 + sx, y as f64 + sy, poly) {
                    cov += 1;
                }
            }
            if cov > 0 {
                f.blend(x, y, rgb, alpha * cov as f32 / 4.0);
            }
        }
    }
}

fn fill_round_rect(f: &mut Frame, x: f64, y: f64, w: f64, h: f64, r: f64, rgb: [f32; 3], alpha: f32) {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    for py in (y.floor() as i64)..=((y + h).ceil() as i64) {
        for px in (x.floor() as i64)..=((x + w).ceil() as i64) {
            let d = sd_round_rect(px as f64 + 0.5, py as f64 + 0.5, cx, cy, w / 2.0, h / 2.0, r);
            let cov = (0.5 - d).clamp(0.0, 1.0) as f32;
            f.blend(px, py, rgb, alpha * cov);
        }
    }
}

/// Layout of the recording on the canvas.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
}

pub struct Renderer {
    pub project: Project,
    pub out_w: u32,
    pub out_h: u32,
    track: Option<CursorTrack>,
    clicks: Vec<(f64, f64, f64)>,
    labels: Vec<KeyLabel>,
    background: Frame,
    pub layout: Layout,
}

fn lerp3(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    [0, 1, 2].map(|k| (a[k] as f64 + (b[k] as f64 - a[k] as f64) * t).round() as u8)
}

impl Renderer {
    /// The smallest source decode size that still gives at least one source
    /// pixel per output pixel at the deepest zoom. Decoding a Retina recording
    /// at full size for a 1080p export wastes most of the decode and sampling
    /// work, so the decoder scales it down first (with a better filter than
    /// the bilinear sampler would give when minifying).
    pub fn decode_size(&self) -> (u32, u32) {
        let p = &self.project;
        let (sw, sh) = (p.source_width.max(2), p.source_height.max(2));
        let max_zoom = p.zoom.iter().map(|z| z.scale).fold(1.0, f64::max);
        let need_w = (self.layout.w * max_zoom).ceil();
        if need_w >= sw as f64 {
            return (sw, sh);
        }
        let k = need_w / sw as f64;
        let even = |v: f64| ((v.ceil() as u32 + 1) & !1).max(2);
        (even(sw as f64 * k).min(sw), even(sh as f64 * k).min(sh))
    }

    /// `bg_image` is an RGBA image already scaled to the output size; only
    /// used with `Background::Image`.
    pub fn new(project: Project, log: Option<&EventLog>, out_w: u32, out_h: u32, bg_image: Option<Frame>) -> Self {
        let track = log.map(CursorTrack::new).filter(|t| !t.is_empty());
        let clicks = log.map(|l| l.clicks().collect()).unwrap_or_default();
        let labels = match log {
            Some(l) if project.keys.enabled => build_key_labels(l, project.keys.hold_ms, project.keys.shortcuts_only),
            _ => Vec::new(),
        };
        let background = Self::paint_background(&project.style.background, out_w, out_h, bg_image);
        let short = out_w.min(out_h) as f64;
        let pad = project.style.padding.clamp(0.0, 0.4) * short;
        let (aw, ah) = (out_w as f64 - 2.0 * pad, out_h as f64 - 2.0 * pad);
        let src_aspect = project.source_width as f64 / project.source_height as f64;
        let (w, h) = if aw / ah > src_aspect { (ah * src_aspect, ah) } else { (aw, aw / src_aspect) };
        let layout = Layout {
            x: (out_w as f64 - w) / 2.0,
            y: (out_h as f64 - h) / 2.0,
            w,
            h,
            radius: (project.style.corner_radius * short).min(w.min(h) / 2.0),
        };
        let mut background = background;
        Self::bake_shadow(&mut background, &project, layout, out_w, out_h);
        Self { project, out_w, out_h, track, clicks, labels, background, layout }
    }

    /// The drop shadow never changes between frames, so it is painted into the
    /// background once instead of being recomputed for every pixel of every frame.
    fn bake_shadow(bg: &mut Frame, project: &Project, l: Layout, out_w: u32, out_h: u32) {
        let short = out_w.min(out_h) as f64;
        let shadow_px = project.style.shadow * short;
        if shadow_px <= 0.0 {
            return;
        }
        let shadow_off = shadow_px * 0.35;
        let shadow_op = project.style.shadow_opacity as f32;
        let (cx, cy, hw, hh) = (l.x + l.w / 2.0, l.y + l.h / 2.0, l.w / 2.0, l.h / 2.0);
        let w = out_w as usize;
        bg.data.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            let fy = y as f64 + 0.5;
            for x in 0..w {
                let fx = x as f64 + 0.5;
                let d = sd_round_rect(fx, fy, cx, cy, hw, hh, l.radius);
                if d > -1.0 {
                    let ds = sd_round_rect(fx, fy - shadow_off, cx, cy, hw, hh, l.radius);
                    let k = (1.0 - (ds / shadow_px).clamp(0.0, 1.0)) as f32;
                    let a = shadow_op * k * k;
                    for c in row[x * 4..x * 4 + 3].iter_mut() {
                        *c = (*c as f32 * (1.0 - a)) as u8;
                    }
                }
            }
        });
    }

    fn paint_background(bg: &Background, w: u32, h: u32, image: Option<Frame>) -> Frame {
        if let (Background::Image { .. }, Some(img)) = (bg, image) {
            if img.w == w && img.h == h {
                return img;
            }
        }
        let mut f = Frame::new(w, h);
        let (from, to, angle) = match bg {
            Background::Solid { color } => (*color, *color, 0.0),
            Background::Gradient { from, to, angle_deg } => (*from, *to, *angle_deg),
            Background::Image { .. } => ([24, 24, 27], [24, 24, 27], 0.0),
        };
        let (dx, dy) = (angle.to_radians().cos(), angle.to_radians().sin());
        // Project corners to find the gradient range.
        let proj = |x: f64, y: f64| x * dx + y * dy;
        let cs = [proj(0.0, 0.0), proj(w as f64, 0.0), proj(0.0, h as f64), proj(w as f64, h as f64)];
        let (lo, hi) = (cs.iter().cloned().fold(f64::MAX, f64::min), cs.iter().cloned().fold(f64::MIN, f64::max));
        f.data.par_chunks_mut((w * 4) as usize).enumerate().for_each(|(y, row)| {
            for x in 0..w as usize {
                let t = (proj(x as f64, y as f64) - lo) / (hi - lo).max(1e-6);
                let c = lerp3(from, to, t);
                row[x * 4..x * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        });
        f
    }

    fn caption_at(&self, src_t: f64) -> Option<&crate::project::Caption> {
        if !self.project.caption_style.burn_in {
            return None;
        }
        self.project.captions.iter().find(|c| src_t >= c.start_ms && src_t < c.end_ms)
    }

    fn draw_caption(&self, out: &mut Frame, c: &crate::project::Caption, src_t: f64) {
        let st = &self.project.caption_style;
        let short = self.out_w.min(self.out_h) as f64;
        let px = (st.size * short).max(12.0) as f32;
        let max_w = self.out_w as f32 * 0.84;
        // Words removed by cuts (fillers) are not shown.
        let words: Vec<&crate::project::CaptionWord> = c.words.iter().filter(|w| crate::captions::word_visible(&self.project, w)).collect();
        let body = if c.words.is_empty() { c.text.clone() } else { words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ") };
        if body.trim().is_empty() {
            return;
        }
        let lines = text::wrap(&body, px, max_w);
        let lh = text::line_height(px) * 1.15;
        let pad = px * 0.5;
        let block_h = lh * lines.len() as f32 + pad * 2.0;
        let block_w = lines.iter().map(|l| text::measure(l, px)).fold(0.0, f32::max) + pad * 2.4;
        let bx = (self.out_w as f32 - block_w) / 2.0;
        let by = self.out_h as f32 - block_h - (short as f32 * 0.05);
        fill_round_rect(out, bx as f64, by as f64, block_w as f64, block_h as f64, (px * 0.35) as f64, [10.0, 10.0, 12.0], st.background_opacity as f32);
        // Which word is being spoken now (by index into the caption's words).
        let active = words.iter().position(|w| src_t >= w.start_ms && src_t < w.end_ms);
        let col = st.color.map(|v| v as f32);
        let hi = st.highlight.map(|v| v as f32);
        let mut word_idx = 0usize;
        for (li, line) in lines.iter().enumerate() {
            let lw = text::measure(line, px);
            let mut x = (self.out_w as f32 - lw) / 2.0;
            let y = by + pad + li as f32 * lh;
            for (wi, word) in line.split(' ').enumerate() {
                let piece = if wi == 0 { word.to_string() } else { format!(" {word}") };
                let color = if Some(word_idx) == active { hi } else { col };
                x = text::draw(out, x, y, px, &piece, color, 1.0);
                word_idx += 1;
            }
        }
    }

    /// Webcam bubble diameter in output pixels.
    pub fn webcam_size(&self) -> u32 {
        let short = self.out_w.min(self.out_h) as f64;
        let d = self.project.webcam.as_ref().map(|w| w.size.clamp(0.08, 0.5)).unwrap_or(0.24) * short;
        ((d.round() as u32) & !1).max(16)
    }

    /// Composite the webcam frame (already square, `webcam_size`) in its corner.
    pub fn draw_webcam(&self, out: &mut Frame, cam: &Frame) {
        use crate::project::Corner;
        let Some(w) = self.project.webcam.as_ref() else { return };
        let d = cam.w as f64;
        let short = self.out_w.min(self.out_h) as f64;
        let m = short * 0.035;
        let (x0, y0) = match w.corner {
            Corner::TopLeft => (m, m),
            Corner::TopRight => (self.out_w as f64 - d - m, m),
            Corner::BottomLeft => (m, self.out_h as f64 - d - m),
            Corner::BottomRight => (self.out_w as f64 - d - m, self.out_h as f64 - d - m),
        };
        let r = if w.circle { d / 2.0 } else { d * 0.16 };
        let (cx, cy, h) = (x0 + d / 2.0, y0 + d / 2.0, d / 2.0);
        // Soft shadow, then the video, then a thin white ring.
        for yy in (y0 as i64 - 8)..(y0 + d) as i64 + 12 {
            for xx in (x0 as i64 - 8)..(x0 + d) as i64 + 8 {
                let (fx, fy) = (xx as f64 + 0.5, yy as f64 + 0.5);
                let ds = sd_round_rect(fx, fy - 4.0, cx, cy, h, h, r);
                if ds > 0.0 && ds < 10.0 {
                    out.blend(xx, yy, [0.0; 3], (0.22 * (1.0 - ds / 10.0)) as f32);
                }
                let dd = sd_round_rect(fx, fy, cx, cy, h, h, r);
                let cov = (0.5 - dd).clamp(0.0, 1.0) as f32;
                if cov > 0.0 {
                    let px = cam.px((fx - x0) as i64, (fy - y0) as i64);
                    out.blend(xx, yy, [px[0], px[1], px[2]], cov);
                }
                let ring = (1.5 - (dd + 1.5).abs()).clamp(0.0, 1.0) as f32;
                if ring > 0.0 {
                    out.blend(xx, yy, [255.0; 3], 0.9 * ring);
                }
            }
        }
    }

    pub fn camera(&self, src_t: f64) -> Camera {
        let p = &self.project;
        camera_at(
            src_t,
            &p.zoom,
            self.track.as_ref(),
            p.source_width as f64,
            p.source_height as f64,
            CameraTiming::new(p.zoom_config.ease_in_ms, p.zoom_config.ease_out_ms, p.cursor.smoothing_ms.max(350.0)),
        )
    }

    /// Render one output frame. `src` must be the source frame at `src_t`.
    pub fn render(&self, src: &Frame, src_t: f64) -> Frame {
        let p = &self.project;
        // 1. Blur boxes are applied in source space, so they move with zoom.
        let blurs: Vec<&BlurBox> = p.blur.iter().filter(|b| src_t >= b.start_ms && src_t <= b.end_ms).collect();
        let owned;
        let src = if blurs.is_empty() {
            src
        } else {
            let mut c = src.clone();
            // Blur boxes are in source pixels; the decoded frame may be smaller.
            let k = src.w as f64 / p.source_width as f64;
            for b in blurs {
                if (k - 1.0).abs() < 1e-9 {
                    blur_region(&mut c, b);
                } else {
                    let mut sb = b.clone();
                    sb.x *= k;
                    sb.y *= k;
                    sb.w *= k;
                    sb.h *= k;
                    sb.radius = ((b.radius as f64 * k).round() as u32).max(1);
                    blur_region(&mut c, &sb);
                }
            }
            owned = c;
            &owned
        };

        let cam = self.camera(src_t);
        let (vx, vy, vw, vh) = cam.view_rect(p.source_width as f64, p.source_height as f64);
        let sx_scale = src.w as f64 / p.source_width as f64;
        let sy_scale = src.h as f64 / p.source_height as f64;
        let l = self.layout;
        let short = self.out_w.min(self.out_h) as f64;
        let (cx, cy, hw, hh) = (l.x + l.w / 2.0, l.y + l.h / 2.0, l.w / 2.0, l.h / 2.0);

        // 2. Zoomed recording over the pre-baked background and shadow. Only the
        // rows and columns the card can touch are visited.
        let mut out = self.background.clone();
        let w = self.out_w as usize;
        let y0 = (l.y - 1.0).floor().max(0.0) as usize;
        let y1 = ((l.y + l.h + 1.0).ceil() as usize).min(self.out_h as usize);
        let x0 = (l.x - 1.0).floor().max(0.0) as usize;
        let x1 = ((l.x + l.w + 1.0).ceil() as usize).min(w);
        out.data[y0 * w * 4..y1 * w * 4].par_chunks_mut(w * 4).enumerate().for_each(|(i, row)| {
            let fy = (y0 + i) as f64 + 0.5;
            for x in x0..x1 {
                let fx = x as f64 + 0.5;
                let d = sd_round_rect(fx, fy, cx, cy, hw, hh, l.radius);
                let cov = (0.5 - d).clamp(0.0, 1.0) as f32;
                if cov > 0.0 {
                    let px = &mut row[x * 4..x * 4 + 4];
                    let u = (fx - l.x) / l.w;
                    let v = (fy - l.y) / l.h;
                    let s = src.sample((vx + u * vw) * sx_scale, (vy + v * vh) * sy_scale);
                    for k in 0..3 {
                        px[k] = (px[k] as f32 + (s[k] - px[k] as f32) * cov).round() as u8;
                    }
                }
            }
        });

        // Source -> output mapping for overlays.
        let to_out = |sx: f64, sy: f64| (l.x + (sx - vx) / vw * l.w, l.y + (sy - vy) / vh * l.h);
        let px_per_src = l.w / vw;
        let base = (p.source_width as f64 / 1440.0).max(0.75);

        // 3. Click ripples.
        if p.cursor.click_ripples {
            let rc = p.cursor.ripple_color.map(|c| c as f32);
            for &(t, x, y) in &self.clicks {
                let age = src_t - t;
                if !(0.0..=600.0).contains(&age) {
                    continue;
                }
                let k = age / 600.0;
                let (ox, oy) = to_out(x, y);
                let r = (10.0 + 34.0 * (1.0 - (1.0 - k).powi(3))) * base * px_per_src * p.cursor.size / 1.5;
                let thick = (3.5 * base * px_per_src).max(1.5);
                let alpha = (0.85 * (1.0 - k)) as f32;
                let ext = (r + thick + 2.0) as i64;
                for yy in (oy as i64 - ext)..=(oy as i64 + ext) {
                    for xx in (ox as i64 - ext)..=(ox as i64 + ext) {
                        let d = ((xx as f64 + 0.5 - ox).powi(2) + (yy as f64 + 0.5 - oy).powi(2)).sqrt();
                        let ring = (thick / 2.0 + 0.5 - (d - r).abs()).clamp(0.0, 1.0) as f32;
                        let fill = if d < r { 0.18 } else { 0.0 };
                        out.blend(xx, yy, rc, alpha * ring.max(fill));
                    }
                }
            }
        }

        // Where the cursor is in output space (for label placement).
        let cursor_out = self.track.as_ref().and_then(|tr| tr.smooth_at(src_t, p.cursor.smoothing_ms)).map(|(x, y)| to_out(x, y));

        // 4. Cursor (smoothed, optionally bigger).
        if p.cursor.visible {
            if let Some(tr) = &self.track {
                if let Some((sx, sy)) = tr.smooth_at(src_t, p.cursor.smoothing_ms) {
                    let (ox, oy) = to_out(sx, sy);
                    let s = p.cursor.size * base * px_per_src;
                    let outline: Vec<(f64, f64)> = ARROW
                        .iter()
                        .map(|&(x, y)| (ox + (x * 1.18 - 1.2) * s, oy + (y * 1.12 - 1.4) * s))
                        .collect();
                    let shadow: Vec<(f64, f64)> = outline.iter().map(|&(x, y)| (x + 1.5 * s, y + 2.0 * s)).collect();
                    let inner: Vec<(f64, f64)> = ARROW.iter().map(|&(x, y)| (ox + x * s, oy + y * s)).collect();
                    fill_poly(&mut out, &shadow, [0.0; 3], 0.25);
                    fill_poly(&mut out, &outline, [255.0; 3], 1.0);
                    fill_poly(&mut out, &inner, [17.0, 17.0, 20.0], 1.0);
                }
            }
        }

        // 5. Keystroke overlay, bottom centre of the canvas.
        if p.keys.enabled {
            if let Some(lab) = self.labels.iter().find(|k| src_t >= k.start_ms && src_t < k.end_ms) {
                let mut text: String = lab.text.chars().rev().take(28).collect::<Vec<_>>().into_iter().rev().collect();
                if text.trim().is_empty() {
                    text = "SPACE".into();
                }
                let fade_in = ((src_t - lab.start_ms) / 120.0).clamp(0.0, 1.0) as f32;
                let fade_out = ((lab.end_ms - src_t) / 200.0).clamp(0.0, 1.0) as f32;
                let a = fade_in.min(fade_out);
                let px = (short * 0.034).max(12.0) as f32;
                let tw = text::measure(&text, px) as f64;
                let th = text::line_height(px) as f64;
                let (padx, pady) = (px as f64 * 0.8, px as f64 * 0.45);
                let bw = tw + padx * 2.0;
                let bh = th + pady * 2.0;
                let bx = (self.out_w as f64 - bw) / 2.0;
                // Bottom centre, above captions when both are visible...
                let lift = if self.caption_at(src_t).is_some() { short * 0.16 } else { 0.0 };
                let bottom = self.out_h as f64 - bh - (short * 0.045).max(pady) - lift;
                // ...unless that would cover what the presenter is working on
                // (the cursor or a click in the last 1.5 s), then the top.
                let top = (short * 0.045).max(pady);
                let margin = short * 0.06;
                let hits = |y: f64| {
                    let inside = |(x0, y0): (f64, f64)| x0 >= bx - margin && x0 <= bx + bw + margin && y0 >= y - margin && y0 <= y + bh + margin;
                    let cursor = cursor_out.is_some_and(inside);
                    let click = self.clicks.iter().any(|&(t, x, y2)| (0.0..1500.0).contains(&(src_t - t)) && inside(to_out(x, y2)));
                    cursor || click
                };
                let by = if hits(bottom) && !hits(top) { top } else { bottom };
                fill_round_rect(&mut out, bx, by, bw, bh, bh * 0.3, [15.0, 15.0, 18.0], 0.82 * a);
                text::draw(&mut out, (bx + padx) as f32, (by + pady) as f32, px, &text, [250.0, 250.0, 250.0], a);
            }
        }

        // 6. Captions with the current word highlighted.
        if p.caption_style.burn_in {
            if let Some(c) = self.caption_at(src_t) {
                self.draw_caption(&mut out, c, src_t);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Event, EventKind};

    #[test]
    fn key_labels_group_typing_and_split_shortcuts() {
        let mut log = EventLog::new(100, 100);
        let k = |t: f64, key: &str, m: &[&str]| Event {
            t,
            kind: EventKind::Key { key: key.into(), modifiers: m.iter().map(|s| s.to_string()).collect() },
        };
        log.events = vec![k(0.0, "h", &[]), k(100.0, "i", &[]), k(500.0, "S", &["Ctrl"]), k(3000.0, "Enter", &[])];
        let labels = build_key_labels(&log, 1000.0, false);
        let texts: Vec<_> = labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["Typing...", "Ctrl + S", "Enter"]);
        assert_eq!(labels[0].end_ms, 500.0);
        let only = build_key_labels(&log, 1000.0, true);
        assert_eq!(only.len(), 1);
    }

    #[test]
    fn render_produces_padded_frame() {
        let mut p = Project::new("x".into(), None, 64, 36, 1000.0);
        p.style.background = Background::Solid { color: [10, 20, 30] };
        p.style.shadow = 0.0;
        let r = Renderer::new(p, None, 320, 180, None);
        let mut src = Frame::new(64, 36);
        src.data.chunks_mut(4).for_each(|c| c.copy_from_slice(&[200, 0, 0, 255]));
        let out = r.render(&src, 0.0);
        assert_eq!(&out.data[0..3], &[10, 20, 30]);
        let mid = ((90 * 320 + 160) * 4) as usize;
        assert_eq!(&out.data[mid..mid + 3], &[200, 0, 0]);
    }

    #[test]
    fn decode_size_shrinks_retina_sources_only_when_safe() {
        let mut p = Project::new("x".into(), None, 2880, 1800, 1000.0);
        let r = Renderer::new(p.clone(), None, 1920, 1080, None);
        let (w, h) = r.decode_size();
        assert!(w < 2880 && w as f64 >= r.layout.w && w % 2 == 0 && h % 2 == 0);
        // A deep zoom needs every source pixel.
        p.zoom.push(crate::zoom::ZoomSegment { start_ms: 0.0, end_ms: 500.0, scale: 2.5, follow_cursor: false, focus: (10.0, 10.0), reason: Default::default() });
        let r = Renderer::new(p, None, 1920, 1080, None);
        assert_eq!(r.decode_size(), (2880, 1800));
    }

    #[test]
    fn baked_shadow_darkens_only_outside_the_card() {
        let mut p = Project::new("x".into(), None, 64, 36, 1000.0);
        p.style.background = Background::Solid { color: [200, 200, 200] };
        p.style.shadow = 0.05;
        p.style.shadow_opacity = 0.5;
        let r = Renderer::new(p, None, 320, 180, None);
        let mut src = Frame::new(64, 36);
        src.data.chunks_mut(4).for_each(|c| c.copy_from_slice(&[0, 0, 255, 255]));
        let a = r.render(&src, 0.0);
        let b = r.render(&src, 500.0);
        assert_eq!(a.data, b.data, "frames are deterministic");
        let below = ((((r.layout.y + r.layout.h) as usize + 2) * 320 + 160) * 4) as usize;
        assert!(a.data[below] < 200, "shadow under the card");
        assert_eq!(&a.data[0..3], &[200, 200, 200]);
    }

    #[test]
    fn blur_follows_a_downscaled_source() {
        let mut p = Project::new("x".into(), None, 200, 100, 1000.0);
        p.style.shadow = 0.0;
        p.style.padding = 0.0;
        p.blur.push(BlurBox { x: 0.0, y: 0.0, w: 100.0, h: 100.0, start_ms: 0.0, end_ms: 1000.0, radius: 20, ..serde_json::from_str::<BlurBox>(r#"{"x":0,"y":0,"w":1,"h":1,"start_ms":0,"end_ms":1}"#).unwrap() });
        let r = Renderer::new(p, None, 200, 100, None);
        // Half-size source with a sharp stripe pattern in the blurred half.
        let mut src = Frame::new(100, 50);
        for (i, c) in src.data.chunks_mut(4).enumerate() {
            let v = if (i % 100) % 2 == 0 { 255 } else { 0 };
            c.copy_from_slice(&[v, v, v, 255]);
        }
        let out = r.render(&src, 10.0);
        let px = |x: usize, y: usize| out.data[(y * 200 + x) * 4] as i32;
        // Left half (blurred) is smooth, right half keeps contrast.
        assert!((px(40, 50) - px(42, 50)).abs() < 40);
        assert!((0..8).map(|x| px(150 + x, 50)).max().unwrap() - (0..8).map(|x| px(150 + x, 50)).min().unwrap() > 60);
    }
}
