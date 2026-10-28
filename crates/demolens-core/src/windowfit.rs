//! Where the editor window goes on a given screen. The window is kept inside
//! the usable part of the monitor (the work area: no menu bar, no Dock or
//! taskbar), so nothing is cut off at the right or bottom on a laptop.

/// Never smaller than this when the screen has room for it.
pub const MIN_W: f64 = 1024.0;
pub const MIN_H: f64 = 640.0;
/// At most this share of the work area, so there is always a margin.
pub const MAX_SHARE: f64 = 0.9;

/// A window rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The size to use: `wanted` (a saved size, or the default) limited to 90% of
/// the work area, raised to 1024x640, and never larger than the work area.
pub fn fit_size(wanted: (f64, f64), work_w: f64, work_h: f64) -> (f64, f64) {
    let clamp = |want: f64, min: f64, avail: f64| {
        let max = (avail * MAX_SHARE).floor();
        want.min(max).max(min).min(avail.floor()).max(1.0).round()
    };
    (clamp(wanted.0, MIN_W, work_w), clamp(wanted.1, MIN_H, work_h))
}

/// The window rectangle: the fitted size, centred in the work area
/// (`work` is x, y, w, h in logical pixels).
pub fn fit_window(wanted: (f64, f64), work: Rect) -> Rect {
    let (w, h) = fit_size(wanted, work.w, work.h);
    Rect { x: (work.x + (work.w - w) / 2.0).round(), y: (work.y + (work.h - h) / 2.0).round(), w, h }
}

/// The smallest size the user may drag the window to on this screen.
pub fn min_size(work_w: f64, work_h: f64) -> (f64, f64) {
    (MIN_W.min(work_w.floor()), MIN_H.min(work_h.floor()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(w: f64, h: f64) -> Rect {
        Rect { x: 0.0, y: 25.0, w, h }
    }

    #[test]
    fn the_default_window_fits_a_macbook_air_work_area() {
        // M-series Air: about 1470x956 logical, minus the menu bar and Dock.
        let r = fit_window((1360.0, 860.0), work(1470.0, 880.0));
        assert!(r.w <= 1470.0 * 0.9 && r.h <= 880.0 * 0.9, "{r:?}");
        assert!(r.x >= 0.0 && r.x + r.w <= 1470.0 && r.y >= 25.0 && r.y + r.h <= 25.0 + 880.0, "{r:?}");
        assert_eq!(r.x, ((1470.0 - r.w) / 2.0).round());
    }

    #[test]
    fn a_saved_size_from_a_bigger_screen_is_clamped() {
        let (w, h) = fit_size((2400.0, 1500.0), 1470.0, 880.0);
        assert_eq!((w, h), (1323.0, 792.0));
    }

    #[test]
    fn small_requests_are_raised_to_the_minimum() {
        assert_eq!(fit_size((300.0, 200.0), 1920.0, 1040.0), (1024.0, 640.0));
    }

    #[test]
    fn a_screen_smaller_than_the_minimum_still_holds_the_window() {
        // 1024x600 netbook: the window is the work area, not off screen.
        let r = fit_window((1360.0, 860.0), work(1024.0, 560.0));
        assert!(r.w <= 1024.0 && r.h <= 560.0, "{r:?}");
        assert_eq!(min_size(1024.0, 560.0), (1024.0, 560.0));
    }

    #[test]
    fn four_common_screens_keep_the_window_inside() {
        for (w, h) in [(1366.0, 728.0), (1440.0, 860.0), (1470.0, 880.0), (1920.0, 1040.0)] {
            let r = fit_window((1360.0, 860.0), work(w, h));
            assert!(r.x >= 0.0 && r.y >= 25.0 && r.x + r.w <= w && r.y + r.h <= 25.0 + h, "{w}x{h}: {r:?}");
            assert!(r.w >= 1024.0 || r.w == w.floor(), "{r:?}");
        }
    }
}
