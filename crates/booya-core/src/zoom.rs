//! Auto zoom: turn an event log into editable zoom segments, then evaluate a
//! smooth camera (center + scale) at any point in time.

use crate::events::{CursorTrack, EventLog};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomSegment {
    /// Source time in ms where the zoom starts easing in.
    pub start_ms: f64,
    /// Source time in ms where the zoom has fully eased out.
    pub end_ms: f64,
    /// Zoom factor, 1.0 = no zoom.
    pub scale: f64,
    /// When true the camera follows the (smoothed) cursor. When false it
    /// stays on `focus`.
    pub follow_cursor: bool,
    /// Fixed focus point in source pixels, used when not following.
    pub focus: (f64, f64),
    /// What produced the segment ("click", "typing", "manual").
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoomConfig {
    /// Clicks that keep coming in the same area, each within this gap of the
    /// one before, stay in the zoom that is already running.
    pub merge_gap_ms: f64,
    /// How early the zoom starts before the first click of a group. A little
    /// longer than `ease_in_ms`, so the camera has arrived when the click lands.
    pub lead_in_ms: f64,
    /// How long the zoom holds at full level after the last click or key.
    pub hold_ms: f64,
    /// Minimum total length of a segment (ease in + hold + ease out).
    pub min_duration_ms: f64,
    pub click_scale: f64,
    pub typing_scale: f64,
    /// Keys within this gap count as one typing burst.
    pub typing_gap_ms: f64,
    /// Minimum keys in a burst before it can trigger a zoom.
    pub typing_min_keys: usize,
    /// A typing burst has to last this long to trigger a zoom.
    pub typing_min_ms: f64,
    /// Ease in duration used when evaluating the camera.
    pub ease_in_ms: f64,
    /// Ease out duration used when evaluating the camera.
    pub ease_out_ms: f64,
    /// How many clicks in one small area, inside `cluster_window_ms`, it takes
    /// to zoom. The one setting that decides how often the camera moves.
    pub min_clicks: usize,
    /// The clicks of a cluster have to fall within this long.
    pub cluster_window_ms: f64,
    /// "A small area": the box round a cluster covers at most this share of
    /// the screen (0.22 is a little under half the width and half the height).
    pub cluster_area: f64,
    /// No zoom starts before this, so a recording opens on the whole screen.
    pub intro_ms: f64,
    /// The whole screen stays up at least this long between two zooms.
    pub min_wide_ms: f64,
    /// After scrolling stops the camera stays wide this much longer.
    pub scroll_hold_ms: f64,
    pub intensity: Intensity,
    /// Share of the zoomed frame kept clear around the clicked points, so
    /// nothing a viewer is meant to look at sits on the edge.
    pub bleed: f64,
    /// The deepest zoom ever generated or accepted from the level setting.
    pub max_scale: f64,
}

impl Default for ZoomConfig {
    fn default() -> Self {
        Self {
            merge_gap_ms: 3000.0,
            lead_in_ms: 600.0,
            hold_ms: 2500.0,
            min_duration_ms: 900.0 + 900.0 + 2500.0 + 100.0,
            click_scale: 1.6,
            typing_scale: 1.5,
            typing_gap_ms: 1200.0,
            typing_min_keys: 5,
            typing_min_ms: 1500.0,
            ease_in_ms: 900.0,
            ease_out_ms: 900.0,
            min_clicks: 3,
            cluster_window_ms: 4500.0,
            cluster_area: 0.22,
            intro_ms: 1000.0,
            min_wide_ms: 1500.0,
            scroll_hold_ms: 800.0,
            intensity: Intensity::Normal,
            bleed: 0.17,
            max_scale: 2.0,
        }
    }
}

/// How eagerly auto zoom moves the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Intensity {
    /// Four clicks in one area (or typing, or a marker).
    Subtle,
    #[default]
    Normal,
    /// Two clicks in one area.
    Lively,
}

impl Intensity {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "subtle" => Self::Subtle,
            "lively" => Self::Lively,
            _ => Self::Normal,
        }
    }
}

impl ZoomConfig {
    /// The rules for one intensity. Normal is the default; Subtle is stricter
    /// and Lively is looser.
    pub fn with_intensity(mut self, i: Intensity) -> Self {
        let (clicks, hold, level) = match i {
            Intensity::Subtle => (4, 2000.0, 1.5),
            Intensity::Normal => (3, 2500.0, 1.6),
            Intensity::Lively => (2, 3000.0, 1.8),
        };
        self.intensity = i;
        self.min_clicks = clicks;
        self.hold_ms = hold;
        self.min_duration_ms = self.ease_in_ms + self.ease_out_ms + self.hold_ms + 100.0;
        self.with_level(level)
    }

    /// How many clicks in one small area it takes to zoom (1 to 8), and how
    /// long they may take (1 to 15 s).
    pub fn with_clicks(mut self, min_clicks: usize, window_ms: f64) -> Self {
        self.min_clicks = min_clicks.clamp(1, 8);
        self.cluster_window_ms = window_ms.clamp(1000.0, 15000.0);
        self
    }

    /// Total of ease in, a hold and ease out: the shortest sensible segment.
    pub fn with_speed(mut self, ease_in_ms: f64, ease_out_ms: f64) -> Self {
        self.ease_in_ms = ease_in_ms.clamp(150.0, 3000.0);
        self.ease_out_ms = ease_out_ms.clamp(150.0, 3000.0);
        self.lead_in_ms = self.ease_in_ms + 100.0;
        self.min_duration_ms = self.ease_in_ms + self.ease_out_ms + 1700.0;
        self
    }
    /// How long a zoom holds after the last event, and how close events must be
    /// to share one zoom. Keeps the minimum segment length consistent.
    pub fn with_timing(mut self, hold_ms: f64, merge_gap_ms: f64) -> Self {
        self.hold_ms = hold_ms.clamp(400.0, 8000.0);
        self.merge_gap_ms = merge_gap_ms.clamp(500.0, 10000.0);
        self.min_duration_ms = self.ease_in_ms + self.ease_out_ms + self.hold_ms + 100.0;
        self
    }
    pub fn with_level(mut self, level: f64) -> Self {
        let level = level.clamp(1.1, self.max_scale);
        self.click_scale = level;
        self.typing_scale = (level - 0.1).max(1.1);
        self
    }
}

/// The deepest zoom at which every point stays at least `bleed` of the zoomed
/// frame away from its edges, never more than `base`. Points near the edge of
/// the screen cannot be given that margin (the camera may not leave the screen),
/// so those only get a gentle zoom instead of a tight one.
pub fn bleed_scale(points: &[(f64, f64)], w: f64, h: f64, bleed: f64, base: f64) -> f64 {
    if points.is_empty() || w <= 0.0 || h <= 0.0 {
        return base;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in points {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    let room = (1.0 - 2.0 * bleed).max(0.2);
    let fit_x = if x1 - x0 > 1.0 { w * room / (x1 - x0) } else { f64::MAX };
    let fit_y = if y1 - y0 > 1.0 { h * room / (y1 - y0) } else { f64::MAX };
    let mut s = base.min(fit_x).min(fit_y);
    let edge = points.iter().map(|&(x, y)| (x / w).min(1.0 - x / w).min(y / h).min(1.0 - y / h)).fold(f64::MAX, f64::min);
    if edge < 0.05 {
        s = s.min(1.25);
    }
    s.max(1.0)
}

/// Keys that switch tab or window. What follows them is navigation.
fn is_navigation_shortcut(key: &str, mods: &[String]) -> bool {
    let strong = mods.iter().any(|m| matches!(m.to_ascii_lowercase().as_str(), "ctrl" | "alt" | "meta" | "cmd" | "win" | "super" | "command" | "option"));
    strong && matches!(key.to_ascii_lowercase().as_str(), "tab" | "`" | "~" | "pageup" | "pagedown" | "t" | "w" | "n" | "left" | "right" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
}

/// Cursor position over time where a long silence means "it stayed put" (the
/// log only holds moves, so a gap is a still cursor), not a slow glide.
struct Held {
    s: Vec<(f64, f64, f64)>,
}

impl Held {
    fn new(log: &EventLog) -> Self {
        Self { s: log.cursor_samples() }
    }
    fn at(&self, t: f64) -> Option<(f64, f64)> {
        let s = &self.s;
        if s.is_empty() {
            return None;
        }
        let i = s.partition_point(|p| p.0 <= t);
        if i == 0 {
            return Some((s[0].1, s[0].2));
        }
        let a = s[i - 1];
        if i >= s.len() || s[i].0 - a.0 > 250.0 {
            return Some((a.1, a.2));
        }
        let b = s[i];
        let k = (t - a.0) / (b.0 - a.0).max(1e-6);
        Some((a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k))
    }
}

/// A click that may count towards a zoom.
#[derive(Debug, Clone, Copy)]
struct Click {
    t: f64,
    x: f64,
    y: f64,
}

/// A stretch of scrolling: from its first event to its last, plus the hold
/// after it. The camera is wide inside it.
fn scroll_spans(log: &EventLog, hold_ms: f64) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = Vec::new();
    // Wheel events 450 ms apart or less are one scroll.
    let mut start = None::<f64>;
    let mut last = 0.0;
    for t in log.scrolls() {
        match start {
            Some(_) if t - last <= 450.0 => {}
            Some(s) => {
                spans.push((s, last + hold_ms));
                start = Some(t);
            }
            None => start = Some(t),
        }
        last = t;
    }
    if let Some(s) = start {
        spans.push((s, last + hold_ms));
    }
    spans
}

/// The clicks that can count towards a zoom. Everything that is not a plain
/// aimed press of the main button is left out: right and middle clicks, the
/// second click of a double click, the menu bar, the Dock or taskbar, clicks
/// that start a drag (moving a window, selecting), clicks while scrolling,
/// clicks right after a tab or window switch, the click that stops the
/// recording.
fn usable_clicks(log: &EventLog, held: &Held, duration_ms: f64, scrolls: &[(f64, f64)]) -> Vec<Click> {
    use crate::events::{EventKind, MouseButton};
    let (fw, fh) = (log.header.width as f64, log.header.height as f64);
    let diag = fw.hypot(fh).max(1.0);
    let dist = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    let nav_until: Vec<(f64, f64)> = log.keys().filter(|(_, k, m)| is_navigation_shortcut(k, m)).map(|(t, _, _)| (t, t + 1200.0)).collect();
    let after_nav = |t: f64| nav_until.iter().any(|&(a, b)| t >= a && t <= b);
    let navigation_area = |x: f64, y: f64| y < fh * 0.05 || y > fh * 0.94 || x < fw * 0.02 || x > fw * 0.98;
    let releases: Vec<(f64, f64, f64, MouseButton)> = log
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Release { x, y, button } => Some((e.t, x, y, button)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    let mut last_click = f64::MIN;
    for e in &log.events {
        let EventKind::Click { x, y, button } = e.kind else { continue };
        let t = e.t;
        let quick_repeat = t - last_click < 350.0;
        last_click = t;
        if !matches!(button, MouseButton::Left | MouseButton::Touch) || quick_repeat {
            continue;
        }
        if navigation_area(x, y) || after_nav(t) || scrolls.iter().any(|&(a, b)| t >= a - 300.0 && t <= b) {
            continue;
        }
        if duration_ms > 0.0 && t > duration_ms - 1000.0 {
            continue; // the click that ends the recording
        }
        // A drag or a window being moved: the button stays down while the cursor travels.
        let dragged = releases.iter().find(|r| r.0 >= t && r.3 == button).is_some_and(|r| r.0 - t > 400.0 && dist((r.1, r.2), (x, y)) > diag * 0.03);
        if dragged {
            continue;
        }
        // Straight after a big jump of the cursor the viewer is still finding it.
        if dist(held.at(t - 250.0).unwrap_or((x, y)), (x, y)) > diag * 0.4 {
            continue;
        }
        out.push(Click { t, x, y });
    }
    out
}

/// Whether points fit in "a small area": the box round them covers at most
/// `area` of the screen, and is no wider or taller than 0.6 of it.
fn fits_area(pts: &[(f64, f64)], fw: f64, fh: f64, area: f64) -> bool {
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    let (w, h) = ((x1 - x0) / fw.max(1.0), (y1 - y0) / fh.max(1.0));
    w <= 0.6 && h <= 0.6 && w * h <= area
}

/// Something worth a zoom: where, when it starts and ends, how many clicks.
#[derive(Debug, Clone)]
struct Moment {
    first: f64,
    last: f64,
    pts: Vec<(f64, f64)>,
    reason: &'static str,
    weight: usize,
}

/// The cluster rule. A zoom needs at least `min_clicks` clicks inside one
/// small area (see [`fits_area`]) within `cluster_window_ms`. Once there is
/// one, further clicks that keep coming in the same area (each within
/// `merge_gap_ms` of the one before) stay in it.
fn click_moments(clicks: &[Click], fw: f64, fh: f64, cfg: &ZoomConfig) -> Vec<Moment> {
    let need = cfg.min_clicks.max(1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < clicks.len() {
        let mut j = i;
        let mut pts = vec![(clicks[i].x, clicks[i].y)];
        while j + 1 < clicks.len() && clicks[j + 1].t - clicks[i].t <= cfg.cluster_window_ms {
            pts.push((clicks[j + 1].x, clicks[j + 1].y));
            if !fits_area(&pts, fw, fh, cfg.cluster_area) {
                pts.pop();
                break;
            }
            j += 1;
        }
        if j - i + 1 < need {
            i += 1;
            continue;
        }
        // Keep going while the clicking goes on in the same place.
        while j + 1 < clicks.len() && clicks[j + 1].t - clicks[j].t <= cfg.merge_gap_ms {
            pts.push((clicks[j + 1].x, clicks[j + 1].y));
            if !fits_area(&pts, fw, fh, cfg.cluster_area) {
                pts.pop();
                break;
            }
            j += 1;
        }
        out.push(Moment { first: clicks[i].t, last: clicks[j].t, pts, reason: "click", weight: j - i + 1 });
        i = j + 1;
    }
    out
}

/// Typing in one place for longer than `typing_min_ms` (with at least
/// `typing_min_keys` keys, gaps under `typing_gap_ms`). It zooms where the
/// last click was just before it (the field), else where the cursor is.
fn typing_moments(log: &EventLog, held: &Held, clicks: &[Click], cfg: &ZoomConfig) -> Vec<Moment> {
    let (fw, fh) = (log.header.width as f64, log.header.height as f64);
    let keys: Vec<f64> = log.keys().filter(|(_, k, m)| m.iter().all(|m| m.eq_ignore_ascii_case("shift")) && !matches!(k.to_ascii_lowercase().as_str(), "tab" | "escape" | "esc" | "left" | "right" | "up" | "down" | "pageup" | "pagedown" | "home" | "end")).map(|(t, _, _)| t).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < keys.len() {
        let mut j = i;
        while j + 1 < keys.len() && keys[j + 1] - keys[j] <= cfg.typing_gap_ms {
            j += 1;
        }
        if j - i + 1 >= cfg.typing_min_keys && keys[j] - keys[i] >= cfg.typing_min_ms {
            let start = keys[i];
            let at = clicks.iter().rev().find(|c| c.t <= start && start - c.t <= 4000.0).map(|c| (c.x, c.y)).or_else(|| held.at(start)).unwrap_or((fw / 2.0, fh / 2.0));
            out.push(Moment { first: start, last: keys[j], pts: vec![at], reason: "typing", weight: j - i + 1 });
        }
        i = j + 1;
    }
    out
}

/// Build zoom segments. The camera stays on the whole screen unless there is
/// a reason, and the only reasons are:
///
///  * `min_clicks` clicks in one small area within a few seconds (a person
///    working on one spot),
///  * typing into a field for longer than a second and a half,
///  * a marker the presenter dropped on purpose.
///
/// Nothing else zooms: not the cursor resting, not the picture changing, not
/// a single click. A zoom never starts in the first second, is dropped when
/// the viewer would not get time to see it, and is cut short (eased back to
/// the full screen) when scrolling starts, a window is dragged, or the cursor
/// jumps across the screen.
pub fn generate_zoom_segments(log: &EventLog, duration_ms: f64, cfg: &ZoomConfig) -> Vec<ZoomSegment> {
    let held = Held::new(log);
    let (fw, fh) = (log.header.width as f64, log.header.height as f64);
    let scrolls = scroll_spans(log, cfg.scroll_hold_ms);
    let clicks = usable_clicks(log, &held, duration_ms, &scrolls);

    let mut moments = click_moments(&clicks, fw, fh, cfg);
    moments.extend(typing_moments(log, &held, &clicks, cfg).into_iter().filter(|m| !scrolls.iter().any(|&(a, b)| m.first >= a && m.first <= b)));
    moments.extend(log.marks().map(|(t, x, y)| Moment { first: t, last: t, pts: vec![(x, y)], reason: "marker", weight: cfg.min_clicks.max(1) + 1 }));
    moments.sort_by(|a, b| a.first.total_cmp(&b.first));

    let mut segs: Vec<(ZoomSegment, usize, Vec<(f64, f64)>)> = Vec::new();
    for m in moments {
        let mut start = (m.first - cfg.lead_in_ms).max(cfg.intro_ms);
        let mut end = m.last + cfg.hold_ms + cfg.ease_out_ms;
        if end - start < cfg.min_duration_ms {
            end = start + cfg.min_duration_ms;
        }
        if duration_ms > 0.0 {
            end = end.min(duration_ms);
            if end - start < cfg.min_duration_ms {
                start = (end - cfg.min_duration_ms).max(cfg.intro_ms);
            }
        }
        if end - start < cfg.min_duration_ms * 0.9 {
            continue;
        }
        let n = m.pts.len() as f64;
        let focus = (m.pts.iter().map(|p| p.0).sum::<f64>() / n, m.pts.iter().map(|p| p.1).sum::<f64>() / n);
        let base = if m.reason == "typing" { cfg.typing_scale } else { cfg.click_scale }.min(cfg.max_scale);
        // Never tighter than leaves a clear margin around everything clicked.
        let scale = bleed_scale(&m.pts, fw, fh, cfg.bleed, base);
        if scale < 1.12 {
            continue;
        }
        segs.push((ZoomSegment { start_ms: start, end_ms: end, scale, follow_cursor: true, focus, reason: m.reason.into() }, m.weight, m.pts));
    }

    // Scrolling and a cursor that jumps across the screen end a zoom early.
    let jump_after = |from: f64, to: f64| -> Option<f64> {
        let mut t = from + 300.0;
        while t < to {
            if let (Some(a), Some(b)) = (held.at(t - 300.0), held.at(t)) {
                if (a.0 - b.0).hypot(a.1 - b.1) > fw.hypot(fh) * 0.45 {
                    return Some(t - 300.0);
                }
            }
            t += 100.0;
        }
        None
    };
    let mut cut: Vec<(ZoomSegment, usize, Vec<(f64, f64)>)> = Vec::new();
    for (mut s, w, pts) in segs {
        if let Some(j) = jump_after(s.start_ms + cfg.ease_in_ms, s.end_ms - cfg.ease_out_ms) {
            s.end_ms = s.end_ms.min(j + cfg.ease_out_ms);
        }
        let mut pieces = vec![s];
        for &(a, b) in &scrolls {
            let mut next = Vec::new();
            for p in pieces {
                if b <= p.start_ms || a >= p.end_ms {
                    next.push(p);
                    continue;
                }
                // Before the scroll: ease out as it starts. After it: only if there is still room.
                if a > p.start_ms + cfg.ease_in_ms {
                    next.push(ZoomSegment { end_ms: a + cfg.ease_out_ms, ..p.clone() });
                }
                if p.end_ms - b >= cfg.min_duration_ms {
                    next.push(ZoomSegment { start_ms: b, ..p.clone() });
                }
            }
            pieces = next;
        }
        for p in pieces {
            if p.end_ms - p.start_ms >= cfg.min_duration_ms * 0.9 {
                cut.push((p, w, pts.clone()));
            }
        }
    }

    // The whole screen shows for at least `min_wide_ms` between two zooms.
    // Zooms closer than that become one (the camera pans) when what they
    // look at still fits one view with a margin; otherwise the one with more
    // behind it stays.
    let mut kept: Vec<(ZoomSegment, usize, Vec<(f64, f64)>)> = Vec::new();
    for (s, w, pts) in cut {
        if let Some((p, pw, ppts)) = kept.last_mut() {
            if s.start_ms < p.end_ms + cfg.min_wide_ms {
                let mut both = ppts.clone();
                both.extend(pts.iter().copied());
                let fit = bleed_scale(&both, fw, fh, cfg.bleed, cfg.max_scale);
                if fit >= 1.2 && s.start_ms - p.end_ms < cfg.min_wide_ms {
                    p.end_ms = p.end_ms.max(s.end_ms);
                    p.scale = fit.min(p.scale.max(s.scale));
                    let n = both.len() as f64;
                    p.focus = (both.iter().map(|q| q.0).sum::<f64>() / n, both.iter().map(|q| q.1).sum::<f64>() / n);
                    if p.reason != s.reason {
                        p.reason = "click".into();
                    }
                    *pw += w;
                    *ppts = both;
                } else if w > *pw {
                    *p = s;
                    *pw = w;
                    *ppts = pts;
                }
                continue;
            }
        }
        kept.push((s, w, pts));
    }
    kept.into_iter().map(|(s, _, _)| s).collect()
}

/// Critically damped spring response from 0 to 1: it leaves at once, gathers
/// speed gently and settles softly with no overshoot. `t` is 0..1 of the move.
pub fn spring(t: f64) -> f64 {
    const K: f64 = 4.0;
    let t = t.clamp(0.0, 1.0);
    let f = |x: f64| 1.0 - (1.0 + K * x) * (-K * x).exp();
    f(t) / f(1.0)
}

/// Smooth ease in/out (smootherstep). Its fastest moment is about 1.9 times
/// the average speed, against 3 times for a cubic, so a zoom never lurches.
pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Camera state: the source rectangle shown in the output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
}

impl Camera {
    /// Visible source rectangle (x, y, w, h) for a source of size (w, h).
    pub fn view_rect(&self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let vw = w / self.scale;
        let vh = h / self.scale;
        (self.cx - vw / 2.0, self.cy - vh / 2.0, vw, vh)
    }
}

/// Clamp a center so the zoomed viewport stays inside the source frame.
pub fn clamp_center(cx: f64, cy: f64, scale: f64, w: f64, h: f64) -> (f64, f64) {
    let hw = w / scale / 2.0;
    let hh = h / scale / 2.0;
    (cx.clamp(hw, w - hw), cy.clamp(hh, h - hh))
}

/// Timing of the camera easing, in ms.
#[derive(Debug, Clone, Copy)]
pub struct CameraTiming {
    pub ease_in_ms: f64,
    pub ease_out_ms: f64,
    pub follow_smoothing_ms: f64,
    /// Fraction of the visible half-size the cursor may roam before the camera
    /// starts to follow. 0 follows every move; 0.4 keeps a calm, still frame.
    pub deadzone: f64,
}

/// How far ahead of a zoom's start the camera aims, so it arrives at the action.
const ANTICIPATION_MS: f64 = 1000.0;
/// Step of the camera simulation. Small enough to be smooth, large enough to be cheap.
const SIM_STEP_MS: f64 = 40.0;

impl CameraTiming {
    pub fn new(ease_in_ms: f64, ease_out_ms: f64, follow_smoothing_ms: f64) -> Self {
        Self { ease_in_ms, ease_out_ms, follow_smoothing_ms, deadzone: 0.4 }
    }
}

/// Where the zoomed camera aims at time `t`.
///
/// The cursor can wander inside a dead zone without moving the camera; once it
/// pushes past the edge the camera is pulled along, eased so it never snaps.
/// This is what makes the footage feel steady instead of seasick. Starting
/// from where the cursor will be when the action begins means the zoom lands
/// on the content rather than on wherever the pointer happened to be.
fn follow_target(s: &ZoomSegment, tr: &CursorTrack, t: f64, timing: &CameraTiming, w: f64, to_track: &dyn Fn(f64) -> f64) -> (f64, f64) {
    let smooth = timing.follow_smoothing_ms;
    let at = |x: f64| tr.smooth_at(to_track(x), smooth);
    let Some(mut pos) = at((s.start_ms + ANTICIPATION_MS).min(s.end_ms)).or(Some(s.focus)) else { return s.focus };
    if timing.deadzone <= 0.0 {
        return at(t).unwrap_or(pos);
    }
    let half = w / (2.0 * s.scale.max(1.0)) * timing.deadzone;
    let tau = (smooth * 0.9).max(60.0);
    let k = 1.0 - (-SIM_STEP_MS / tau).exp();
    let mut x = s.start_ms;
    while x < t {
        let step = SIM_STEP_MS.min(t - x);
        x += step;
        if let Some(c) = at(x) {
            // The point on the dead-zone border nearest the cursor is where the camera wants to be.
            let want = |p: f64, c: f64| if (c - p).abs() > half { c - (c - p).signum() * half } else { p };
            let kk = if step < SIM_STEP_MS { 1.0 - (-step / tau).exp() } else { k };
            pos.0 += (want(pos.0, c.0) - pos.0) * kk;
            pos.1 += (want(pos.1, c.1) - pos.1) * kk;
        }
    }
    pos
}

/// Evaluate the camera at source time `t`.
///
/// The zoom amount eases in over `ease_in_ms` at the start of a segment and
/// eases out over `ease_out_ms` at the end. The level is interpolated on a log
/// scale so the zoom feels even, and the target point follows the smoothed
/// cursor (or the fixed focus) and blends toward the frame center as the zoom
/// fades, so zooming in and out never jumps.
pub fn camera_at(t: f64, segments: &[ZoomSegment], track: Option<&CursorTrack>, w: f64, h: f64, timing: CameraTiming) -> Camera {
    camera_at_mapped(t, segments, track, w, h, timing, &|x| x)
}

/// Like [`camera_at`], for segments and a time `t` on another clock than the
/// cursor track (the exported timeline, after cuts). `to_track` turns a time
/// on the segments' clock into a time on the track's clock.
pub fn camera_at_mapped(t: f64, segments: &[ZoomSegment], track: Option<&CursorTrack>, w: f64, h: f64, timing: CameraTiming, to_track: &dyn Fn(f64) -> f64) -> Camera {
    let center = (w / 2.0, h / 2.0);
    let seg = segments.iter().find(|s| t >= s.start_ms && t <= s.end_ms);
    let Some(s) = seg else {
        return Camera { cx: center.0, cy: center.1, scale: 1.0 };
    };
    let len = (s.end_ms - s.start_ms).max(1.0);
    let (mut ein, mut eout) = (timing.ease_in_ms.max(1.0), timing.ease_out_ms.max(1.0));
    if ein + eout > len {
        let f = len / (ein + eout);
        ein *= f;
        eout *= f;
    }
    let k_in = spring((t - s.start_ms) / ein);
    let k_out = spring((s.end_ms - t) / eout);
    let k = k_in.min(k_out);
    let scale = s.scale.max(1.0).powf(k);

    let target = if s.follow_cursor {
        track.map(|tr| follow_target(s, tr, t, &timing, w, to_track)).unwrap_or(s.focus)
    } else {
        s.focus
    };
    // Blend from frame center to target as k goes 0 -> 1.
    let tx = center.0 + (target.0 - center.0) * k;
    let ty = center.1 + (target.1 - center.1) * k;
    let (cx, cy) = clamp_center(tx, ty, scale, w, h);
    Camera { cx, cy, scale }
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: CameraTiming = CameraTiming { ease_in_ms: 400.0, ease_out_ms: 400.0, follow_smoothing_ms: 0.0, deadzone: 0.0 };
    const CALM: CameraTiming = CameraTiming { ease_in_ms: 400.0, ease_out_ms: 400.0, follow_smoothing_ms: 250.0, deadzone: 0.4 };
    use crate::events::{Event, EventKind, MouseButton};

    fn log_with(events: Vec<Event>) -> EventLog {
        let mut l = EventLog::new(1920, 1080);
        l.events = events;
        l
    }
    fn click(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Click { x, y, button: MouseButton::Left } }
    }
    fn key(t: f64, k: &str, mods: &[&str]) -> Event {
        Event { t, kind: EventKind::Key { key: k.into(), modifiers: mods.iter().map(|s| s.to_string()).collect() } }
    }
    fn mv(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Move { x, y } }
    }

    #[test]
    fn empty_log_has_no_zoom() {
        let segs = generate_zoom_segments(&log_with(vec![]), 10_000.0, &ZoomConfig::default());
        assert!(segs.is_empty());
    }

    #[test]
    fn a_marker_zooms_to_the_cursor_without_any_click() {
        let log = log_with(vec![mv(1000.0, 100.0, 100.0), Event { t: 5000.0, kind: EventKind::Mark { x: 1500.0, y: 800.0 } }]);
        let segs = generate_zoom_segments(&log, 12_000.0, &ZoomConfig::default());
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(segs[0].start_ms <= 5000.0 && segs[0].end_ms > 5000.0);
        assert!(log.clicks().next().is_none(), "a marker is not a click, so no ripple is drawn");
        assert_eq!(log.marks().count(), 1);
    }

    #[test]
    fn markers_survive_a_save_and_load() {
        let mut log = log_with(vec![Event { t: 10.0, kind: EventKind::Mark { x: 3.0, y: 4.0 } }]);
        log.events.push(mv(20.0, 1.0, 2.0));
        let path = std::env::temp_dir().join(format!("booya-mark-{}.jsonl", std::process::id()));
        log.save(&path).unwrap();
        let back = EventLog::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.marks().collect::<Vec<_>>(), vec![(10.0, 3.0, 4.0)]);
    }

    fn rel(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Release { x, y, button: MouseButton::Left } }
    }
    /// A straight glide from a to b ending at `t_end`, one sample every 50 ms.
    fn glide(t_end: f64, a: (f64, f64), b: (f64, f64), ms: f64) -> Vec<Event> {
        let n = (ms / 50.0) as usize;
        (0..=n).map(|i| mv(t_end - ms + i as f64 * 50.0, a.0 + (b.0 - a.0) * i as f64 / n as f64, a.1 + (b.1 - a.1) * i as f64 / n as f64)).collect()
    }

    // Clicks `gap` ms apart starting at `t0`, all near (x, y).
    fn burst(t0: f64, n: usize, gap: f64, x: f64, y: f64) -> Vec<Event> {
        (0..n).map(|i| click(t0 + i as f64 * gap, x + (i as f64 * 37.0) % 120.0, y + (i as f64 * 23.0) % 70.0)).collect()
    }
    fn gen(events: Vec<Event>) -> Vec<ZoomSegment> {
        generate_zoom_segments(&log_with(events), 60_000.0, &ZoomConfig::default())
    }

    #[test]
    fn three_clicks_in_one_small_area_within_a_few_seconds_zoom_once() {
        let cfg = ZoomConfig::default();
        let segs = gen(burst(5000.0, 3, 1000.0, 900.0, 500.0));
        assert_eq!(segs.len(), 1, "{segs:?}");
        let s = &segs[0];
        assert_eq!(s.start_ms, 5000.0 - cfg.lead_in_ms);
        assert_eq!(s.end_ms, 7000.0 + cfg.hold_ms + cfg.ease_out_ms);
        assert!(s.scale >= 1.12 && s.scale <= cfg.click_scale);
        assert_eq!(s.reason, "click");
        assert!(s.follow_cursor);
    }

    #[test]
    fn one_or_two_clicks_never_zoom_at_normal() {
        assert!(gen(vec![click(10_000.0, 900.0, 500.0)]).is_empty(), "the single Wi-Fi click");
        assert!(gen(burst(10_000.0, 2, 800.0, 900.0, 500.0)).is_empty());
        // Lively asks for two, Subtle for four.
        let two = log_with(burst(10_000.0, 2, 800.0, 900.0, 500.0));
        assert_eq!(generate_zoom_segments(&two, 60_000.0, &ZoomConfig::default().with_intensity(Intensity::Lively)).len(), 1);
        let three = log_with(burst(10_000.0, 3, 800.0, 900.0, 500.0));
        assert!(generate_zoom_segments(&three, 60_000.0, &ZoomConfig::default().with_intensity(Intensity::Subtle)).is_empty());
        assert_eq!(generate_zoom_segments(&three, 60_000.0, &ZoomConfig::default().with_clicks(3, 4500.0)).len(), 1);
        assert!(generate_zoom_segments(&three, 60_000.0, &ZoomConfig::default().with_clicks(4, 4500.0)).is_empty());
    }

    #[test]
    fn clicks_too_slow_or_too_far_apart_do_not_make_a_cluster() {
        // Three clicks spread over 9 s.
        assert!(gen(burst(10_000.0, 3, 4500.0, 900.0, 500.0)).is_empty());
        // Three clicks close in time but across the screen (not one small area).
        let spread = vec![click(10_000.0, 200.0, 200.0), click(10_800.0, 1700.0, 300.0), click(11_600.0, 900.0, 900.0)];
        assert!(gen(spread).is_empty());
        // Two near each other and one far away is still only two.
        let two_and_one = vec![click(10_000.0, 900.0, 500.0), click(10_800.0, 940.0, 520.0), click(11_600.0, 1750.0, 950.0)];
        assert!(gen(two_and_one).is_empty());
    }

    #[test]
    fn nothing_but_clicks_and_typing_ever_starts_a_zoom() {
        // The recording the user sent: the cursor sits still at the start, rests
        // after moving, a window slides in. None of that has a click.
        let mut ev = vec![mv(0.0, 900.0, 500.0), mv(400.0, 902.0, 501.0)];
        ev.extend(glide(9000.0, (200.0, 200.0), (1100.0, 600.0), 800.0));
        ev.push(mv(13_000.0, 1103.0, 602.0));
        ev.extend(glide(17_000.0, (1100.0, 600.0), (1500.0, 300.0), 500.0));
        ev.push(mv(19_800.0, 1502.0, 301.0));
        assert!(gen(ev.clone()).is_empty(), "movement alone must never zoom");
        // One click at the Wi-Fi icon among all that movement: still nothing.
        ev.push(click(10_000.0, 1100.0, 600.0));
        ev.sort_by(|a, b| a.t.total_cmp(&b.t));
        assert!(gen(ev).is_empty());
    }

    #[test]
    fn a_recording_never_starts_zoomed_in() {
        let segs = gen(burst(300.0, 4, 400.0, 900.0, 500.0));
        let cfg = ZoomConfig::default();
        assert!(segs.iter().all(|s| s.start_ms >= cfg.intro_ms), "{segs:?}");
    }

    #[test]
    fn typing_in_a_field_for_longer_than_a_second_and_a_half_zooms() {
        let cfg = ZoomConfig::default();
        let typing = |n: usize, gap: f64| -> Vec<Event> { std::iter::once(click(2500.0, 800.0, 400.0)).chain((0..n).map(|i| key(3000.0 + i as f64 * gap, "A", &[]))).collect() };
        // 12 keys over 1.65 s.
        let segs = gen(typing(12, 150.0));
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!(segs[0].reason, "typing");
        assert_eq!(segs[0].scale, cfg.typing_scale.min(bleed_scale(&[(800.0, 400.0)], 1920.0, 1080.0, cfg.bleed, cfg.typing_scale)));
        // A short burst, and a lone key, do not.
        assert!(gen(typing(6, 150.0)).is_empty());
        assert!(gen(vec![mv(0.0, 10.0, 10.0), key(3000.0, "A", &[])]).is_empty());
        // Tab, Enter and arrows are not typing in a field.
        let nav: Vec<Event> = (0..12).map(|i| key(3000.0 + i as f64 * 200.0, "Tab", &[])).collect();
        assert!(gen(nav).is_empty());
    }

    #[test]
    fn scrolling_eases_out_and_keeps_the_screen_wide_until_it_stops_plus_a_hold() {
        let cfg = ZoomConfig::default();
        let scroll = |t: f64| Event { t, kind: EventKind::Scroll { x: 900.0, y: 500.0, dx: 0.0, dy: -3.0 } };
        let mut ev = burst(5000.0, 3, 700.0, 900.0, 500.0);
        // The zoom runs from 4.4 s to 10.9 s. Scrolling from 8.0 s to 9.0 s.
        ev.extend((0..11).map(|i| scroll(8000.0 + i as f64 * 100.0)));
        ev.sort_by(|a, b| a.t.total_cmp(&b.t));
        let log = log_with(ev);
        let segs = generate_zoom_segments(&log, 60_000.0, &cfg);
        // The zoom ends as scrolling starts (plus the ease out) ...
        assert!(!segs.is_empty());
        assert!(segs[0].end_ms <= 8000.0 + cfg.ease_out_ms + 1.0, "{segs:?}");
        // ... and nothing is zoomed until 0.8 s after the last wheel event, and what is left is too short to bother.
        for s in &segs {
            assert!(s.start_ms >= 9000.0 + cfg.scroll_hold_ms || s.end_ms <= 8000.0 + cfg.ease_out_ms + 1.0, "{segs:?}");
        }
        // The camera really is at 1x through the scroll and the hold.
        let track = CursorTrack::new(&log);
        let timing = CameraTiming { ease_in_ms: cfg.ease_in_ms, ease_out_ms: cfg.ease_out_ms, follow_smoothing_ms: 0.0, deadzone: 0.4 };
        for t in [9000.0, 9400.0, 9700.0] {
            let c = camera_at(t, &segs, Some(&track), 1920.0, 1080.0, timing);
            assert_eq!(c.scale, 1.0, "wide at {t}");
        }
    }

    #[test]
    fn clicks_made_while_scrolling_do_not_count() {
        let scroll = |t: f64| Event { t, kind: EventKind::Scroll { x: 900.0, y: 500.0, dx: 0.0, dy: -3.0 } };
        let mut ev: Vec<Event> = (0..40).map(|i| scroll(10_000.0 + i as f64 * 100.0)).collect();
        ev.extend(burst(10_500.0, 3, 700.0, 900.0, 500.0));
        ev.sort_by(|a, b| a.t.total_cmp(&b.t));
        assert!(gen(ev).is_empty());
    }

    #[test]
    fn dragging_a_window_and_big_cursor_jumps_are_not_clicks_to_zoom_on() {
        // Three presses that are all drags.
        let mut drags = Vec::new();
        for i in 0..3 {
            let t = 10_000.0 + i as f64 * 1500.0;
            drags.push(click(t, 600.0, 500.0));
            drags.push(rel(t + 900.0, 1200.0, 700.0));
        }
        assert!(gen(drags).is_empty());
        // The Dock, the menu bar and a right click are not targets either.
        let nav = vec![click(10_000.0, 900.0, 1070.0), click(10_800.0, 920.0, 1072.0), click(11_600.0, 940.0, 1070.0)];
        assert!(gen(nav).is_empty());
        let right: Vec<Event> = (0..3).map(|i| Event { t: 10_000.0 + i as f64 * 800.0, kind: EventKind::Click { x: 900.0, y: 500.0, button: MouseButton::Right } }).collect();
        assert!(gen(right).is_empty());
        // The click that ends the recording does not count.
        let end = burst(57_500.0, 3, 1000.0, 900.0, 500.0);
        assert!(gen(end).is_empty());
    }

    #[test]
    fn a_zoom_ends_early_when_the_cursor_jumps_across_the_screen() {
        let cfg = ZoomConfig::default();
        let mut ev = burst(5000.0, 3, 700.0, 300.0, 300.0);
        ev.push(mv(8000.0, 330.0, 320.0));
        ev.push(mv(8300.0, 1800.0, 950.0));
        ev.push(mv(10_000.0, 1802.0, 951.0));
        ev.sort_by(|a, b| a.t.total_cmp(&b.t));
        let segs = gen(ev);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].end_ms < 5000.0 + 1400.0 + cfg.hold_ms + cfg.ease_out_ms, "{segs:?}");
    }

    #[test]
    fn a_typical_one_minute_demo_gets_three_to_six_zooms_not_dozens() {
        // 60 s of a realistic session: lots of single clicks around the UI,
        // movement, a scroll, a few spots where the presenter works for a moment.
        let mut ev: Vec<Event> = Vec::new();
        // About 30 lone clicks spread over the screen and the minute.
        for i in 0..30 {
            let t = 2500.0 + i as f64 * 1850.0;
            ev.push(click(t, 150.0 + (i as f64 * 331.0) % 1600.0, 120.0 + (i as f64 * 197.0) % 800.0));
        }
        // Five places where they click around three or four times.
        for (t, x, y) in [(8000.0, 500.0, 400.0), (19_000.0, 1400.0, 300.0), (30_000.0, 900.0, 700.0), (41_000.0, 400.0, 800.0), (51_000.0, 1500.0, 650.0)] {
            ev.extend(burst(t, 4, 650.0, x, y));
        }
        // The cursor glides from each click to the next, taking up to 0.6 s.
        let mut clicks: Vec<(f64, f64, f64)> = ev.iter().filter_map(|e| if let EventKind::Click { x, y, .. } = e.kind { Some((e.t, x, y)) } else { None }).collect();
        clicks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut prev = (960.0, 540.0);
        let mut prev_t = 0.0;
        for &(t, x, y) in &clicks {
            let ms = 600.0f64.min(t - prev_t - 50.0).max(100.0);
            ev.extend(glide(t, prev, (x, y), ms));
            prev = (x, y);
            prev_t = t;
        }
        // One scroll.
        ev.extend((0..15).map(|i| Event { t: 24_000.0 + i as f64 * 120.0, kind: EventKind::Scroll { x: 900.0, y: 500.0, dx: 0.0, dy: -2.0 } }));
        ev.sort_by(|a, b| a.t.total_cmp(&b.t));
        let segs = gen(ev);
        assert!((3..=6).contains(&segs.len()), "{} zooms: {segs:?}", segs.len());
        // The whole screen shows for at least 1.5 s between zooms, and holds are long.
        for w in segs.windows(2) {
            assert!(w[1].start_ms - w[0].end_ms >= 1499.0, "{segs:?}");
        }
        assert!(segs.iter().all(|s| s.end_ms - s.start_ms >= 3000.0));
    }

    #[test]
    fn a_click_cluster_that_keeps_going_stays_one_zoom() {
        // Ten clicks, a second and a half apart, in one area: one zoom, not three.
        let segs = gen(burst(5000.0, 10, 1500.0, 900.0, 500.0));
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(segs[0].end_ms > 5000.0 + 9.0 * 1500.0);
    }

    #[test]
    fn two_clusters_close_in_time_and_place_become_one_pan_and_far_ones_stay_apart() {
        let near = [burst(5000.0, 3, 600.0, 700.0, 400.0), burst(11_500.0, 3, 600.0, 900.0, 450.0)].concat();
        assert_eq!(gen(near).len(), 1);
        let apart = [burst(5000.0, 3, 600.0, 300.0, 300.0), burst(11_000.0, 3, 600.0, 1500.0, 800.0)].concat();
        let segs = gen(apart);
        assert!(segs.len() <= 2);
        for w in segs.windows(2) {
            assert!(w[1].start_ms - w[0].end_ms >= 1499.0, "{segs:?}");
        }
    }

    #[test]
    fn zoom_settings_round_trip_through_the_project() {
        let mut z = crate::project::ZoomConfigSer::default();
        assert_eq!((z.min_clicks, z.click_window_ms), (3, 4500.0));
        z.min_clicks = 5;
        let c = z.to_config();
        assert_eq!(c.min_clicks, 5);
        // An old project file without the new fields loads with the defaults.
        let old: crate::project::ZoomConfigSer = serde_json::from_str(r#"{"level":1.5,"intensity":"lively"}"#).unwrap();
        assert_eq!(old.min_clicks, 3);
    }

    #[test]
    fn the_default_is_slow_and_gentle() {
        let c = ZoomConfig::default();
        assert!(c.ease_in_ms >= 900.0 && c.ease_in_ms <= 1200.0);
        assert!(c.ease_out_ms >= 900.0 && c.ease_out_ms <= 1200.0);
        assert!(c.click_scale >= 1.4 && c.click_scale <= 1.6);
        assert_eq!(c.min_clicks, 3);
        let segs = gen(burst(5000.0, 3, 500.0, 900.0, 500.0));
        let s = &segs[0];
        assert!(s.end_ms - s.start_ms - c.ease_in_ms - c.ease_out_ms >= 1500.0, "the full zoom holds at least 1.5 s");
        assert!(5000.0 - s.start_ms >= 300.0 && 5000.0 - s.start_ms <= 700.0, "the camera arrives before the first click");
    }

    #[test]
    fn spring_easing_settles_without_overshoot_and_starts_gently() {
        assert_eq!(spring(0.0), 0.0);
        assert!((spring(1.0) - 1.0).abs() < 1e-12);
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = spring(i as f64 / 100.0);
            assert!(v >= prev && v <= 1.0 + 1e-12, "{v}");
            prev = v;
        }
        // Leaves quickly (more than smootherstep at 30%), arrives softly.
        assert!(spring(0.3) > ease_in_out(0.3));
        assert!(spring(0.99) - spring(0.95) < 0.02);
    }

    #[test]
    fn the_default_zoom_is_gentle_and_never_deeper_than_two_times() {
        let c = ZoomConfig::default();
        assert!(c.click_scale <= 1.8 && c.click_scale >= 1.5, "{}", c.click_scale);
        assert_eq!(c.with_level(9.0).click_scale, 2.0);
    }

    #[test]
    fn a_click_on_the_very_edge_gets_only_a_gentle_zoom() {
        assert!(bleed_scale(&[(10.0, 500.0), (60.0, 520.0)], 1920.0, 1080.0, 0.17, 1.5) <= 1.25);
        assert_eq!(bleed_scale(&[(900.0, 500.0), (960.0, 520.0)], 1920.0, 1080.0, 0.17, 1.5), 1.5);
    }

    #[test]
    fn a_cut_in_the_middle_of_a_zoom_does_not_make_the_camera_jump() {
        use crate::project::{Clip, Project};
        let mut p = Project::new("a.mp4".into(), None, 1920, 1080, 20_000.0);
        p.zoom = vec![ZoomSegment { start_ms: 2000.0, end_ms: 12_000.0, scale: 2.0, follow_cursor: false, focus: (1400.0, 300.0), reason: "click".into() }];
        // Remove 5 s from the middle of the zoom.
        p.clips = vec![Clip { src_start_ms: 0.0, src_end_ms: 6000.0, speed: 1.0, mute: false }, Clip { src_start_ms: 11_000.0, src_end_ms: 20_000.0, speed: 1.0, mute: false }];
        let segs = p.zoom_on_output();
        assert_eq!(segs.len(), 1);
        assert_eq!((segs[0].start_ms, segs[0].end_ms), (2000.0, 7000.0));
        let timing = CameraTiming { ease_in_ms: 1000.0, ease_out_ms: 1000.0, follow_smoothing_ms: 0.0, deadzone: 0.0 };
        // Just before and just after the seam (out time 6000) the camera is the same.
        let before = camera_at_mapped(5990.0, &segs, None, 1920.0, 1080.0, timing, &|o| p.out_to_src(o));
        let after = camera_at_mapped(6010.0, &segs, None, 1920.0, 1080.0, timing, &|o| p.out_to_src(o));
        assert!((before.scale - after.scale).abs() < 0.01 && (before.cx - after.cx).abs() < 5.0, "{before:?} {after:?}");
        // And the zoom still eases back out over the last second, with no snap.
        let a = camera_at_mapped(6200.0, &segs, None, 1920.0, 1080.0, timing, &|o| p.out_to_src(o));
        let b = camera_at_mapped(6900.0, &segs, None, 1920.0, 1080.0, timing, &|o| p.out_to_src(o));
        assert!(a.scale > b.scale && b.scale > 1.0, "{a:?} {b:?}");
    }

    #[test]
    fn zoom_steps_are_animated_every_frame_when_zooms_touch() {
        // Two segments back to back: the camera must move a little every
        // frame (never jump) through the whole sequence.
        let segs = vec![
            ZoomSegment { start_ms: 1000.0, end_ms: 5000.0, scale: 1.5, follow_cursor: false, focus: (500.0, 400.0), reason: "click".into() },
            ZoomSegment { start_ms: 5000.0, end_ms: 9000.0, scale: 1.5, follow_cursor: false, focus: (1500.0, 700.0), reason: "click".into() },
        ];
        let timing = CameraTiming { ease_in_ms: 1000.0, ease_out_ms: 1000.0, follow_smoothing_ms: 0.0, deadzone: 0.0 };
        let mut prev = camera_at(0.0, &segs, None, 1920.0, 1080.0, timing);
        let mut t = 33.0;
        while t < 10_000.0 {
            let c = camera_at(t, &segs, None, 1920.0, 1080.0, timing);
            assert!((c.scale - prev.scale).abs() < 0.05 && (c.cx - prev.cx).abs() < 60.0 && (c.cy - prev.cy).abs() < 60.0, "jump at {t}: {prev:?} -> {c:?}");
            prev = c;
            t += 33.0;
        }
    }

    #[test]
    fn shortcuts_alone_do_not_zoom() {
        let segs = generate_zoom_segments(
            &log_with(vec![key(1000.0, "C", &["Ctrl"]), key(1200.0, "V", &["Ctrl"]), key(1400.0, "S", &["Ctrl"])]),
            10_000.0,
            &ZoomConfig::default(),
        );
        assert!(segs.is_empty());
    }

    #[test]
    fn with_level_and_speed_apply() {
        let c = ZoomConfig::default().with_level(2.5).with_speed(600.0, 800.0);
        assert_eq!(c.click_scale, 2.0, "never deeper than 2x");
        assert_eq!(c.ease_in_ms, 600.0);
        assert_eq!(c.min_duration_ms, 600.0 + 800.0 + 1700.0);
    }

    #[test]
    fn zoom_in_never_moves_faster_than_twice_its_average_speed() {
        let step = 1.0 / 400.0;
        let mut peak = 0.0f64;
        for i in 0..400 {
            let a = ease_in_out(i as f64 * step);
            let b = ease_in_out((i + 1) as f64 * step);
            peak = peak.max((b - a) / step);
        }
        assert!(peak < 2.0, "{peak}");
    }

    #[test]
    fn small_cursor_moves_do_not_pan_the_camera() {
        // Zoomed 2x on a 1920 wide source: the visible half width is 480 px. The
        // cursor wobbling by 150 px stays inside the dead zone, so the camera
        // does not move at all.
        let log = log_with(vec![mv(0.0, 900.0, 500.0), mv(2500.0, 1050.0, 560.0), mv(3500.0, 880.0, 520.0), mv(4500.0, 1000.0, 540.0), mv(6000.0, 960.0, 540.0)]);
        let track = CursorTrack::new(&log);
        let seg = ZoomSegment { start_ms: 1000.0, end_ms: 9000.0, scale: 2.0, follow_cursor: true, focus: (960.0, 540.0), reason: "click".into() };
        let timing = CameraTiming { ease_in_ms: 1200.0, ease_out_ms: 1200.0, follow_smoothing_ms: 600.0, deadzone: 0.48 };
        let at = |t: f64| camera_at(t, std::slice::from_ref(&seg), Some(&track), 1920.0, 1080.0, timing);
        let a = at(3000.0);
        for t in [4000.0, 5000.0, 6000.0, 7000.0] {
            let c = at(t);
            assert!((c.cx - a.cx).abs() < 12.0 && (c.cy - a.cy).abs() < 12.0, "panned at {t}: {a:?} -> {c:?}");
        }
    }

    #[test]
    fn easing_is_monotonic_and_bounded() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-9);
        let mut prev = 0.0;
        for i in 0..=100 {
            let v = ease_in_out(i as f64 / 100.0);
            assert!(v >= prev);
            prev = v;
        }
    }

    #[test]
    fn camera_eases_and_stays_in_frame() {
        let seg = ZoomSegment {
            start_ms: 1000.0,
            end_ms: 3000.0,
            scale: 2.0,
            follow_cursor: false,
            focus: (0.0, 0.0),
            reason: "manual".into(),
        };
        let segs = [seg];
        let before = camera_at(500.0, &segs, None, 1920.0, 1080.0, T);
        assert_eq!(before.scale, 1.0);
        let mid = camera_at(2000.0, &segs, None, 1920.0, 1080.0, T);
        assert_eq!(mid.scale, 2.0);
        // Focus at the corner is clamped so the view stays inside the frame.
        let (x, y, w, h) = mid.view_rect(1920.0, 1080.0);
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 1920.0 + 1e-9 && y + h <= 1080.0 + 1e-9);
        let easing = camera_at(1200.0, &segs, None, 1920.0, 1080.0, T);
        assert!(easing.scale > 1.0 && easing.scale < 2.0);
        // Continuity: small time steps give small scale changes.
        let mut prev = camera_at(900.0, &segs, None, 1920.0, 1080.0, T).scale;
        let mut t = 900.0;
        while t < 3100.0 {
            t += 16.0;
            let s = camera_at(t, &segs, None, 1920.0, 1080.0, T).scale;
            assert!((s - prev).abs() < 0.15, "jump at {t}");
            prev = s;
        }
    }

    #[test]
    fn camera_follows_cursor() {
        let log = log_with(vec![mv(0.0, 600.0, 400.0), mv(2000.0, 1200.0, 600.0)]);
        let track = CursorTrack::new(&log);
        let seg = ZoomSegment { start_ms: 0.0, end_ms: 4000.0, scale: 2.0, follow_cursor: true, focus: (0.0, 0.0), reason: "click".into() };
        let a = camera_at(800.0, &[seg.clone()], Some(&track), 1920.0, 1080.0, T);
        let b = camera_at(1600.0, &[seg], Some(&track), 1920.0, 1080.0, T);
        assert!(b.cx > a.cx);
    }

    #[test]
    fn camera_holds_still_while_the_cursor_wanders_then_follows() {
        let log = log_with(vec![mv(0.0, 900.0, 500.0), mv(1500.0, 1000.0, 540.0), mv(3000.0, 1000.0, 540.0), mv(5000.0, 1800.0, 900.0)]);
        let track = CursorTrack::new(&log);
        let seg = ZoomSegment { start_ms: 0.0, end_ms: 8000.0, scale: 2.0, follow_cursor: true, focus: (900.0, 500.0), reason: "click".into() };
        let segs = [seg];
        let a = camera_at(2000.0, &segs, Some(&track), 1920.0, 1080.0, CALM);
        let b = camera_at(3000.0, &segs, Some(&track), 1920.0, 1080.0, CALM);
        // The cursor drifted about 100px inside the dead zone: the camera barely moves.
        assert!((a.cx - b.cx).abs() < 30.0, "{} vs {}", a.cx, b.cx);
        let c = camera_at(6500.0, &segs, Some(&track), 1920.0, 1080.0, CALM);
        assert!(c.cx > b.cx + 200.0, "camera did not follow the cursor out of the dead zone");
    }

    #[test]
    fn smoothing_reduces_jitter() {
        let mut ev = Vec::new();
        for i in 0..100 {
            let jitter = if i % 2 == 0 { 20.0 } else { -20.0 };
            ev.push(mv(i as f64 * 16.0, 500.0 + jitter, 500.0));
        }
        let track = CursorTrack::new(&log_with(ev));
        let raw = track.raw_at(800.0).unwrap().0;
        let smooth = track.smooth_at(800.0, 120.0).unwrap().0;
        assert!((smooth - 500.0).abs() < (raw - 500.0).abs());
    }
}
