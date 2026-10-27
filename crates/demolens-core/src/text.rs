//! Text rendering with an embedded font (DejaVu Sans Bold, see
//! assets/DejaVu-LICENSE.txt), used by captions and the keystroke overlay.
//! Embedding the font keeps output identical on every OS.

use crate::render::Frame;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use std::sync::OnceLock;

static FONT_BYTES: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");

fn font() -> &'static FontRef<'static> {
    static F: OnceLock<FontRef<'static>> = OnceLock::new();
    F.get_or_init(|| FontRef::try_from_slice(FONT_BYTES).expect("embedded font"))
}

/// Width in pixels of `text` at size `px`.
pub fn measure(text: &str, px: f32) -> f32 {
    let f = font().as_scaled(PxScale::from(px));
    let mut w = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = f.glyph_id(c);
        if let Some(p) = prev {
            w += f.kern(p, id);
        }
        w += f.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Line height (ascent - descent) at size `px`.
pub fn line_height(px: f32) -> f32 {
    let f = font().as_scaled(PxScale::from(px));
    f.ascent() - f.descent()
}

/// Draw `text` with its top-left at (x, y). Returns the x after the text.
pub fn draw(frame: &mut Frame, x: f32, y: f32, px: f32, text: &str, rgb: [f32; 3], alpha: f32) -> f32 {
    let f = font().as_scaled(PxScale::from(px));
    let base = y + f.ascent();
    let mut cx = x;
    let mut prev = None;
    for c in text.chars() {
        let id = f.glyph_id(c);
        if let Some(p) = prev {
            cx += f.kern(p, id);
        }
        let g = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(cx, base));
        if let Some(o) = font().outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                frame.blend_pub(b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64, rgb, cov * alpha);
            });
        }
        cx += f.h_advance(id);
        prev = Some(id);
    }
    cx
}

/// Greedy word wrap to `max_w` pixels.
pub fn wrap(text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && measure(&candidate, px) > max_w {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = candidate;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measure_and_wrap() {
        assert!(measure("Hello", 40.0) > measure("Hi", 40.0));
        let lines = wrap("one two three four five six seven", 40.0, measure("one two three", 40.0) + 1.0);
        assert_eq!(lines[0], "one two three");
        assert!(lines.len() >= 2);
    }
}
