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

/// A soft wallpaper: bilinear blend of four corner colours through a gentle
/// sine warp, so the colours swirl instead of forming a flat quad.
fn paint_mesh(mut f: Frame, c: &[[u8; 3]; 4]) -> Frame {
    let (w, h) = (f.w as usize, f.h as usize);
    let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
    f.data.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let v0 = y as f64 / h.max(1) as f64;
        for x in 0..w {
            let u0 = x as f64 / w.max(1) as f64;
            let u = smooth((u0 + 0.14 * (v0 * 4.4 + 1.0).sin()).clamp(0.0, 1.0));
            let v = smooth((v0 + 0.14 * (u0 * 4.4).sin()).clamp(0.0, 1.0));
            let top = lerp3(c[0], c[1], u);
            let bot = lerp3(c[2], c[3], u);
            let p = lerp3(top, bot, v);
            row[x * 4..x * 4 + 4].copy_from_slice(&[p[0], p[1], p[2], 255]);
        }
    });
    f
}

/// Distance from a point to the segment a-b.
fn sd_segment(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let l2 = (dx * dx + dy * dy).max(1e-9);
    let t = (((px - ax) * dx + (py - ay) * dy) / l2).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Signed distance to a triangle. Negative inside.
fn sd_triangle(px: f64, py: f64, a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let d = sd_segment(px, py, a.0, a.1, b.0, b.1).min(sd_segment(px, py, b.0, b.1, c.0, c.1)).min(sd_segment(px, py, c.0, c.1, a.0, a.1));
    let cross = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0) * (py - p.1) - (q.1 - p.1) * (px - p.0);
    let (s1, s2, s3) = (cross(a, b), cross(b, c), cross(c, a));
    let inside = (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0);
    if inside { -d } else { d }
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
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct KeyLabel {
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
}

/// Turn the event log into the labels the keystroke overlay shows.
pub fn build_key_labels(log: &EventLog, k: &crate::project::KeyOverlay) -> Vec<KeyLabel> {
    crate::keylabels::build_key_labels(
        log,
        &crate::keylabels::LabelOptions { hold_ms: k.hold_ms, shortcuts_only: k.shortcuts_only, symbols: k.symbols },
    )
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

/// Frosted glass over a rounded rectangle of `f`: what is under it is blurred,
/// tinted and lit, and the edge gets a bright rim that is strongest towards the
/// top left (like light catching a glass pane). `alpha` fades all of it.
#[allow(clippy::too_many_arguments)]
pub fn frost_rect(f: &mut Frame, x: f64, y: f64, w: f64, h: f64, radius: f64, blur_px: f64, tint: [f32; 3], tint_op: f32, highlight: f32, alpha: f32) {
    if alpha <= 0.0 || w < 2.0 || h < 2.0 {
        return;
    }
    let x0 = (x.floor() as i64).max(0);
    let y0 = (y.floor() as i64).max(0);
    let x1 = ((x + w).ceil() as i64).min(f.w as i64);
    let y1 = ((y + h).ceil() as i64).min(f.h as i64);
    if x1 <= x0 + 1 || y1 <= y0 + 1 {
        return;
    }
    let (rw, rh) = ((x1 - x0) as u32, (y1 - y0) as u32);
    // Blur a copy of the region, so the frost only ever shows what is behind the glass.
    let mut tile = Frame::new(rw, rh);
    for row in 0..rh as usize {
        let src = ((y0 as usize + row) * f.w as usize + x0 as usize) * 4;
        let dst = row * rw as usize * 4;
        tile.data[dst..dst + rw as usize * 4].copy_from_slice(&f.data[src..src + rw as usize * 4]);
    }
    if blur_px >= 1.0 {
        let b = BlurBox { x: 0.0, y: 0.0, w: rw as f64, h: rh as f64, start_ms: 0.0, end_ms: 0.0, radius: (blur_px * 2.0).round().max(2.0) as u32 };
        blur_region(&mut tile, &b);
    }
    let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let radius = radius.clamp(0.0, hw.min(hh));
    for py in y0..y1 {
        for px in x0..x1 {
            let (fx, fy) = (px as f64 + 0.5, py as f64 + 0.5);
            let d = sd_round_rect(fx, fy, cx, cy, hw, hh, radius);
            let cov = (0.5 - d).clamp(0.0, 1.0) as f32;
            if cov > 0.0 {
                let ti = (((py - y0) as usize) * rw as usize + (px - x0) as usize) * 4;
                // Lighter towards the top, like a lit pane.
                let sheen = (1.0 - ((fy - y) / h).clamp(0.0, 1.0)) as f32 * 0.07;
                let mut c = [0f32; 3];
                for k in 0..3 {
                    let v = tile.data[ti + k] as f32 * (1.0 - tint_op) + tint[k] * tint_op;
                    c[k] = v + (255.0 - v) * sheen;
                }
                f.blend(px, py, c, cov * alpha);
            }
            let rim = (1.4 - (d + 0.7).abs()).clamp(0.0, 1.0) as f32;
            if rim > 0.0 && highlight > 0.0 {
                // Outward normal from the distance field.
                let nx = sd_round_rect(fx + 1.0, fy, cx, cy, hw, hh, radius) - sd_round_rect(fx - 1.0, fy, cx, cy, hw, hh, radius);
                let ny = sd_round_rect(fx, fy + 1.0, cx, cy, hw, hh, radius) - sd_round_rect(fx, fy - 1.0, cx, cy, hw, hh, radius);
                let n = (nx * nx + ny * ny).sqrt().max(1e-9);
                let toward = ((-nx / n) * 0.7071 + (-ny / n) * 0.7071) as f32;
                let wgt = 0.30 + 0.70 * toward.max(0.0) + 0.35 * (-toward).max(0.0);
                f.blend(px, py, [255.0; 3], (highlight * wgt * rim * alpha).min(1.0));
            }
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
    /// The plain backdrop, used behind title cards.
    card_bg: Frame,
    pub layout: Layout,
    /// The glass panel behind the recording (glass style only).
    pub panel: Option<Layout>,
    glass: crate::project::Glass,
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
            Some(l) if project.keys.enabled => build_key_labels(l, &project.keys),
            _ => Vec::new(),
        };
        let background = Self::paint_background(&project.style.background, out_w, out_h, bg_image);
        let short = out_w.min(out_h) as f64;
        let glass = project.style.glass.clamped();
        let inset_px = if glass.enabled { (glass.inset * short).round() } else { 0.0 };
        // With glass, the panel needs its own margin around the recording.
        let pad = project.style.padding.clamp(0.0, 0.4) * short + inset_px;
        // A side-by-side webcam takes a column off the screen's side.
        let col = Self::side_column(&project, out_w, out_h);
        let (aw, ah) = (out_w as f64 - 2.0 * pad - col.map(|c| c.0).unwrap_or(0.0), out_h as f64 - 2.0 * pad);
        let src_aspect = project.source_width as f64 / project.source_height as f64;
        let (w, h) = if aw / ah > src_aspect { (ah * src_aspect, ah) } else { (aw, aw / src_aspect) };
        // Centre the screen in the area that is left (right of the camera when it is on the left).
        let area_x = match col {
            Some((c, true)) => c + pad,
            _ => pad,
        };
        let layout = Layout {
            x: if col.is_some() { area_x + (aw - w) / 2.0 } else { (out_w as f64 - w) / 2.0 },
            y: (out_h as f64 - h) / 2.0,
            w,
            h,
            radius: (project.style.corner_radius * short).min(w.min(h) / 2.0),
        };
        let mut layout = layout;
        let panel = glass.enabled.then(|| {
            let (pw, ph) = (layout.w + 2.0 * inset_px, layout.h + 2.0 * inset_px);
            let pr = (glass.radius * short).min(pw.min(ph) / 2.0);
            // Concentric corners: the recording's radius follows the panel's.
            layout.radius = layout.radius.max((pr - inset_px).max(0.0)).min(layout.w.min(layout.h) / 2.0);
            Layout { x: layout.x - inset_px, y: layout.y - inset_px, w: pw, h: ph, radius: pr }
        });
        let card_bg = background.clone();
        let mut background = background;
        Self::bake_shadow(&mut background, &project, panel.unwrap_or(layout), out_w, out_h);
        if let Some(pl) = panel {
            let t = glass.tint.map(|v| v as f32);
            frost_rect(&mut background, pl.x, pl.y, pl.w, pl.h, pl.radius, glass.blur * short, t, glass.tint_opacity as f32, glass.highlight as f32, 1.0);
        }
        Self { project, out_w, out_h, track, clicks, labels, background, card_bg, layout, panel, glass }
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
        if let Background::Mesh { colors } = bg {
            return paint_mesh(f, colors);
        }
        let (from, to, angle) = match bg {
            Background::Solid { color } => (*color, *color, 0.0),
            Background::Gradient { from, to, angle_deg } => (*from, *to, *angle_deg),
            Background::Image { .. } | Background::Mesh { .. } => ([24, 24, 27], [24, 24, 27], 0.0),
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

    /// The plate behind captions, callouts and key labels: a dark translucent
    /// pill, or frosted glass when the glass style is on. `fade` is 0 to 1.
    #[allow(clippy::too_many_arguments)]
    fn plate(&self, out: &mut Frame, x: f64, y: f64, w: f64, h: f64, r: f64, rgb: [f32; 3], alpha: f32, fade: f32) {
        let g = &self.glass;
        if g.enabled && g.overlays {
            let short = self.out_w.min(self.out_h) as f64;
            frost_rect(out, x, y, w, h, r, (g.blur * short * 0.7).max(3.0), [18.0, 20.0, 28.0], 0.55, g.highlight as f32, fade);
        } else {
            fill_round_rect(out, x, y, w, h, r, rgb, alpha * fade);
        }
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
        self.plate(out, bx as f64, by as f64, block_w as f64, block_h as f64, (px * 0.35) as f64, [10.0, 10.0, 12.0], st.background_opacity as f32, 1.0);
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

    /// Width of the camera column and whether it is on the left, for the side layout.
    fn side_column(project: &Project, out_w: u32, out_h: u32) -> Option<(f64, bool)> {
        use crate::project::{Corner, WebcamLayout};
        let w = project.webcam.as_ref().filter(|w| w.visible && w.layout == WebcamLayout::Side)?;
        let short = out_w.min(out_h) as f64;
        let d = ((w.size.clamp(0.08, 0.5) * 1.6).min(0.45) * short).max(16.0);
        let left = matches!(w.corner, Corner::TopLeft | Corner::BottomLeft);
        Some((d + short * 0.035, left))
    }

    /// Webcam bubble diameter in output pixels.
    pub fn webcam_size(&self) -> u32 {
        let short = self.out_w.min(self.out_h) as f64;
        let d = match Self::side_column(&self.project, self.out_w, self.out_h) {
            Some((col, _)) => col - short * 0.035,
            None => self.project.webcam.as_ref().map(|w| w.size.clamp(0.08, 0.5)).unwrap_or(0.24) * short,
        };
        ((d.round() as u32) & !1).max(16)
    }

    /// Composite the webcam frame (already square, `webcam_size`) in its corner.
    pub fn draw_webcam(&self, out: &mut Frame, cam: &Frame) {
        use crate::project::Corner;
        let Some(w) = self.project.webcam.as_ref() else { return };
        let d = cam.w as f64;
        let short = self.out_w.min(self.out_h) as f64;
        let m = short * 0.035;
        let side = Self::side_column(&self.project, self.out_w, self.out_h);
        let (x0, y0) = match w.corner {
            _ if side.is_some() => {
                let left = side.map(|s| s.1).unwrap_or(false);
                // Centred vertically, a margin from its edge.
                (if left { m } else { self.out_w as f64 - d - m }, (self.out_h as f64 - d) / 2.0)
            }
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
            {
                let mut t = CameraTiming::new(p.zoom_config.ease_in_ms, p.zoom_config.ease_out_ms, p.cursor.smoothing_ms.max(600.0));
                t.deadzone = p.zoom_config.steadiness.clamp(0.0, 1.0) * 0.6;
                t
            },
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
                self.plate(&mut out, bx, by, bw, bh, bh * 0.3, [15.0, 15.0, 18.0], 0.82, a);
                text::draw(&mut out, (bx + padx) as f32, (by + pady) as f32, px, &text, [250.0, 250.0, 250.0], a);
            }
        }

        // 6. Captions with the current word highlighted.
        if p.caption_style.burn_in {
            if let Some(c) = self.caption_at(src_t) {
                self.draw_caption(&mut out, c, src_t);
            }
        }
        self.draw_shapes(&mut out, src_t);
        self.draw_callouts(&mut out, src_t);
        self.draw_watermark(&mut out);
        out
    }

    /// Arrows, boxes, ellipses and highlights, anti-aliased, fading in and out.
    fn draw_shapes(&self, out: &mut Frame, src_t: f64) {
        use crate::project::ShapeKind;
        let (w, h) = (self.out_w as f64, self.out_h as f64);
        let short = w.min(h);
        for s in &self.project.shapes {
            if src_t < s.start_ms || src_t > s.end_ms {
                continue;
            }
            let k = (((src_t - s.start_ms) / 200.0).min((s.end_ms - src_t) / 200.0)).clamp(0.0, 1.0);
            let k = (k * k * (3.0 - 2.0 * k)) as f32;
            let rgb = s.rgb();
            let lw = (s.width.clamp(0.001, 0.05) * short).max(2.0);
            let (x1, y1, x2, y2) = (s.x1.clamp(0.0, 1.0) * w, s.y1.clamp(0.0, 1.0) * h, s.x2.clamp(0.0, 1.0) * w, s.y2.clamp(0.0, 1.0) * h);
            let (bx0, bx1, by0, by1) = (x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2));
            let pad = lw * 2.0 + 2.0;
            let region = |f: &mut Frame, x0: f64, y0: f64, x1: f64, y1: f64, sd: &dyn Fn(f64, f64) -> f64, fill: bool, alpha: f32| {
                for py in ((y0 - pad).floor().max(0.0) as i64)..=((y1 + pad).ceil().min(h - 1.0) as i64) {
                    for px in ((x0 - pad).floor().max(0.0) as i64)..=((x1 + pad).ceil().min(w - 1.0) as i64) {
                        let d = sd(px as f64 + 0.5, py as f64 + 0.5);
                        let d = if fill { d } else { d.abs() - lw / 2.0 };
                        let cov = (0.5 - d).clamp(0.0, 1.0) as f32;
                        f.blend(px, py, rgb, alpha * cov);
                    }
                }
            };
            match s.kind {
                ShapeKind::Highlight | ShapeKind::Box => {
                    let (cx, cy, hw, hh) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0, ((bx1 - bx0) / 2.0).max(2.0), ((by1 - by0) / 2.0).max(2.0));
                    let r = (short * 0.012).min(hw).min(hh);
                    let sd = move |px: f64, py: f64| sd_round_rect(px, py, cx, cy, hw, hh, r);
                    if s.kind == ShapeKind::Highlight {
                        region(out, bx0, by0, bx1, by1, &sd, true, 0.38 * k);
                    } else {
                        region(out, bx0, by0, bx1, by1, &sd, false, k);
                    }
                }
                ShapeKind::Ellipse => {
                    let (cx, cy, rx, ry) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0, ((bx1 - bx0) / 2.0).max(2.0), ((by1 - by0) / 2.0).max(2.0));
                    // First order distance to an ellipse: good enough for strokes.
                    let sd = move |px: f64, py: f64| {
                        let (nx, ny) = ((px - cx) / rx, (py - cy) / ry);
                        let len = (nx * nx + ny * ny).sqrt();
                        let grad = ((nx / rx).powi(2) + (ny / ry).powi(2)).sqrt() / len.max(1e-6);
                        (len - 1.0) / grad.max(1e-6)
                    };
                    region(out, bx0, by0, bx1, by1, &sd, false, k);
                }
                ShapeKind::Arrow => {
                    let (dx, dy) = (x2 - x1, y2 - y1);
                    let len = (dx * dx + dy * dy).sqrt();
                    if len < 4.0 {
                        continue;
                    }
                    let (ux, uy) = (dx / len, dy / len);
                    let head = (lw * 4.5).min(len * 0.6);
                    let (sx, sy) = (x2 - ux * head * 0.8, y2 - uy * head * 0.8);
                    let (nx, ny) = (-uy, ux);
                    let (ax, ay) = (x2 - ux * head + nx * head * 0.5, y2 - uy * head + ny * head * 0.5);
                    let (cx2, cy2) = (x2 - ux * head - nx * head * 0.5, y2 - uy * head - ny * head * 0.5);
                    // Filled head (triangle) and a shaft that ends inside it.
                    let sd = move |px: f64, py: f64| {
                        let shaft = sd_segment(px, py, x1, y1, sx, sy) - lw / 2.0;
                        let tri = sd_triangle(px, py, (x2, y2), (ax, ay), (cx2, cy2));
                        shaft.min(tri)
                    };
                    region(out, bx0, by0, bx1, by1, &sd, true, k);
                }
            }
        }
    }

    /// Notes that fade in and out over the video, in a rounded pill.
    fn draw_callouts(&self, out: &mut Frame, src_t: f64) {
        let short = self.out_w.min(self.out_h) as f32;
        let px = (short * 0.036).max(12.0);
        for c in &self.project.callouts {
            if src_t < c.start_ms || src_t > c.end_ms || c.text.trim().is_empty() {
                continue;
            }
            // 250 ms fades at each end.
            let k = (((src_t - c.start_ms) / 250.0).min((c.end_ms - src_t) / 250.0)).clamp(0.0, 1.0) as f32;
            let k = k * k * (3.0 - 2.0 * k);
            let lines = text::wrap(c.text.trim(), px, self.out_w as f32 * 0.6);
            let lh = text::line_height(px) * 1.1;
            let pad = px * 0.6;
            let bw = lines.iter().map(|l| text::measure(l, px)).fold(0.0, f32::max) + pad * 2.0;
            let bh = lh * lines.len() as f32 + pad * 1.2;
            let bx = (c.x.clamp(0.0, 1.0) as f32 * self.out_w as f32 - bw / 2.0).clamp(4.0, (self.out_w as f32 - bw - 4.0).max(4.0));
            let by = (c.y.clamp(0.0, 1.0) as f32 * self.out_h as f32 - bh / 2.0).clamp(4.0, (self.out_h as f32 - bh - 4.0).max(4.0));
            self.plate(out, bx as f64, by as f64, bw as f64, bh as f64, (bh * 0.32) as f64, [20.0, 20.0, 26.0], 0.86, k);
            for (i, line) in lines.iter().enumerate() {
                let x = bx + (bw - text::measure(line, px)) / 2.0;
                text::draw(out, x, by + pad * 0.6 + i as f32 * lh, px, line, [255.0, 255.0, 255.0], k);
            }
        }
    }

    /// A title card over the style's background. `k` (0 to 1) is how visible the text is,
    /// so a card can fade its words in and out.
    pub fn card_frame(&self, card: &crate::project::Card, k: f32) -> Frame {
        let mut f = self.card_bg.clone();
        let short = self.out_w.min(self.out_h) as f32;
        // Dark text on a light backdrop, white on a dark one.
        let n = (f.data.len() / 4).max(1);
        let step = (n / 400).max(1);
        let (mut sum, mut cnt) = (0.0f32, 0.0f32);
        for i in (0..n).step_by(step) {
            let d = &f.data[i * 4..i * 4 + 3];
            sum += 0.2126 * d[0] as f32 + 0.7152 * d[1] as f32 + 0.0722 * d[2] as f32;
            cnt += 1.0;
        }
        let light = sum / cnt > 150.0;
        let (ink, shadow) = if light { ([15.0, 15.0, 20.0], [255.0, 255.0, 255.0]) } else { ([255.0, 255.0, 255.0], [0.0, 0.0, 0.0]) };
        let tpx = short * 0.085;
        let spx = tpx * 0.42;
        let max_w = self.out_w as f32 * 0.8;
        let title = text::wrap(card.title.trim(), tpx, max_w);
        let sub = text::wrap(card.subtitle.trim(), spx, max_w);
        let th = text::line_height(tpx) * title.len() as f32;
        let sh = text::line_height(spx) * 1.15 * sub.len() as f32;
        let gap = if title.is_empty() || sub.is_empty() { 0.0 } else { tpx * 0.35 };
        let mut y = (self.out_h as f32 - (th + gap + sh)) / 2.0;
        for line in &title {
            let x = (self.out_w as f32 - text::measure(line, tpx)) / 2.0;
            text::draw(&mut f, x + 2.0, y + 2.0, tpx, line, shadow, k * 0.25);
            text::draw(&mut f, x, y, tpx, line, ink, k);
            y += text::line_height(tpx);
        }
        y += gap;
        for line in &sub {
            let x = (self.out_w as f32 - text::measure(line, spx)) / 2.0;
            text::draw(&mut f, x, y, spx, line, ink, k * 0.8);
            y += text::line_height(spx) * 1.15;
        }
        f
    }

    /// The watermark text, in its corner with a soft shadow so it reads on any background.
    fn draw_watermark(&self, out: &mut Frame) {
        use crate::project::Corner;
        let Some(w) = self.project.watermark.as_ref().filter(|w| !w.text.trim().is_empty()) else { return };
        let short = self.out_w.min(self.out_h) as f32;
        let px = (w.size.clamp(0.012, 0.08) as f32 * short).max(10.0);
        let alpha = w.opacity.clamp(0.1, 1.0) as f32;
        let text: String = w.text.chars().take(80).collect();
        let tw = text::measure(&text, px);
        let m = short * 0.035;
        let x = match w.corner {
            Corner::TopLeft | Corner::BottomLeft => m,
            _ => self.out_w as f32 - tw - m,
        };
        let y = match w.corner {
            Corner::TopLeft | Corner::TopRight => m,
            _ => self.out_h as f32 - text::line_height(px) - m,
        };
        text::draw(out, x + 1.5, y + 1.5, px, &text, [0.0, 0.0, 0.0], alpha * 0.55);
        text::draw(out, x, y, px, &text, [255.0, 255.0, 255.0], alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Event, EventKind};

    #[test]
    fn key_labels_group_typing_and_split_shortcuts() {
        let mut log = EventLog::new(100, 100);
        log.header.platform = "linux".into();
        let k = |t: f64, key: &str, m: &[&str]| Event {
            t,
            kind: EventKind::Key { key: key.into(), modifiers: m.iter().map(|s| s.to_string()).collect() },
        };
        log.events = vec![k(0.0, "h", &[]), k(100.0, "i", &[]), k(500.0, "S", &["Ctrl"]), k(3000.0, "Enter", &[])];
        let mut ko = crate::project::KeyOverlay { enabled: true, shortcuts_only: false, hold_ms: 1000.0, symbols: false };
        let labels = build_key_labels(&log, &ko);
        let texts: Vec<_> = labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["Typing...", "Ctrl + S", "Enter"]);
        assert_eq!(labels[0].end_ms, 500.0);
        ko.shortcuts_only = true;
        let only = build_key_labels(&log, &ko);
        assert_eq!(only.len(), 2, "typing is hidden, the shortcut and the Enter key stay");
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

#[cfg(test)]
mod webcam_layout_tests {
    use super::*;
    use crate::project::{Corner, Webcam, WebcamLayout};

    fn project(layout: WebcamLayout, corner: Corner) -> Project {
        let mut p = Project::new("a.mp4".into(), None, 1920, 1080, 5000.0);
        let mut w = Webcam::new("c.mkv".into());
        w.layout = layout;
        w.corner = corner;
        p.webcam = Some(w);
        p
    }

    #[test]
    fn side_layout_shrinks_the_screen_and_keeps_the_camera_clear_of_it() {
        let bubble = Renderer::new(project(WebcamLayout::Bubble, Corner::BottomLeft), None, 1280, 720, None);
        let side = Renderer::new(project(WebcamLayout::Side, Corner::BottomLeft), None, 1280, 720, None);
        assert!(side.layout.w < bubble.layout.w);
        let cam_right_edge = 720.0 * 0.035 / 1.0 + side.webcam_size() as f64;
        assert!(side.layout.x >= cam_right_edge - 1.0, "screen starts at {} but the camera ends at {}", side.layout.x, cam_right_edge);
        let right = Renderer::new(project(WebcamLayout::Side, Corner::TopRight), None, 1280, 720, None);
        assert!(right.layout.x + right.layout.w <= 1280.0 - right.webcam_size() as f64);
    }
}

#[cfg(test)]
mod watermark_tests {
    use super::*;
    use crate::project::Watermark;

    #[test]
    fn watermark_draws_in_its_corner_only() {
        let mut p = Project::new("a.mp4".into(), None, 640, 360, 1000.0);
        p.style.padding = 0.0;
        p.watermark = Some(Watermark { text: "acme.com".into(), opacity: 1.0, ..Default::default() });
        let r = Renderer::new(p.clone(), None, 640, 360, None);
        let src = Frame { w: 640, h: 360, data: vec![0; 640 * 360 * 4].chunks_exact_mut(4).flat_map(|_| [10u8, 10, 10, 255]).collect() };
        let with = r.render(&src, 0.0);
        p.watermark = None;
        let without = Renderer::new(p, None, 640, 360, None).render(&src, 0.0);
        let diff = |x0: usize, x1: usize, y0: usize, y1: usize| (y0..y1).any(|y| (x0..x1).any(|x| with.data[(y * 640 + x) * 4] != without.data[(y * 640 + x) * 4]));
        assert!(diff(480, 640, 300, 360), "nothing drawn in the bottom right");
        assert!(!diff(0, 320, 0, 180), "something drawn far from the corner");
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;
    use crate::project::Card;

    #[test]
    fn a_card_draws_its_title_in_the_middle_and_can_fade() {
        let mut p = Project::new("a.mp4".into(), None, 640, 360, 1000.0);
        p.style.background = crate::project::Background::Solid { color: [10, 10, 10] };
        let r = Renderer::new(p, None, 640, 360, None);
        let card = Card { title: "Hello".into(), subtitle: "World".into(), duration_ms: 2000.0 };
        let on = r.card_frame(&card, 1.0);
        let off = r.card_frame(&card, 0.0);
        assert_ne!(on.data, off.data, "text should show");
        assert_eq!(off.data[0], 10);
        // Nothing drawn in a far corner.
        assert_eq!(on.data[(5 * 640 + 5) * 4], 10);
    }
}

#[cfg(test)]
mod callout_tests {
    use super::*;
    use crate::project::Callout;

    #[test]
    fn a_callout_shows_only_inside_its_time_range() {
        let mut p = Project::new("a.mp4".into(), None, 640, 360, 10_000.0);
        p.style.padding = 0.0;
        p.callouts.push(Callout { text: "Click here".into(), start_ms: 1000.0, end_ms: 4000.0, x: 0.5, y: 0.8 });
        let r = Renderer::new(p, None, 640, 360, None);
        let src = Frame { w: 640, h: 360, data: [10u8, 10, 10, 255].iter().cycle().take(640 * 360 * 4).copied().collect() };
        let before = r.render(&src, 500.0);
        let during = r.render(&src, 2500.0);
        let after = r.render(&src, 4500.0);
        assert_eq!(before.data, after.data);
        assert_ne!(before.data, during.data);
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;
    use crate::project::{Shape, ShapeKind};

    fn shape(kind: ShapeKind) -> Shape {
        Shape { kind, start_ms: 1000.0, end_ms: 4000.0, x1: 0.2, y1: 0.2, x2: 0.7, y2: 0.6, color: "#ff0000".into(), width: 0.008 }
    }

    fn frames(kind: ShapeKind) -> (Frame, Frame, Frame) {
        let mut p = Project::new("a.mp4".into(), None, 640, 360, 10_000.0);
        p.style.padding = 0.0;
        p.shapes.push(shape(kind));
        let r = Renderer::new(p, None, 640, 360, None);
        let src = Frame { w: 640, h: 360, data: [10u8, 10, 10, 255].iter().cycle().take(640 * 360 * 4).copied().collect() };
        (r.render(&src, 500.0), r.render(&src, 2500.0), r.render(&src, 4500.0))
    }

    #[test]
    fn every_shape_shows_only_inside_its_time_range() {
        for k in [ShapeKind::Arrow, ShapeKind::Box, ShapeKind::Ellipse, ShapeKind::Highlight] {
            let (before, during, after) = frames(k);
            assert_eq!(before.data, after.data, "{k:?}");
            assert_ne!(before.data, during.data, "{k:?}");
        }
    }

    #[test]
    fn a_box_outlines_and_a_highlight_fills() {
        let px = |f: &Frame, x: usize, y: usize| f.data[(y * 640 + x) * 4];
        let (_, boxed, _) = frames(ShapeKind::Box);
        let (_, hi, _) = frames(ShapeKind::Highlight);
        // Centre of the shape: empty for the outline, tinted for the highlight.
        assert_eq!(px(&boxed, 288, 144), 10);
        assert!(px(&hi, 288, 144) > 60);
        // On the left edge of the box (x = 0.2 * 640 = 128).
        assert!(px(&boxed, 128, 144) > 100);
    }

    #[test]
    fn an_arrow_has_a_head_at_the_target() {
        let (_, a, _) = frames(ShapeKind::Arrow);
        let px = |x: usize, y: usize| a.data[(y * 640 + x) * 4];
        // Just inside the tip (0.7 * 640 = 448, 0.6 * 360 = 216).
        assert!(px(442, 213) > 100 || px(440, 212) > 100);
        assert_eq!(px(600, 40), 10);
    }

    #[test]
    fn an_unreadable_colour_falls_back_to_amber() {
        let mut s = shape(ShapeKind::Box);
        s.color = "nope".into();
        assert_eq!(s.rgb(), [251.0, 191.0, 36.0]);
    }
}

#[cfg(test)]
mod mesh_tests {
    use super::*;

    #[test]
    fn a_mesh_wallpaper_blends_its_four_corners() {
        let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        let f = paint_mesh(Frame::new(200, 120), &colors);
        let px = |x: usize, y: usize| [f.data[(y * 200 + x) * 4], f.data[(y * 200 + x) * 4 + 1], f.data[(y * 200 + x) * 4 + 2]];
        assert!(px(1, 1)[0] > 200 && px(1, 1)[2] < 60, "top left is red: {:?}", px(1, 1));
        assert!(px(198, 118)[0] > 200 && px(198, 118)[1] > 200, "bottom right is yellow: {:?}", px(198, 118));
        assert_ne!(px(100, 60), px(1, 1));
    }

    #[test]
    fn mesh_round_trips_through_json() {
        let b = Background::Mesh { colors: [[1, 2, 3]; 4] };
        let s = serde_json::to_string(&b).unwrap();
        assert!(s.contains("\"type\":\"mesh\""));
        assert_eq!(serde_json::from_str::<Background>(&s).unwrap(), b);
    }
}

#[cfg(test)]
mod glass_tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Frame {
        let mut f = Frame::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if (x / 4 + y / 4) % 2 == 0 { 230 } else { 20 };
                let i = ((y * w + x) * 4) as usize;
                f.data[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        f
    }

    fn at(f: &Frame, x: u32, y: u32) -> u8 {
        f.data[((y * f.w + x) * 4) as usize]
    }

    #[test]
    fn frost_blurs_inside_and_leaves_the_outside_alone() {
        let before = checker(200, 120);
        let mut f = before.clone();
        frost_rect(&mut f, 40.0, 30.0, 120.0, 60.0, 12.0, 8.0, [255.0; 3], 0.0, 0.0, 1.0);
        assert_eq!(at(&f, 5, 5), at(&before, 5, 5), "outside must not change");
        assert_eq!(at(&f, 195, 115), at(&before, 195, 115));
        // The checkerboard collapses towards its mean inside.
        let spread = |fr: &Frame| {
            let v: Vec<u8> = (40..60).map(|x| at(fr, x, 60)).collect();
            *v.iter().max().unwrap() as i32 - *v.iter().min().unwrap() as i32
        };
        assert!(spread(&f) < spread(&before) / 2, "frost should blur: {} vs {}", spread(&f), spread(&before));
    }

    #[test]
    fn the_rim_light_is_stronger_at_the_top_left_than_the_bottom_right() {
        let mut f = Frame::new(200, 120);
        for px in f.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[40, 40, 40, 255]);
        }
        frost_rect(&mut f, 20.0, 20.0, 160.0, 80.0, 16.0, 0.0, [40.0; 3], 0.0, 1.0, 1.0);
        let rim = |x: u32, y: u32| (0..4).map(|d| at(&f, x, y + d)).max().unwrap();
        // Top edge, left of centre, versus the bottom edge, right of centre.
        let top_left = (0..4).map(|d| at(&f, 50, 20 + d)).max().unwrap();
        let bottom_right = (0..4).map(|d| at(&f, 150, 96 + d)).max().unwrap();
        assert!(top_left > bottom_right, "top left {top_left} should beat bottom right {bottom_right}");
        assert!(top_left > 120, "rim should be visibly bright, got {top_left}");
        let _ = rim;
    }

    #[test]
    fn glass_adds_a_panel_around_a_smaller_recording() {
        let mut p = Project::new("a.mp4".into(), None, 1920, 1080, 5000.0);
        let plain = Renderer::new(p.clone(), None, 1280, 720, None);
        assert!(plain.panel.is_none());
        p.style.glass.enabled = true;
        let g = Renderer::new(p, None, 1280, 720, None);
        let panel = g.panel.expect("glass has a panel");
        assert!(g.layout.w < plain.layout.w, "the recording makes room for the panel");
        assert!(panel.x < g.layout.x && panel.y < g.layout.y);
        assert!(panel.x + panel.w > g.layout.x + g.layout.w);
        assert!(panel.radius >= g.layout.radius);
        assert!(panel.x >= 0.0 && panel.x + panel.w <= 1280.0 && panel.y >= 0.0 && panel.y + panel.h <= 720.0);
    }

    #[test]
    fn glass_overlays_differ_from_the_flat_pill() {
        use crate::project::Callout;
        let mut p = Project::new("a.mp4".into(), None, 640, 360, 4000.0);
        p.callouts.push(Callout { text: "Hello".into(), x: 0.5, y: 0.5, start_ms: 0.0, end_ms: 3000.0 });
        let src = checker(640, 360);
        let flat = Renderer::new(p.clone(), None, 640, 360, None).render(&src, 1500.0);
        p.style.glass.enabled = true;
        p.style.glass.overlays = true;
        let glass = Renderer::new(p.clone(), None, 640, 360, None).render(&src, 1500.0);
        assert_ne!(flat.data, glass.data);
        // With overlays off the callout pill is the flat one again; only the frame differs.
        p.style.glass.overlays = false;
        let no_overlay = Renderer::new(p, None, 640, 360, None);
        assert!(no_overlay.panel.is_some());
    }
}
