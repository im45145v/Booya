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
    /// Events closer than this are grouped into one zoom (no bouncing).
    pub merge_gap_ms: f64,
    /// How early the zoom starts before the first event of a group. A little
    /// longer than `ease_in_ms`, so the camera has arrived when the click lands.
    pub lead_in_ms: f64,
    /// How long the zoom holds at full level after the last event of a group.
    pub hold_ms: f64,
    /// Minimum total length of a segment (ease in + hold + ease out).
    pub min_duration_ms: f64,
    pub click_scale: f64,
    pub typing_scale: f64,
    /// Keys within this gap count as one typing burst.
    pub typing_gap_ms: f64,
    /// Minimum keys in a burst before it triggers a zoom.
    pub typing_min_keys: usize,
    /// Ease in duration used when evaluating the camera.
    pub ease_in_ms: f64,
    /// Ease out duration used when evaluating the camera.
    pub ease_out_ms: f64,
    /// Segments closer than this are bridged into one, so the camera pans
    /// instead of zooming out and back in.
    pub bridge_gap_ms: f64,
    /// What a group of events has to score to be worth a zoom (see
    /// `collect_activity`). Lower means more zooms.
    pub threshold: f64,
    /// At most one zoom per this long: of two closer ones the better one stays.
    pub spacing_ms: f64,
    /// A next event this close on screen (share of the screen diagonal)
    /// keeps the zoom and pans to it.
    pub near_share: f64,
    /// The cursor staying about this long where it arrived counts as interest.
    pub dwell_ms: f64,
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
            typing_min_keys: 3,
            ease_in_ms: 900.0,
            ease_out_ms: 900.0,
            bridge_gap_ms: 1800.0,
            threshold: 0.55,
            spacing_ms: 4500.0,
            near_share: 0.25,
            dwell_ms: 1000.0,
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
    /// Only clear moments: typing, a marker, or several aimed clicks together.
    Subtle,
    #[default]
    Normal,
    /// Every aimed or unaimed click, and shorter gaps between zooms.
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
        let (threshold, spacing, near, bridge, hold, dwell, level) = match i {
            Intensity::Subtle => (0.8, 5500.0, 0.2, 2200.0, 2000.0, 1500.0, 1.5),
            Intensity::Normal => (0.55, 4500.0, 0.25, 1800.0, 2500.0, 1000.0, 1.6),
            Intensity::Lively => (0.35, 3500.0, 0.3, 1500.0, 3000.0, 800.0, 1.8),
        };
        self.intensity = i;
        self.threshold = threshold;
        self.spacing_ms = spacing;
        self.near_share = near;
        self.bridge_gap_ms = bridge;
        self.dwell_ms = dwell;
        self.hold_ms = hold;
        self.min_duration_ms = self.ease_in_ms + self.ease_out_ms + self.hold_ms + 100.0;
        self.with_level(level)
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

/// One thing that may deserve a zoom, with how much it deserves it (0..1).
#[derive(Debug, Clone, Copy)]
struct Activity {
    t: f64,
    x: f64,
    y: f64,
    scale: f64,
    /// 0 for keys after the first of a burst: they extend a zoom, not start one.
    score: f64,
    typing: bool,
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

/// Score what happened and decide where the camera should go.
///
/// A zoom is worth it when the viewer has something small to look at:
///  * a **click** that was aimed (the cursor settled before it): 0.75, an
///    unsettled one 0.4. Clicks in the menu bar, tab strip, Dock or taskbar,
///    drags, a click right after a big cursor jump, the second click of a
///    double click and clicks just after a tab or window switch are
///    navigation and score nothing,
///  * the **start of typing**: 0.8,
///  * the cursor **dwelling** about a second where it arrived: 0.6,
///  * a **marker**: 1.0.
/// Events that follow each other and are close on screen (within a quarter of
/// it) are one group; a group zooms when its best score plus 0.15 for every
/// further event reaches the intensity's threshold.
fn collect_activity(log: &EventLog, track: &Held, duration_ms: f64, cfg: &ZoomConfig) -> Vec<Activity> {
    use crate::events::{EventKind, MouseButton};
    let (fw, fh) = (log.header.width as f64, log.header.height as f64);
    let diag = fw.hypot(fh).max(1.0);
    let dist = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    let mut acts: Vec<Activity> = Vec::new();

    // Tab and window switches: what follows is not worth a zoom.
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

    let mut last_click = f64::MIN;
    for e in &log.events {
        let EventKind::Click { x, y, button } = e.kind else { continue };
        let t = e.t;
        let quick_repeat = t - last_click < 350.0;
        last_click = t;
        if button == MouseButton::Right || button == MouseButton::Middle || quick_repeat {
            continue;
        }
        if navigation_area(x, y) || after_nav(t) {
            continue;
        }
        // A drag: the button stays down while the cursor travels.
        let dragged = releases.iter().find(|r| r.0 >= t && r.3 == button).is_some_and(|r| r.0 - t > 400.0 && dist((r.1, r.2), (x, y)) > diag * 0.03);
        if dragged {
            continue;
        }
        let at = |dt: f64| track.at(t - dt).unwrap_or((x, y));
        // Straight after a big jump of the cursor: the viewer is still finding it.
        if dist(at(250.0), (x, y)) > diag * 0.4 {
            continue;
        }
        let aimed = [300.0, 200.0, 100.0].iter().all(|&dt| dist(at(dt), (x, y)) <= diag * 0.012);
        let score = if button == MouseButton::Touch || aimed { 0.75 } else { 0.4 };
        acts.push(Activity { t, x, y, scale: cfg.click_scale, score, typing: false });
    }
    // Markers are deliberate: zoom to where the cursor was when it was dropped.
    acts.extend(log.marks().map(|(t, x, y)| Activity { t, x, y, scale: cfg.click_scale, score: 1.0, typing: false }));

    // Typing bursts: runs of key presses with small gaps. Shortcuts (keys with
    // modifiers other than Shift) are not typing and do not zoom on their own.
    let keys: Vec<f64> = log.keys().filter(|(_, _, m)| m.iter().all(|m| m.eq_ignore_ascii_case("shift"))).map(|(t, _, _)| t).collect();
    let mut i = 0;
    while i < keys.len() {
        let mut j = i;
        while j + 1 < keys.len() && keys[j + 1] - keys[j] <= cfg.typing_gap_ms {
            j += 1;
        }
        if j - i + 1 >= cfg.typing_min_keys {
            let fallback = (fw / 2.0, fh / 2.0);
            for (n, &t) in keys[i..=j].iter().enumerate() {
                let (x, y) = track.at(t).unwrap_or(fallback);
                acts.push(Activity { t, x, y, scale: cfg.typing_scale, score: if n == 0 { 0.8 } else { 0.0 }, typing: true });
            }
        }
        i = j + 1;
    }

    // Dwell: the cursor arrived somewhere and stayed there about a second.
    if !track.s.is_empty() && cfg.dwell_ms > 0.0 {
        let end = if duration_ms > 0.0 { duration_ms } else { f64::MAX };
        let r = diag * 0.035;
        let mut anchor = track.at(0.0).unwrap_or((0.0, 0.0));
        let mut since = 0.0;
        let mut done = false;
        let mut t = 0.0;
        let step = 100.0;
        let last_sample = track.s.last().map(|s| s.0).unwrap_or(0.0);
        let stop = end.min(last_sample + cfg.dwell_ms * 3.0);
        while t <= stop {
            let p = track.at(t).unwrap_or(anchor);
            if dist(p, anchor) > r {
                anchor = p;
                since = t;
                done = false;
            } else if !done && t - since >= cfg.dwell_ms {
                done = true;
                // It arrived from somewhere (not parked from the start) ...
                let came = (1..=15).any(|k| track.at((since - k as f64 * 100.0).max(0.0)).is_some_and(|q| dist(q, anchor) > diag * 0.08));
                // ... on something other than the screen edge, with no click or typing of its own.
                let busy = acts.iter().any(|a| a.score > 0.0 && a.t >= since - 300.0 && a.t <= t + 1500.0);
                if came && !busy && !navigation_area(anchor.0, anchor.1) && !after_nav(t) {
                    acts.push(Activity { t, x: anchor.0, y: anchor.1, scale: cfg.click_scale, score: 0.6, typing: false });
                }
            }
            t += step;
        }
    }
    acts.sort_by(|a, b| a.t.total_cmp(&b.t));
    acts
}

/// Score of a group: the best event plus a little for every further one.
fn group_score(g: &[Activity]) -> f64 {
    let n = g.iter().filter(|a| a.score > 0.0).count();
    let best = g.iter().map(|a| a.score).fold(0.0, f64::max);
    if n == 0 {
        0.0
    } else {
        best + 0.15 * (n - 1) as f64
    }
}

/// Build zoom segments from clicks, typing, dwelling and markers.
pub fn generate_zoom_segments(log: &EventLog, duration_ms: f64, cfg: &ZoomConfig) -> Vec<ZoomSegment> {
    let track = Held::new(log);
    let (fw, fh) = (log.header.width as f64, log.header.height as f64);
    let acts = collect_activity(log, &track, duration_ms, cfg);
    let near = |a: &Activity, b: &Activity| ((a.x - b.x) / fw.max(1.0)).hypot((a.y - b.y) / fh.max(1.0)) <= cfg.near_share;

    // Group what follows closely. Staying zoomed and panning is for events that
    // are close on screen; a far one only joins when it comes straight after
    // (inside the bridge gap), so the camera goes there directly instead of
    // zooming out and in again.
    let mut groups: Vec<Vec<Activity>> = Vec::new();
    for a in acts {
        match groups.last_mut() {
            Some(g) => {
                let l = *g.last().unwrap();
                let gap = a.t - l.t;
                if gap <= cfg.bridge_gap_ms || (gap <= cfg.merge_gap_ms && near(&l, &a)) {
                    g.push(a)
                } else {
                    groups.push(vec![a])
                }
            }
            None => groups.push(vec![a]),
        }
    }

    // Activity that is close in time but far apart on the screen cannot share
    // one view with a margin round it: split it where the view stops fitting.
    let mut split: Vec<Vec<Activity>> = Vec::new();
    for g in groups {
        let mut cur: Vec<Activity> = Vec::new();
        for a in g {
            let mut pts: Vec<(f64, f64)> = cur.iter().map(|c| (c.x, c.y)).collect();
            pts.push((a.x, a.y));
            if !cur.is_empty() && bleed_scale(&pts, fw, fh, cfg.bleed, cfg.max_scale) < 1.2 {
                split.push(std::mem::take(&mut cur));
            }
            cur.push(a);
        }
        if !cur.is_empty() {
            split.push(cur);
        }
    }

    let mut segs: Vec<(ZoomSegment, f64)> = Vec::new();
    for g in split {
        let score = group_score(&g);
        if score < cfg.threshold {
            continue;
        }
        let first = g[0].t;
        let last = g[g.len() - 1].t;
        let mut start = (first - cfg.lead_in_ms).max(0.0);
        let mut end = last + cfg.hold_ms + cfg.ease_out_ms;
        if end - start < cfg.min_duration_ms {
            end = start + cfg.min_duration_ms;
        }
        if duration_ms > 0.0 {
            end = end.min(duration_ms);
            if end - start < cfg.min_duration_ms {
                start = (end - cfg.min_duration_ms).max(0.0);
            }
        }
        if end <= start {
            continue;
        }
        let n = g.len() as f64;
        let focus = (g.iter().map(|a| a.x).sum::<f64>() / n, g.iter().map(|a| a.y).sum::<f64>() / n);
        let base = g.iter().map(|a| a.scale).fold(1.0, f64::max).min(cfg.max_scale);
        let pts: Vec<(f64, f64)> = g.iter().map(|a| (a.x, a.y)).collect();
        // Never tighter than leaves a clear margin around everything clicked.
        let scale = bleed_scale(&pts, fw, fh, cfg.bleed, base);
        if scale < 1.12 {
            continue; // spread over the whole screen: staying wide shows it better
        }
        let typing = g.iter().all(|a| a.typing);
        segs.push((
            ZoomSegment { start_ms: start, end_ms: end, scale, follow_cursor: true, focus, reason: if typing { "typing".into() } else { "click".into() } },
            score,
        ));
    }

    // The lead-in can make neighbours overlap, and a zoom out followed at once
    // by a zoom in looks like a bounce; merge both so the camera pans instead.
    let mut merged: Vec<(ZoomSegment, f64)> = Vec::new();
    for (s, sc) in segs {
        if let Some((p, psc)) = merged.last_mut() {
            if s.start_ms <= p.end_ms + cfg.bridge_gap_ms {
                p.end_ms = p.end_ms.max(s.end_ms);
                p.scale = p.scale.max(s.scale);
                p.focus = ((p.focus.0 + s.focus.0) / 2.0, (p.focus.1 + s.focus.1) / 2.0);
                if p.reason != s.reason {
                    p.reason = "click".into();
                }
                *psc = psc.max(sc);
                continue;
            }
        }
        merged.push((s, sc));
    }
    // Never more than one zoom in `spacing_ms`: of two that are too close,
    // the one that scored higher stays.
    let mut kept: Vec<(ZoomSegment, f64)> = Vec::new();
    for (s, sc) in merged {
        match kept.last_mut() {
            Some((p, psc)) if s.start_ms < p.start_ms + cfg.spacing_ms => {
                if sc > *psc {
                    *p = s;
                    *psc = sc;
                }
            }
            _ => kept.push((s, sc)),
        }
    }
    kept.into_iter().map(|(s, _)| s).collect()
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
        let path = std::env::temp_dir().join(format!("demolens-mark-{}.jsonl", std::process::id()));
        log.save(&path).unwrap();
        let back = EventLog::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.marks().collect::<Vec<_>>(), vec![(10.0, 3.0, 4.0)]);
    }

    #[test]
    fn two_clicks_make_one_segment() {
        let cfg = ZoomConfig::default();
        let segs = generate_zoom_segments(&log_with(vec![click(3000.0, 400.0, 300.0), click(3800.0, 500.0, 320.0)]), 10_000.0, &cfg);
        assert_eq!(segs.len(), 1);
        let s = &segs[0];
        assert_eq!(s.start_ms, 3000.0 - cfg.lead_in_ms);
        assert_eq!(s.end_ms, 3800.0 + cfg.hold_ms + cfg.ease_out_ms);
        assert_eq!(s.scale, cfg.click_scale);
        assert_eq!(s.focus, (450.0, 310.0));
        assert_eq!(s.reason, "click");
    }

    fn rel(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Release { x, y, button: MouseButton::Left } }
    }
    /// A straight glide from a to b ending at `t_end`, one sample every 50 ms.
    fn glide(t_end: f64, a: (f64, f64), b: (f64, f64), ms: f64) -> Vec<Event> {
        let n = (ms / 50.0) as usize;
        (0..=n).map(|i| mv(t_end - ms + i as f64 * 50.0, a.0 + (b.0 - a.0) * i as f64 / n as f64, a.1 + (b.1 - a.1) * i as f64 / n as f64)).collect()
    }
    fn zoomed(events: Vec<Event>, cfg: &ZoomConfig) -> usize {
        generate_zoom_segments(&log_with(events), 3_800.0, cfg).len()
    }

    #[test]
    fn a_single_aimed_click_zooms_with_a_short_lead_in() {
        let cfg = ZoomConfig::default();
        let segs = generate_zoom_segments(&log_with(vec![click(3000.0, 900.0, 500.0)]), 10_000.0, &cfg);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!(segs[0].start_ms, 3000.0 - cfg.lead_in_ms);
        assert!(cfg.lead_in_ms >= 300.0 && cfg.lead_in_ms <= 700.0);
        assert_eq!(segs[0].end_ms, segs[0].start_ms + cfg.min_duration_ms);
        // A fast double click is one click.
        assert_eq!(generate_zoom_segments(&log_with(vec![click(3000.0, 900.0, 500.0), click(3150.0, 900.0, 500.0)]), 10_000.0, &cfg).len(), 1);
    }

    #[test]
    fn navigation_is_not_zoomed_on() {
        let cfg = ZoomConfig::default();
        let one = |e: Vec<Event>| zoomed(e, &cfg);
        // The Dock or taskbar, the menu bar and the tab strip.
        assert_eq!(one(vec![click(3000.0, 900.0, 1070.0)]), 0);
        assert_eq!(one(vec![click(3000.0, 900.0, 12.0)]), 0);
        // A right click opens a menu: not a target.
        assert_eq!(one(vec![Event { t: 3000.0, kind: EventKind::Click { x: 900.0, y: 500.0, button: MouseButton::Right } }]), 0);
        // A drag: the button stays down while the cursor travels.
        assert_eq!(one(vec![click(1000.0, 600.0, 500.0), rel(1900.0, 1200.0, 700.0)]), 0);
        // Just after Cmd+Tab or Ctrl+Tab.
        assert_eq!(one(vec![key(2800.0, "Tab", &["Meta"]), click(3000.0, 900.0, 500.0)]), 0);
        // Right after a big cursor jump.
        let mut jump = glide(3000.0, (100.0, 100.0), (1800.0, 900.0), 150.0);
        jump.push(click(3000.0, 1800.0, 900.0));
        assert_eq!(one(jump), 0);
        // The same click on a small target does zoom.
        assert_eq!(one(vec![click(3000.0, 900.0, 500.0)]), 1);
    }

    #[test]
    fn intensity_changes_how_selective_it_is() {
        // A click the cursor was still moving into: not aimed.
        let mut hurried = glide(3000.0, (200.0, 200.0), (900.0, 500.0), 400.0);
        hurried.push(click(3000.0, 900.0, 500.0));
        let at = |i| zoomed(hurried.clone(), &ZoomConfig::default().with_intensity(i));
        assert_eq!(at(Intensity::Subtle), 0);
        assert_eq!(at(Intensity::Normal), 0);
        assert_eq!(at(Intensity::Lively), 1);
        // An aimed click: Normal and Lively yes, Subtle needs a second event.
        let aimed = vec![click(3000.0, 900.0, 500.0)];
        let at2 = |i| zoomed(aimed.clone(), &ZoomConfig::default().with_intensity(i));
        assert_eq!((at2(Intensity::Subtle), at2(Intensity::Normal), at2(Intensity::Lively)), (0, 1, 1));
        let two = vec![click(1500.0, 900.0, 500.0), click(2500.0, 940.0, 520.0)];
        assert_eq!(zoomed(two, &ZoomConfig::default().with_intensity(Intensity::Subtle)), 1);
        // Typing is clear enough for every intensity.
        let typing = vec![mv(0.0, 800.0, 600.0), key(1000.0, "H", &[]), key(1150.0, "I", &[]), key(1300.0, "!", &[])];
        assert_eq!(zoomed(typing, &ZoomConfig::default().with_intensity(Intensity::Subtle)), 1);
        assert!(ZoomConfig::default().with_intensity(Intensity::Lively).click_scale > ZoomConfig::default().with_intensity(Intensity::Subtle).click_scale);
    }

    #[test]
    fn a_cursor_that_rests_where_it_arrived_zooms() {
        let mut ev = glide(1000.0, (200.0, 200.0), (900.0, 500.0), 600.0);
        ev.push(mv(3000.0, 903.0, 502.0));
        let segs = generate_zoom_segments(&log_with(ev.clone()), 12_000.0, &ZoomConfig::default());
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(segs[0].start_ms > 1000.0, "the zoom follows the dwell, it does not lead it");
        assert_eq!(generate_zoom_segments(&log_with(ev), 12_000.0, &ZoomConfig::default().with_intensity(Intensity::Subtle)).len(), 0);
        // A cursor parked from the start is not interest.
        assert!(generate_zoom_segments(&log_with(vec![mv(0.0, 900.0, 500.0), mv(6000.0, 901.0, 500.0)]), 12_000.0, &ZoomConfig::default()).is_empty());
    }

    #[test]
    fn a_cluster_is_one_zoom_and_a_gap_between_zooms_is_kept() {
        let cfg = ZoomConfig::default();
        // Three clicks two seconds apart, close on screen: one zoom that pans.
        let near = vec![click(1000.0, 800.0, 400.0), click(3000.0, 1000.0, 500.0), click(5000.0, 1100.0, 450.0)];
        let segs = generate_zoom_segments(&log_with(near), 30_000.0, &cfg);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(segs[0].follow_cursor);
        // Two aimed clicks far apart in time: two zooms with room between.
        let mut far = vec![click(1000.0, 800.0, 400.0)];
        far.extend(glide(13_000.0, (800.0, 400.0), (1100.0, 450.0), 400.0));
        far.push(click(14_000.0, 1100.0, 450.0));
        let segs = generate_zoom_segments(&log_with(far.clone()), 30_000.0, &cfg);
        assert_eq!(segs.len(), 2);
        assert!(segs[1].start_ms - segs[0].end_ms >= cfg.bridge_gap_ms);
        // Never more than one zoom per `spacing_ms`.
        let packed = ZoomConfig { spacing_ms: 20_000.0, ..ZoomConfig::default() };
        assert_eq!(generate_zoom_segments(&log_with(far), 30_000.0, &packed).len(), 1);
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
    fn clicks_spread_over_the_screen_zoom_less_so_they_keep_a_margin() {
        // 800 px apart leaves room for a zoom with a margin on each side.
        let cfg = ZoomConfig::default();
        let mut ev = glide(1800.0, (560.0, 500.0), (1360.0, 520.0), 800.0);
        ev.insert(0, click(1000.0, 560.0, 500.0));
        ev.push(click(1800.0, 1360.0, 520.0));
        let ok = generate_zoom_segments(&log_with(ev), 10_000.0, &cfg);
        assert!(!ok.is_empty());
        let s = ok.iter().map(|s| s.scale).fold(f64::MAX, f64::min);
        assert!(s >= 1.12 && s < cfg.click_scale, "{s}");
        let view_w = 1920.0 / s;
        let margin = (view_w - 800.0) / 2.0 / view_w;
        assert!(margin >= cfg.bleed - 0.01, "margin {margin}");
    }

    #[test]
    fn clicks_far_apart_on_screen_are_split_not_dropped() {
        let cfg = ZoomConfig::default();
        let c = |t: f64, x: f64, y: f64| click(t, x, y);
        let segs = generate_zoom_segments(&log_with(vec![c(1000.0, 300.0, 300.0), c(1500.0, 340.0, 310.0), c(3000.0, 1600.0, 800.0), c(3500.0, 1640.0, 810.0)]), 20_000.0, &cfg);
        assert!(!segs.is_empty(), "two pairs of clicks must still zoom");
        assert!(segs.iter().all(|s| s.scale >= 1.12));
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
        p.clips = vec![Clip { src_start_ms: 0.0, src_end_ms: 6000.0, speed: 1.0 }, Clip { src_start_ms: 11_000.0, src_end_ms: 20_000.0, speed: 1.0 }];
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
    fn close_clicks_merge_far_clicks_split() {
        let cfg = ZoomConfig::default();
        let segs = generate_zoom_segments(
            &log_with(vec![click(1000.0, 100.0, 100.0), click(2000.0, 300.0, 100.0), click(9000.0, 600.0, 500.0), click(9500.0, 700.0, 520.0)]),
            20_000.0,
            &cfg,
        );
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!(segs[0].focus, (200.0, 100.0));
        assert!(segs[0].end_ms < segs[1].start_ms);
    }

    #[test]
    fn typing_burst_zooms_but_lone_key_does_not() {
        let cfg = ZoomConfig::default();
        let lone = generate_zoom_segments(&log_with(vec![mv(0.0, 10.0, 10.0), key(1000.0, "A", &[])]), 10_000.0, &cfg);
        assert!(lone.is_empty());
        let burst = generate_zoom_segments(
            &log_with(vec![mv(0.0, 800.0, 600.0), key(1000.0, "H", &[]), key(1150.0, "I", &[]), key(1300.0, "!", &["Shift"]), key(1450.0, "O", &[])]),
            10_000.0,
            &cfg,
        );
        assert_eq!(burst.len(), 1);
        assert_eq!(burst[0].reason, "typing");
        assert_eq!(burst[0].scale, cfg.typing_scale);
        assert_eq!(burst[0].focus, (800.0, 600.0));
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
    fn segments_are_clamped_to_duration() {
        let cfg = ZoomConfig::default();
        let segs = generate_zoom_segments(&log_with(vec![click(100.0, 1.0, 1.0), click(4900.0, 1.0, 1.0)]), 5000.0, &cfg);
        for s in &segs {
            assert!(s.start_ms >= 0.0);
            assert!(s.end_ms <= 5000.0);
            assert!(s.end_ms - s.start_ms >= cfg.min_duration_ms - 1e-9);
        }
    }

    #[test]
    fn overlapping_after_lead_in_are_merged() {
        let cfg = ZoomConfig { merge_gap_ms: 500.0, ..Default::default() };
        // 1.6 s between the pairs > merge gap, but hold + lead-in make them overlap.
        let m = |t: f64| click(t, 900.0, 500.0);
        let segs = generate_zoom_segments(&log_with(vec![m(2000.0), m(2400.0), m(3900.0), m(4300.0)]), 20_000.0, &cfg);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn nearby_zooms_are_bridged_not_bounced() {
        let cfg = ZoomConfig { merge_gap_ms: 500.0, ..Default::default() };
        // Segment one ends at 3000+1600+1000 = 5600, segment two starts at
        // 6500-1100 = 5400... use a gap of about 1 s: still one pan, not two zooms.
        let m = |t: f64, x: f64| click(t, x, 500.0);
        let segs = generate_zoom_segments(&log_with(vec![m(2000.0, 900.0), m(2400.0, 900.0), m(7000.0, 1000.0), m(7400.0, 1000.0)]), 30_000.0, &cfg);
        assert_eq!(segs.len(), 1, "{segs:?}");
        let far = generate_zoom_segments(&log_with(vec![m(2000.0, 900.0), m(2400.0, 900.0), m(12_000.0, 1000.0), m(12_400.0, 1000.0)]), 30_000.0, &cfg);
        assert_eq!(far.len(), 2);
    }

    #[test]
    fn defaults_are_slow_and_gentle() {
        let c = ZoomConfig::default();
        assert!(c.ease_in_ms >= 900.0 && c.ease_in_ms <= 1200.0);
        assert!(c.ease_out_ms >= 900.0 && c.ease_out_ms <= 1200.0);
        assert!(c.click_scale >= 1.4 && c.click_scale <= 1.6);
        assert!(c.hold_ms + c.lead_in_ms - c.ease_in_ms >= 1500.0 - 1.0);
        let segs = generate_zoom_segments(&log_with(vec![click(5000.0, 900.0, 500.0), click(5400.0, 900.0, 500.0)]), 30_000.0, &c);
        let s = &segs[0];
        // Full zoom is held for at least 1.5 s.
        assert!(s.end_ms - s.start_ms - c.ease_in_ms - c.ease_out_ms >= 1500.0);
        // Anticipation: the camera arrives before the click.
        assert!(5000.0 - s.start_ms >= 300.0 && 5000.0 - s.start_ms <= 700.0);
    }

    #[test]
    fn timing_settings_change_the_generated_zooms() {
        let log = log_with(vec![click(1000.0, 500.0, 500.0), click(1400.0, 500.0, 500.0), click(6000.0, 900.0, 500.0), click(6400.0, 900.0, 500.0)]);
        let tight = ZoomConfig::default().with_timing(800.0, 1000.0);
        let loose = ZoomConfig::default().with_timing(800.0, 9000.0);
        let a = generate_zoom_segments(&log, 12000.0, &tight);
        let b = generate_zoom_segments(&log, 12000.0, &loose);
        assert!(b.len() < a.len(), "{} {}", a.len(), b.len());
        let c = ZoomConfig::default().with_timing(100.0, 100.0);
        assert_eq!(c.hold_ms, 400.0);
        assert_eq!(c.merge_gap_ms, 500.0);
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
