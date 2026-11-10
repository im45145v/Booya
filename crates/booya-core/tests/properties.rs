//! Property tests: the time maps, cuts, splices and captions keep their
//! promises for any input, not only the hand picked cases in the unit tests.

use booya_core::captions::{plan_cuts, words_to_captions, AutoCutOptions};
use booya_core::events::{Event, EventKind, EventLog, MouseButton};
use booya_core::project::{CaptionWord, Clip, Project};
use booya_core::splice::Splice;
use booya_core::zoom::{generate_zoom_segments, ZoomConfig};
use proptest::prelude::*;

/// A project of `dur` ms cut into clips at the given points, with speeds.
fn project_with_clips(dur: f64, cuts: &[f64], speeds: &[f64]) -> Project {
    let mut p = Project::new("x.mp4".into(), None, 1920, 1080, dur);
    let mut edges: Vec<f64> = cuts.iter().map(|c| c * dur).collect();
    edges.push(0.0);
    edges.push(dur);
    edges.sort_by(f64::total_cmp);
    edges.dedup_by(|a, b| (*a - *b).abs() < 5.0);
    p.clips = edges
        .windows(2)
        .enumerate()
        // Every other piece is removed, so there are gaps between clips.
        .filter(|(i, _)| i % 2 == 0 || edges.len() < 4)
        .map(|(i, w)| Clip { src_start_ms: w[0], src_end_ms: w[1], speed: speeds[i % speeds.len()], mute: false })
        .collect();
    p
}

fn speeds() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(prop_oneof![Just(1.0), Just(0.5), Just(2.0), 0.25f64..4.0], 1..5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Source time to output time and back lands where it started, and the
    /// map never runs backwards.
    #[test]
    fn source_and_output_time_round_trip(dur in 1_000.0f64..600_000.0, cuts in prop::collection::vec(0.0f64..1.0, 0..8), sp in speeds(), probes in prop::collection::vec(0.0f64..1.0, 1..20)) {
        let p = project_with_clips(dur, &cuts, &sp);
        prop_assume!(p.validate().is_ok() && !p.clips.is_empty());
        let out = p.output_duration_ms();
        prop_assert!(out > 0.0);
        let mut last = f64::NEG_INFINITY;
        let mut ts: Vec<f64> = probes.iter().map(|k| k * dur).collect();
        ts.sort_by(f64::total_cmp);
        for s in ts {
            let o = p.src_to_out(s);
            prop_assert!(o >= last - 1e-6, "src_to_out went backwards at {s}");
            prop_assert!((0.0..=out + 1e-6).contains(&o));
            last = o;
            if let Some(o2) = p.output_time(s) {
                prop_assert!((o2 - o).abs() < 1e-6);
                let back = p.out_to_src(o2);
                prop_assert!((back - s).abs() < 1e-3, "{s} -> {o2} -> {back}");
                let back2 = p.source_time(o2).expect("an output time inside the video has a source time");
                prop_assert!((back2 - s).abs() < 1e-3);
            }
        }
        prop_assert!(p.source_time(out + 1.0).is_none());
    }

    /// Auto cut leaves valid clips that never overlap a removed range, and
    /// removes exactly the length that was asked for.
    #[test]
    fn cut_ranges_removes_exactly_what_it_should(dur in 2_000.0f64..300_000.0, ranges in prop::collection::vec((0.0f64..1.0, 0.0f64..0.2), 0..10)) {
        let mut p = Project::new("x.mp4".into(), None, 1920, 1080, dur);
        let cuts: Vec<(f64, f64)> = ranges.iter().map(|&(a, l)| (a * dur, (a + l).min(1.0) * dur)).filter(|(a, b)| b > a).collect();
        p.cut_ranges(&cuts);
        prop_assert!(p.validate().is_ok(), "{:?}", p.clips);
        prop_assert!(!p.clips.is_empty());
        // The union of the cuts, inside the video.
        let mut u = cuts.clone();
        u.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (a, b) in u {
            match merged.last_mut() {
                Some(m) if a <= m.1 => m.1 = m.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        let removed: f64 = merged.iter().map(|(a, b)| b.min(dur) - a.max(0.0)).sum();
        let kept: f64 = p.clips.iter().map(|c| c.src_end_ms - c.src_start_ms).sum();
        if removed < dur - 1.0 {
            // Pieces under 1 ms are dropped, so allow 1 ms per cut.
            prop_assert!((kept - (dur - removed)).abs() <= 1.0 * (cuts.len() as f64 + 1.0), "kept {kept}, expected {}", dur - removed);
        }
        for c in &p.clips {
            for &(a, b) in &cuts {
                prop_assert!(c.src_end_ms <= a + 1e-9 || c.src_start_ms >= b - 1e-9 || kept >= dur - 1.0, "clip {:?} overlaps cut {a}..{b}", c);
            }
        }
    }

    /// A splice moves every moment after it by the same amount, keeps the
    /// order of moments, and drops only what was recorded over.
    #[test]
    fn a_splice_keeps_order(at in 0.0f64..100_000.0, over in 0.0f64..20_000.0, take in 1.0f64..60_000.0, mut ts in prop::collection::vec(0.0f64..200_000.0, 1..30)) {
        let sp = Splice { at_ms: at, until_ms: at + over, take_ms: take };
        ts.sort_by(f64::total_cmp);
        let mut last = f64::NEG_INFINITY;
        for &t in &ts {
            match sp.map_time(t) {
                Some(m) => {
                    prop_assert!(m >= last - 1e-9, "order changed at {t}");
                    last = m;
                    if t < at { prop_assert_eq!(m, t); }
                    if t >= at + over { prop_assert!((m - (t + sp.delta())).abs() < 1e-9); }
                    // Nothing old lands inside the new take.
                    prop_assert!(m < at || m >= at + take - 1e-9, "{t} landed inside the take at {m}");
                }
                None => prop_assert!(t >= at && t < at + over, "{t} was dropped but is outside {at}..{}", at + over),
            }
        }
        for w in ts.windows(2) {
            if let Some((a, b)) = sp.map_range(w[0], w[1]) {
                prop_assert!(b >= a, "range {:?} became {a}..{b}", w);
            }
        }
    }

    /// Captions keep every word in order, never overlap, stay within the
    /// line length (counted in characters, so Telugu and Hindi lines are as
    /// long as English ones), and stay on screen for a moment.
    #[test]
    fn captions_keep_words_in_order_without_overlap(words in prop::collection::vec(("[a-zA-Z]{1,9}|[అ-హ]{1,6}|[क-ह]{1,6}", 0.0f64..900.0, 50.0f64..700.0), 1..60)) {
        let mut t = 0.0;
        let ws: Vec<CaptionWord> = words.iter().map(|(text, gap, len)| {
            t += gap;
            let w = CaptionWord { start_ms: t, end_ms: t + len, text: text.clone() };
            t += len;
            w
        }).collect();
        let caps = words_to_captions(&ws, 42, 900.0);
        let flat: Vec<&str> = caps.iter().flat_map(|c| c.words.iter().map(|w| w.text.as_str())).collect();
        let orig: Vec<&str> = ws.iter().map(|w| w.text.as_str()).collect();
        prop_assert_eq!(flat, orig);
        for c in &caps {
            prop_assert!(c.end_ms >= c.start_ms);
            prop_assert!(c.words.len() == 1 || c.text.chars().count() <= 42, "{} chars: {}", c.text.chars().count(), c.text);
        }
        for w in caps.windows(2) {
            prop_assert!(w[0].end_ms <= w[1].start_ms + 1e-9, "captions overlap: {:?} {:?}", w[0].text, w[1].text);
            // A line only breaks early for a pause or the end of a sentence.
            let (last, next) = (&w[0].words[w[0].words.len() - 1], &w[1].words[0]);
            let joined = w[0].text.chars().count() + 1 + next.text.chars().count();
            let pause = next.start_ms - last.end_ms > 900.0;
            prop_assert!(joined > 42 || pause || last.text.ends_with(['.', '?', '!']), "broke {:?} before {:?} at {} characters", w[0].text, next.text, joined);
        }
    }

    /// Planned cuts are sorted, do not overlap, and are never empty.
    #[test]
    fn planned_cuts_are_sorted_and_disjoint(sil in prop::collection::vec((0.0f64..100_000.0, 0.0f64..5_000.0), 0..20), fill in prop::collection::vec((0.0f64..100_000.0, 50.0f64..600.0, any::<bool>()), 0..20)) {
        let silences: Vec<(f64, f64)> = sil.iter().map(|&(a, l)| (a, a + l)).collect();
        let mut words: Vec<CaptionWord> = fill.iter().map(|&(a, l, f)| CaptionWord { start_ms: a, end_ms: a + l, text: if f { "um".into() } else { "hello".into() } }).collect();
        words.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        let o = AutoCutOptions { cut_fillers: true, cut_silences: true, min_silence_ms: 700.0, keep_ms: 180.0 };
        let cuts = plan_cuts(&words, &silences, &o);
        for c in &cuts {
            prop_assert!(c.1 > c.0 && c.0 >= 0.0, "{:?}", c);
        }
        for w in cuts.windows(2) {
            prop_assert!(w[0].1 < w[1].0, "{:?} and {:?}", w[0], w[1]);
        }
    }

    /// Auto zoom from any pointer log: segments are in order, inside the
    /// video, zoom in rather than out, and aim at a point in the frame, also
    /// when the pointer wandered onto another monitor (negative or too large
    /// coordinates).
    #[test]
    fn auto_zoom_segments_are_sane(dur in 5_000.0f64..120_000.0, evs in prop::collection::vec((0.0f64..1.0, -500.0f64..2_500.0, -500.0f64..1_500.0, 0u8..4), 0..200)) {
        let mut log = EventLog::new(1920, 1080);
        for &(t, x, y, k) in &evs {
            let t = t * dur;
            let kind = match k {
                0 => EventKind::Move { x, y },
                1 => EventKind::Click { x, y, button: MouseButton::Left },
                2 => EventKind::Release { x, y, button: MouseButton::Left },
                _ => EventKind::Key { key: booya_core::events::PRINTABLE.into(), modifiers: Vec::new() },
            };
            log.events.push(Event { t, kind });
        }
        log.events.sort_by(|a, b| a.t.total_cmp(&b.t));
        let segs = generate_zoom_segments(&log, dur, &ZoomConfig::default());
        let (mut last_start, mut last_end) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for s in &segs {
            prop_assert!(s.start_ms < s.end_ms, "{:?}", s);
            prop_assert!(s.start_ms >= 0.0 && s.end_ms <= dur + 1e-6, "{:?} outside 0..{dur}", s);
            // A pan from one zoom to the next overlaps them on purpose, but
            // they stay in order.
            prop_assert!(s.start_ms >= last_start - 1e-6 && s.end_ms >= last_end - 1e-6, "segments out of order at {}", s.start_ms);
            last_start = s.start_ms;
            prop_assert!(s.scale >= 1.0 && s.scale.is_finite(), "{:?}", s);
            prop_assert!(s.focus.0.is_finite() && s.focus.1.is_finite());
            prop_assert!((0.0..=1920.0).contains(&s.focus.0) && (0.0..=1080.0).contains(&s.focus.1), "focus {:?} is off the frame", s.focus);
            last_end = s.end_ms;
        }
    }
}

/// Typing into a window on another monitor (the last click was off the
/// recorded screen) is not on the recording, so it must not zoom.
#[test]
fn typing_on_another_monitor_does_not_zoom() {
    let mut log = EventLog::new(1920, 1080);
    let click = |t: f64, x: f64| Event { t, kind: EventKind::Click { x, y: 400.0, button: MouseButton::Left } };
    let release = |t: f64, x: f64| Event { t, kind: EventKind::Release { x, y: 400.0, button: MouseButton::Left } };
    let typing = |from: f64| (0..14).map(move |i| Event { t: from + i as f64 * 120.0, kind: EventKind::Key { key: booya_core::events::PRINTABLE.into(), modifiers: Vec::new() } });
    // A field clicked on the monitor to the right, then two seconds of typing.
    log.events.push(click(3_000.0, 2_600.0));
    log.events.push(release(3_080.0, 2_600.0));
    log.events.extend(typing(3_600.0));
    let segs = generate_zoom_segments(&log, 20_000.0, &ZoomConfig::default());
    assert!(segs.iter().all(|s| s.reason != "typing"), "{segs:?}");
    // The same typing after a click on the recorded screen still zooms.
    let mut here = EventLog::new(1920, 1080);
    here.events.push(click(3_000.0, 900.0));
    here.events.push(release(3_080.0, 900.0));
    here.events.extend(typing(3_600.0));
    let segs = generate_zoom_segments(&here, 20_000.0, &ZoomConfig::default());
    assert!(segs.iter().any(|s| s.reason == "typing"), "{segs:?}");
}

