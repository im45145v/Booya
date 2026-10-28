//! Device frames: simple vector shapes drawn round the recording.
//!
//! Only plain rounded rectangles, circles and neutral greys. No logos, no
//! notch copied from one maker, no artwork that belongs to anyone. The frame
//! is part of the picture's background, so it never moves with a zoom: the
//! recording zooms inside its "screen".

use crate::project::DeviceFrame;

/// Space the frame needs round the screen, in output pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Insets {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Insets {
    pub fn width(&self) -> f64 {
        self.left + self.right
    }
    pub fn height(&self) -> f64 {
        self.top + self.bottom
    }
}

/// How much room the frame takes. `short` is the shorter side of the output.
pub fn insets(frame: DeviceFrame, short: f64) -> Insets {
    match frame {
        DeviceFrame::None => Insets::default(),
        DeviceFrame::Browser => {
            let bar = (short * 0.048).max(24.0);
            Insets { left: 0.0, top: bar, right: 0.0, bottom: 0.0 }
        }
        DeviceFrame::Laptop => {
            let b = (short * 0.012).max(5.0);
            let base = (short * 0.024).max(10.0);
            let ext = (short * 0.03).max(12.0);
            Insets { left: b + ext, top: b * 1.7, right: b + ext, bottom: b + base }
        }
        DeviceFrame::Phone => {
            let b = (short * 0.02).max(8.0);
            Insets { left: b, top: b * 2.4, right: b, bottom: b * 1.9 }
        }
    }
}

/// Corner radius the recording should have inside this frame.
pub fn screen_radius(frame: DeviceFrame, short: f64) -> Option<f64> {
    match frame {
        DeviceFrame::None => None,
        DeviceFrame::Browser => Some(0.0),
        DeviceFrame::Laptop => Some(short * 0.004),
        DeviceFrame::Phone => Some(short * 0.028),
    }
}

/// The device's outer shape round a screen at (x, y, w, h): where its shadow
/// falls and what it covers.
pub fn outer(frame: DeviceFrame, screen: (f64, f64, f64, f64), short: f64) -> (f64, f64, f64, f64, f64) {
    let i = insets(frame, short);
    let (x, y, w, h) = screen;
    let (l, r) = match frame {
        // The laptop's lid is narrower than its base, and it is the lid that
        // casts the shadow.
        DeviceFrame::Laptop => (i.left - (short * 0.03).max(12.0), i.right - (short * 0.03).max(12.0)),
        _ => (i.left, i.right),
    };
    let radius = match frame {
        DeviceFrame::None => 0.0,
        DeviceFrame::Browser => short * 0.012,
        DeviceFrame::Laptop => short * 0.02,
        DeviceFrame::Phone => short * 0.05,
    };
    (x - l, y - i.top, w + l + r, h + i.top + i.bottom, radius)
}

/// Paint the frame into `bg` (the picture behind the recording).
pub fn draw(bg: &mut crate::render::Frame, frame: DeviceFrame, screen: (f64, f64, f64, f64), short: f64) {
    use crate::render::{fill_circle, fill_round_rect};
    let (x, y, w, h) = screen;
    let i = insets(frame, short);
    match frame {
        DeviceFrame::None => {}
        DeviceFrame::Browser => {
            let (ox, oy, ow, oh, r) = outer(frame, screen, short);
            // Window body, then the title bar over its top.
            fill_round_rect(bg, ox, oy, ow, oh, r, [250.0, 250.0, 251.0], 1.0);
            fill_round_rect(bg, ox, oy, ow, i.top + r, r, [233.0, 234.0, 238.0], 1.0);
            // Square off the bottom of the bar so only the top corners round.
            fill_round_rect(bg, ox, oy + i.top - 1.0, ow, r + 2.0, 0.0, [250.0, 250.0, 251.0], 1.0);
            let cy = oy + i.top / 2.0;
            let d = (i.top * 0.17).max(3.0);
            for (k, shade) in [[196.0, 198.0, 204.0], [196.0, 198.0, 204.0], [196.0, 198.0, 204.0]].iter().enumerate() {
                fill_circle(bg, ox + i.top * 0.55 + k as f64 * d * 2.9, cy, d, *shade, 1.0);
            }
            // The address field.
            let fw = ow * 0.46;
            let fh = i.top * 0.52;
            fill_round_rect(bg, ox + (ow - fw) / 2.0, cy - fh / 2.0, fw, fh, fh / 2.0, [250.0, 250.0, 252.0], 1.0);
            // A thin edge under the bar.
            fill_round_rect(bg, ox, oy + i.top - 1.0, ow, 1.0, 0.0, [214.0, 216.0, 222.0], 1.0);
        }
        DeviceFrame::Laptop => {
            let b = i.right.min(i.left) - (short * 0.03).max(12.0);
            let lid_x = x - b;
            let lid_y = y - i.top;
            let lid_w = w + 2.0 * b;
            let lid_h = h + i.top + b;
            let lr = short * 0.02;
            // Lid in a dark, slightly warm grey, then the screen glass.
            fill_round_rect(bg, lid_x, lid_y, lid_w, lid_h, lr, [34.0, 35.0, 39.0], 1.0);
            // Camera dot in the top bezel.
            fill_circle(bg, x + w / 2.0, lid_y + i.top * 0.5, (b * 0.28).max(1.5), [70.0, 72.0, 80.0], 1.0);
            // The base: wider than the lid, thin, lighter, with a thumb groove.
            let base_h = i.bottom - b;
            let ext = (short * 0.03).max(12.0);
            let by = lid_y + lid_h - 1.0;
            fill_round_rect(bg, lid_x - ext, by, lid_w + 2.0 * ext, base_h + 1.0, base_h * 0.45, [201.0, 204.0, 210.0], 1.0);
            // Square off the top of the base where it meets the lid.
            fill_round_rect(bg, lid_x - ext, by, lid_w + 2.0 * ext, base_h * 0.5, 0.0, [201.0, 204.0, 210.0], 1.0);
            let gw = lid_w * 0.14;
            fill_round_rect(bg, lid_x + (lid_w - gw) / 2.0, by, gw, base_h * 0.45, base_h * 0.22, [168.0, 171.0, 178.0], 1.0);
        }
        DeviceFrame::Phone => {
            let (ox, oy, ow, oh, r) = outer(frame, screen, short);
            // A thin lighter rim, then the dark body.
            fill_round_rect(bg, ox - 1.5, oy - 1.5, ow + 3.0, oh + 3.0, r + 1.5, [92.0, 94.0, 102.0], 1.0);
            fill_round_rect(bg, ox, oy, ow, oh, r, [20.0, 20.0, 24.0], 1.0);
            // Speaker slot and camera dot in the top bezel.
            let cy = oy + i.top * 0.5;
            let sw = ow * 0.22;
            let sh = (i.top * 0.14).max(3.0);
            fill_round_rect(bg, ox + (ow - sw) / 2.0, cy - sh / 2.0, sw, sh, sh / 2.0, [58.0, 60.0, 68.0], 1.0);
            fill_circle(bg, ox + (ow + sw) / 2.0 + sh * 2.2, cy, sh * 0.55, [48.0, 50.0, 60.0], 1.0);
            // Side button.
            let bw = (i.left * 0.35).max(2.0);
            fill_round_rect(bg, ox + ow + 0.5, oy + oh * 0.28, bw, oh * 0.1, bw / 2.0, [60.0, 62.0, 70.0], 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_takes_room_and_none_takes_none() {
        assert_eq!(insets(DeviceFrame::None, 1000.0), Insets::default());
        for f in [DeviceFrame::Laptop, DeviceFrame::Browser, DeviceFrame::Phone] {
            let i = insets(f, 1080.0);
            assert!(i.height() > 10.0, "{f:?} {i:?}");
            assert!(i.left >= 0.0 && i.top > 0.0 && i.right >= 0.0 && i.bottom >= 0.0);
        }
        assert_eq!(DeviceFrame::parse("MacBook"), DeviceFrame::Laptop);
        assert_eq!(DeviceFrame::parse("iphone"), DeviceFrame::Phone);
        assert_eq!(DeviceFrame::parse("nope"), DeviceFrame::None);
    }
}
