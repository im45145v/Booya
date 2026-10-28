//! What is happening on screen, so that nothing is ever cut out of a stretch
//! where something is going on.
//!
//! A demo is often given in silence. Dead air in the microphone says nothing
//! about whether the picture is still: the presenter may be clicking through
//! a page, typing, or waiting on something that loads. Anything that removes
//! time must therefore be checked against three kinds of activity:
//!
//!  * input: any pointer move, click, tap, key or modifier in the event log,
//!  * the picture: frames that change (sampled at 5 per second, compared at a
//!    tiny size so compression noise does not count),
//!  * sound is deliberately **not** used here. Sound is only an extra reason a
//!    stretch may be kept, never a reason to cut it.

use crate::events::EventLog;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

/// Sampling rate of the picture check.
pub const SAMPLE_FPS: f64 = 5.0;
const W: usize = 64;
const H: usize = 36;
/// A pixel counts as changed when its brightness moves by more than this (of 255).
const PIXEL_DELTA: i16 = 14;
/// A frame counts as active when this share of its pixels changed (about 8 of 2304).
const FRAME_SHARE: f64 = 0.0035;

/// Times (ms) at which every frame pair changed enough to count as screen activity.
pub fn visual_changes(src: &Path) -> Result<Vec<f64>> {
    let mut c = crate::proc::command(crate::ffmpeg::ffmpeg_bin());
    c.args(["-v", "error", "-nostdin", "-i"]).arg(crate::ffmpeg::file_url(src));
    c.args(["-an", "-vf", &format!("fps={SAMPLE_FPS},scale={W}:{H}:flags=area,format=gray"), "-f", "rawvideo", "-"]);
    c.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
    let mut child = c.spawn().context("could not start ffmpeg to look for screen activity")?;
    let mut out = child.stdout.take().context("no ffmpeg output")?;
    let mut prev: Option<Vec<u8>> = None;
    let mut buf = vec![0u8; W * H];
    let mut times = Vec::new();
    let mut n = 0usize;
    loop {
        match out.read_exact(&mut buf) {
            Ok(()) => {}
            Err(_) => break,
        }
        if let Some(p) = &prev {
            let changed = p.iter().zip(&buf).filter(|(a, b)| (**a as i16 - **b as i16).abs() > PIXEL_DELTA).count();
            if changed as f64 / (W * H) as f64 >= FRAME_SHARE {
                times.push(n as f64 * 1000.0 / SAMPLE_FPS);
            }
        }
        prev = Some(buf.clone());
        n += 1;
    }
    drop(out);
    let st = child.wait()?;
    if n == 0 || (!st.success() && n < 2) {
        bail!("ffmpeg could not read the video to look for screen activity");
    }
    Ok(times)
}

/// Every moment the presenter did something: moves, clicks, taps, keys, modifiers.
pub fn input_times(log: &EventLog) -> Vec<f64> {
    log.events.iter().map(|e| e.t).collect()
}

/// Ranges (ms) in `[0, duration]` where nothing happens: between two activity
/// times more than `min_gap_ms` apart, shrunk by `pad_after_ms` after the
/// earlier one and `pad_before_ms` before the later one. Ranges shorter than
/// `min_len_ms` after padding are dropped.
pub fn idle_ranges(activity: &[f64], duration_ms: f64, min_gap_ms: f64, pad_after_ms: f64, pad_before_ms: f64, min_len_ms: f64) -> Vec<(f64, f64)> {
    let mut t: Vec<f64> = activity.iter().copied().filter(|x| x.is_finite()).collect();
    t.sort_by(|a, b| a.total_cmp(b));
    let mut edges = vec![0.0];
    edges.extend(t);
    edges.push(duration_ms);
    let mut out = Vec::new();
    for w in edges.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b - a < min_gap_ms {
            continue;
        }
        // The very start and end of the recording have no activity to pad against.
        let s = if a <= 0.0 { 0.0 } else { a + pad_after_ms };
        let e = if b >= duration_ms { duration_ms } else { b - pad_before_ms };
        if e - s >= min_len_ms {
            out.push((s, e));
        }
    }
    out
}

/// Intersection of two sorted range lists.
pub fn intersect(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort_by(|x, y| x.0.total_cmp(&y.0));
    b.sort_by(|x, y| x.0.total_cmp(&y.0));
    while i < a.len() && j < b.len() {
        let s = a[i].0.max(b[j].0);
        let e = a[i].1.min(b[j].1);
        if e > s {
            out.push((s, e));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// How idle stretches are found.
#[derive(Debug, Clone, Copy)]
pub struct IdleOptions {
    /// Nothing may happen for at least this long before a stretch is a candidate.
    pub min_idle_ms: f64,
    /// Kept after the last activity.
    pub pad_after_ms: f64,
    /// Kept before the next activity, so the viewer sees it coming.
    pub pad_before_ms: f64,
    /// Whether to look at the picture too. Turning this off is only for tests.
    pub use_video: bool,
}

impl Default for IdleOptions {
    fn default() -> Self {
        Self { min_idle_ms: 2500.0, pad_after_ms: 700.0, pad_before_ms: 900.0, use_video: true }
    }
}

/// Source ranges in which the screen is completely still and the presenter did nothing.
pub fn idle_stretches(src: &Path, log: Option<&EventLog>, duration_ms: f64, o: &IdleOptions) -> Result<Vec<(f64, f64)>> {
    let mut act = log.map(input_times).unwrap_or_default();
    if o.use_video {
        act.extend(visual_changes(src)?);
    }
    Ok(idle_ranges(&act, duration_ms, o.min_idle_ms, o.pad_after_ms, o.pad_before_ms, 400.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_only_where_nothing_happens() {
        // Activity every second for 14 s, then nothing until 20 s.
        let act: Vec<f64> = (0..14).map(|i| i as f64 * 1000.0).collect();
        let idle = idle_ranges(&act, 20_000.0, 2500.0, 700.0, 900.0, 400.0);
        assert_eq!(idle, vec![(13_700.0, 20_000.0)]);
    }

    #[test]
    fn a_silent_demo_with_steady_activity_has_no_idle_at_all() {
        let act: Vec<f64> = (0..=140).map(|i| i as f64 * 100.0).collect();
        assert!(idle_ranges(&act, 14_000.0, 2500.0, 700.0, 900.0, 400.0).is_empty());
    }

    #[test]
    fn padding_keeps_time_around_the_action() {
        let idle = idle_ranges(&[1000.0, 9000.0], 10_000.0, 2500.0, 700.0, 900.0, 400.0);
        assert_eq!(idle, vec![(1700.0, 8100.0)]);
    }

    #[test]
    fn ranges_intersect() {
        let r = intersect(&[(0.0, 5.0), (8.0, 12.0)], &[(3.0, 9.0), (11.0, 20.0)]);
        assert_eq!(r, vec![(3.0, 5.0), (8.0, 9.0), (11.0, 12.0)]);
    }

    #[test]
    fn still_video_has_no_changes_and_a_flash_has_two() {
        let dir = std::env::temp_dir().join(format!("dl-act-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("flash.mp4");
        let ok = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=black:s=320x180:r=15:d=6", "-vf", "drawbox=x=40:y=40:w=160:h=90:color=white:t=fill:enable='between(t,2,3)'", "-pix_fmt", "yuv420p"])
            .arg(&f)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // no ffmpeg here: the media tests in CI cover this
        }
        let ch = visual_changes(&f).unwrap();
        assert_eq!(ch.len(), 2, "{ch:?}");
        assert!((ch[0] - 2000.0).abs() <= 400.0 && (ch[1] - 3000.0).abs() <= 400.0, "{ch:?}");
        let idle = idle_stretches(&f, None, 6000.0, &IdleOptions { min_idle_ms: 1000.0, pad_after_ms: 300.0, pad_before_ms: 300.0, use_video: true }).unwrap();
        // A still picture is idle even with the box on screen, but the changes themselves never are.
        assert!(idle.iter().all(|&(a, b)| !(a < 2100.0 && b > 1900.0) && !(a < 3100.0 && b > 2900.0)), "{idle:?}");
        assert!(!idle.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
