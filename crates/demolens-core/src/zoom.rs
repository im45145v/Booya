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
}

impl Default for ZoomConfig {
    fn default() -> Self {
        Self {
            merge_gap_ms: 3500.0,
            lead_in_ms: 1300.0,
            hold_ms: 1600.0,
            min_duration_ms: 4100.0,
            click_scale: 1.7,
            typing_scale: 1.6,
            typing_gap_ms: 1200.0,
            typing_min_keys: 3,
            ease_in_ms: 1200.0,
            ease_out_ms: 1200.0,
            bridge_gap_ms: 1500.0,
        }
    }
}

impl ZoomConfig {
    /// Total of ease in, a hold and ease out: the shortest sensible segment.
    pub fn with_speed(mut self, ease_in_ms: f64, ease_out_ms: f64) -> Self {
        self.ease_in_ms = ease_in_ms.clamp(150.0, 3000.0);
        self.ease_out_ms = ease_out_ms.clamp(150.0, 3000.0);
        self.lead_in_ms = self.ease_in_ms + 100.0;
        self.min_duration_ms = self.ease_in_ms + self.ease_out_ms + 1700.0;
        self
    }
    pub fn with_level(mut self, level: f64) -> Self {
        let level = level.clamp(1.1, 4.0);
        self.click_scale = level;
        self.typing_scale = (level - 0.1).max(1.1);
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct Activity {
    t: f64,
    x: f64,
    y: f64,
    scale: f64,
    typing: bool,
}

/// Build zoom segments from clicks and typing bursts.
pub fn generate_zoom_segments(log: &EventLog, duration_ms: f64, cfg: &ZoomConfig) -> Vec<ZoomSegment> {
    let track = CursorTrack::new(log);
    let mut acts: Vec<Activity> = log
        .clicks()
        .map(|(t, x, y)| Activity { t, x, y, scale: cfg.click_scale, typing: false })
        .collect();
    // Markers are deliberate: zoom to where the cursor was when it was dropped.
    acts.extend(log.marks().map(|(t, x, y)| Activity { t, x, y, scale: cfg.click_scale, typing: false }));

    // Typing bursts: runs of key presses with small gaps. Shortcuts (keys with
    // modifiers other than Shift) are not typing and do not zoom on their own.
    let keys: Vec<f64> = log
        .keys()
        .filter(|(_, _, m)| m.iter().all(|m| m.eq_ignore_ascii_case("shift")))
        .map(|(t, _, _)| t)
        .collect();
    let mut i = 0;
    while i < keys.len() {
        let mut j = i;
        while j + 1 < keys.len() && keys[j + 1] - keys[j] <= cfg.typing_gap_ms {
            j += 1;
        }
        if j - i + 1 >= cfg.typing_min_keys {
            let fallback = (log.header.width as f64 / 2.0, log.header.height as f64 / 2.0);
            for &t in &keys[i..=j] {
                let (x, y) = track.raw_at(t).unwrap_or(fallback);
                acts.push(Activity { t, x, y, scale: cfg.typing_scale, typing: true });
            }
        }
        i = j + 1;
    }
    acts.sort_by(|a, b| a.t.total_cmp(&b.t));

    // Group activity.
    let mut groups: Vec<Vec<Activity>> = Vec::new();
    for a in acts {
        match groups.last_mut() {
            Some(g) if a.t - g.last().unwrap().t <= cfg.merge_gap_ms => g.push(a),
            _ => groups.push(vec![a]),
        }
    }

    let mut segs: Vec<ZoomSegment> = Vec::new();
    for g in groups {
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
        let scale = g.iter().map(|a| a.scale).fold(1.0, f64::max);
        let typing = g.iter().all(|a| a.typing);
        segs.push(ZoomSegment {
            start_ms: start,
            end_ms: end,
            scale,
            follow_cursor: true,
            focus,
            reason: if typing { "typing".into() } else { "click".into() },
        });
    }

    // Lead-in can make neighbours overlap, and a zoom out followed at once by a
    // zoom in looks like a bounce; merge both so the camera pans instead.
    let mut merged: Vec<ZoomSegment> = Vec::new();
    for s in segs {
        if let Some(p) = merged.last_mut() {
            if s.start_ms <= p.end_ms + cfg.bridge_gap_ms {
                p.end_ms = p.end_ms.max(s.end_ms);
                p.scale = p.scale.max(s.scale);
                p.focus = ((p.focus.0 + s.focus.0) / 2.0, (p.focus.1 + s.focus.1) / 2.0);
                if p.reason != s.reason {
                    p.reason = "click".into();
                }
                continue;
            }
        }
        merged.push(s);
    }
    merged
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
fn follow_target(s: &ZoomSegment, tr: &CursorTrack, t: f64, timing: &CameraTiming, w: f64) -> (f64, f64) {
    let smooth = timing.follow_smoothing_ms;
    let at = |x: f64| tr.smooth_at(x, smooth);
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
    let k_in = ease_in_out((t - s.start_ms) / ein);
    let k_out = ease_in_out((s.end_ms - t) / eout);
    let k = k_in.min(k_out);
    let scale = s.scale.max(1.0).powf(k);

    let target = if s.follow_cursor {
        track.map(|tr| follow_target(s, tr, t, &timing, w)).unwrap_or(s.focus)
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
    fn single_click_makes_one_segment() {
        let cfg = ZoomConfig::default();
        let segs = generate_zoom_segments(&log_with(vec![click(3000.0, 400.0, 300.0)]), 10_000.0, &cfg);
        assert_eq!(segs.len(), 1);
        let s = &segs[0];
        assert_eq!(s.start_ms, 3000.0 - cfg.lead_in_ms);
        assert_eq!(s.end_ms, 3000.0 + cfg.hold_ms + cfg.ease_out_ms);
        assert_eq!(s.scale, cfg.click_scale);
        assert_eq!(s.focus, (400.0, 300.0));
        assert_eq!(s.reason, "click");
    }

    #[test]
    fn close_clicks_merge_far_clicks_split() {
        let cfg = ZoomConfig::default();
        let segs = generate_zoom_segments(
            &log_with(vec![click(1000.0, 100.0, 100.0), click(2000.0, 300.0, 100.0), click(9000.0, 50.0, 50.0)]),
            20_000.0,
            &cfg,
        );
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].focus, (200.0, 100.0));
        assert!(segs[0].end_ms < segs[1].start_ms);
    }

    #[test]
    fn typing_burst_zooms_but_lone_key_does_not() {
        let cfg = ZoomConfig::default();
        let lone = generate_zoom_segments(&log_with(vec![mv(0.0, 10.0, 10.0), key(1000.0, "A", &[])]), 10_000.0, &cfg);
        assert!(lone.is_empty());
        let burst = generate_zoom_segments(
            &log_with(vec![mv(0.0, 800.0, 600.0), key(1000.0, "H", &[]), key(1150.0, "I", &[]), key(1300.0, "!", &["Shift"])]),
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
        // 1.9 s gap > merge gap, but hold + lead-in make them overlap.
        let segs = generate_zoom_segments(&log_with(vec![click(2000.0, 0.0, 0.0), click(3900.0, 0.0, 0.0)]), 20_000.0, &cfg);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn nearby_zooms_are_bridged_not_bounced() {
        let cfg = ZoomConfig { merge_gap_ms: 500.0, ..Default::default() };
        // Segment one ends at 3000+1600+1000 = 5600, segment two starts at
        // 6500-1100 = 5400... use a gap of about 1 s: still one pan, not two zooms.
        let segs = generate_zoom_segments(&log_with(vec![click(2000.0, 0.0, 0.0), click(7000.0, 900.0, 0.0)]), 30_000.0, &cfg);
        assert_eq!(segs.len(), 1, "{segs:?}");
        let far = generate_zoom_segments(&log_with(vec![click(2000.0, 0.0, 0.0), click(12_000.0, 900.0, 0.0)]), 30_000.0, &cfg);
        assert_eq!(far.len(), 2);
    }

    #[test]
    fn defaults_are_slow_and_gentle() {
        let c = ZoomConfig::default();
        assert!(c.ease_in_ms >= 900.0 && c.ease_in_ms <= 1200.0);
        assert!(c.ease_out_ms >= 900.0 && c.ease_out_ms <= 1200.0);
        assert!(c.click_scale >= 1.6 && c.click_scale <= 1.8);
        assert!(c.hold_ms + c.lead_in_ms - c.ease_in_ms >= 1500.0 - 1.0);
        let segs = generate_zoom_segments(&log_with(vec![click(5000.0, 10.0, 10.0)]), 30_000.0, &c);
        let s = &segs[0];
        // Full zoom is held for at least 1.5 s.
        assert!(s.end_ms - s.start_ms - c.ease_in_ms - c.ease_out_ms >= 1500.0);
        // Anticipation: the camera arrives before the click.
        assert!(5000.0 - s.start_ms > c.ease_in_ms);
    }

    #[test]
    fn with_level_and_speed_apply() {
        let c = ZoomConfig::default().with_level(2.5).with_speed(600.0, 800.0);
        assert_eq!(c.click_scale, 2.5);
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
