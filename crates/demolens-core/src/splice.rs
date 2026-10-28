//! Put a new take (or an imported video) into a recording, or record over a
//! stretch of it.
//!
//! The editor works on one source file, so a splice builds a new source:
//! `before + take + after`. Everything that is timed against the source
//! (clips, zooms, captions, callouts, blur boxes, arrows) is moved to match,
//! and the take's mouse and key log is merged into the recording's log at the
//! same place. The previous source file is left where it is, so Undo still
//! has something to go back to.

use crate::events::{scale_coords, EventLog};
use crate::ffmpeg::{self, best_encoder, ffmpeg_bin};
use crate::project::{Clip, Project};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Where a take goes, in source ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Splice {
    /// The take starts here.
    pub at_ms: f64,
    /// Where the recording carries on. Equal to `at_ms` for an insert; later
    /// for "record over this stretch".
    pub until_ms: f64,
    /// Length of the take.
    pub take_ms: f64,
}

impl Splice {
    /// How far everything after the splice moves.
    pub fn delta(&self) -> f64 {
        self.take_ms - (self.until_ms - self.at_ms)
    }

    /// Where a time range of the old source ends up. None when it lay wholly
    /// inside the stretch that was recorded over.
    pub fn map_range(&self, s: f64, e: f64) -> Option<(f64, f64)> {
        let (at, until, d) = (self.at_ms, self.until_ms, self.delta());
        if e <= at {
            Some((s, e))
        } else if s >= until {
            Some((s + d, e + d))
        } else if s >= at && e <= until {
            None
        } else if s < at {
            // Starts before the splice and ends inside it or after it.
            if e <= until {
                Some((s, at))
            } else {
                Some((s, e + d))
            }
        } else {
            // Starts inside the replaced stretch and ends after it.
            Some((at + self.take_ms, e + d))
        }
    }

    /// Where a single moment lands, or None if it was recorded over.
    pub fn map_time(&self, t: f64) -> Option<f64> {
        if t < self.at_ms {
            Some(t)
        } else if t >= self.until_ms {
            Some(t + self.delta())
        } else {
            None
        }
    }
}

/// Move everything in `p` that is timed against the source, and add what the
/// `take` project brought (its clips, and zooms made from its own input log).
/// `scale` is the take's pixels to this source's pixels.
pub fn remap_project(p: &mut Project, sp: &Splice, take: &Project, scale: f64) {
    // Clips: the part before, the take, the part after.
    let (at, until, d) = (sp.at_ms, sp.until_ms, sp.delta());
    let mut before = Vec::new();
    let mut after = Vec::new();
    for c in &p.clips {
        if c.src_end_ms <= at {
            before.push(c.clone());
            continue;
        }
        if c.src_start_ms >= until {
            after.push(Clip { src_start_ms: c.src_start_ms + d, src_end_ms: c.src_end_ms + d, speed: c.speed, mute: c.mute });
            continue;
        }
        if c.src_start_ms < at {
            before.push(Clip { src_start_ms: c.src_start_ms, src_end_ms: at, speed: c.speed, mute: c.mute });
        }
        if c.src_end_ms > until {
            after.push(Clip { src_start_ms: until + d, src_end_ms: c.src_end_ms + d, speed: c.speed, mute: c.mute });
        }
    }
    let mut mid: Vec<Clip> = take
        .clips
        .iter()
        .map(|c| Clip { src_start_ms: c.src_start_ms + at, src_end_ms: c.src_end_ms.min(sp.take_ms) + at, speed: c.speed, mute: c.mute })
        .filter(|c| c.src_end_ms - c.src_start_ms >= 1.0)
        .collect();
    if mid.is_empty() {
        mid.push(Clip { src_start_ms: at, src_end_ms: at + sp.take_ms, speed: 1.0, mute: false });
    }
    before.retain(|c| c.src_end_ms - c.src_start_ms >= 1.0);
    after.retain(|c| c.src_end_ms - c.src_start_ms >= 1.0);
    p.clips = before.into_iter().chain(mid).chain(after).collect();

    p.zoom = p
        .zoom
        .drain(..)
        .filter_map(|mut z| {
            let (s, e) = sp.map_range(z.start_ms, z.end_ms)?;
            z.start_ms = s;
            z.end_ms = e;
            Some(z)
        })
        .chain(take.zoom.iter().cloned().map(|mut z| {
            z.start_ms += at;
            z.end_ms += at;
            z.focus = (z.focus.0 * scale, z.focus.1 * scale);
            z
        }))
        .collect();
    p.zoom.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));

    p.blur = p
        .blur
        .drain(..)
        .filter_map(|mut b| {
            let (s, e) = sp.map_range(b.start_ms, b.end_ms)?;
            b.start_ms = s;
            b.end_ms = e;
            Some(b)
        })
        .collect();
    p.callouts = p
        .callouts
        .drain(..)
        .filter_map(|mut c| {
            let (s, e) = sp.map_range(c.start_ms, c.end_ms)?;
            c.start_ms = s;
            c.end_ms = e;
            Some(c)
        })
        .collect();
    p.shapes = p
        .shapes
        .drain(..)
        .filter_map(|mut c| {
            let (s, e) = sp.map_range(c.start_ms, c.end_ms)?;
            c.start_ms = s;
            c.end_ms = e;
            Some(c)
        })
        .collect();
    p.captions = p
        .captions
        .drain(..)
        .filter_map(|mut c| {
            let (s, e) = sp.map_range(c.start_ms, c.end_ms)?;
            // Words keep their own place; the ones that were recorded over go.
            c.words = c
                .words
                .drain(..)
                .filter_map(|mut w| {
                    let (ws, we) = sp.map_range(w.start_ms, w.end_ms)?;
                    w.start_ms = ws;
                    w.end_ms = we;
                    Some(w)
                })
                .collect();
            c.start_ms = s;
            c.end_ms = e;
            Some(c)
        })
        .collect();
    p.duration_ms = (p.duration_ms + d).max(1.0);
    // One offset cannot describe a camera whose recording now has a hole in it.
    if let Some(w) = p.webcam.as_mut() {
        w.visible = false;
    }
}

/// The two event logs as one: the old one up to the splice, the take's from
/// there, then the rest of the old one after the take.
pub fn merge_events(old: Option<&EventLog>, take: Option<&EventLog>, sp: &Splice, w: u32, h: u32) -> Option<EventLog> {
    if old.is_none() && take.is_none() {
        return None;
    }
    let mut out = match old {
        Some(o) => EventLog { header: o.header.clone(), events: Vec::new() },
        None => {
            let t = take.unwrap();
            let mut l = EventLog::new(w, h);
            l.header = t.header.clone();
            l.events.clear();
            l
        }
    };
    if let Some(o) = old {
        for e in &o.events {
            if let Some(t) = sp.map_time(e.t) {
                let mut e = e.clone();
                e.t = t;
                out.events.push(e);
            }
        }
    }
    if let Some(t) = take {
        // The take's screen may be a different size than the recording's.
        let k = if out.header.width > 0 && t.header.width > 0 { out.header.width as f64 / t.header.width as f64 } else { 1.0 };
        let mut ev: Vec<_> = t.events.iter().filter(|e| e.t <= sp.take_ms).cloned().collect();
        if (k - 1.0).abs() > 0.001 {
            scale_coords(&mut ev, k);
        }
        for mut e in ev {
            e.t += sp.at_ms;
            out.events.push(e);
        }
    }
    out.events.sort_by(|a, b| a.t.total_cmp(&b.t));
    Some(out)
}

/// A file name next to `src` that does not exist yet: `<stem>-take<N><ext>`.
fn next_name(src: &Path, tag: &str, ext: &str) -> PathBuf {
    let dir = src.parent().unwrap_or(Path::new("."));
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "recording".into());
    // `recording-take2` + another take becomes `recording-take3`, not `recording-take2-take1`.
    let base = stem.rsplit_once(&format!("-{tag}")).filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty()).map(|(b, _)| b.to_string()).unwrap_or(stem);
    (1..).map(|n| dir.join(format!("{base}-{tag}{n}{ext}"))).find(|p| !p.exists()).unwrap()
}

/// Build the joined video: old[0, at) + take + old[until, end).
pub fn join_media(src: &Path, take: &Path, sp: &Splice, out: &Path, w: u32, h: u32, fps: f64) -> Result<()> {
    let si = ffmpeg::probe(src).context("read the recording")?;
    let ti = ffmpeg::probe(take).context("read the new take")?;
    let src_len = si.duration_ms;
    let mut parts: Vec<(usize, f64, f64)> = Vec::new(); // (input, start ms, end ms)
    if sp.at_ms > 1.0 {
        parts.push((0, 0.0, sp.at_ms.min(src_len)));
    }
    parts.push((1, 0.0, sp.take_ms.min(ti.duration_ms.max(1.0))));
    if sp.until_ms < src_len - 1.0 {
        parts.push((0, sp.until_ms, src_len));
    }
    let any_audio = si.has_audio || ti.has_audio;
    let fps = if fps >= 5.0 && fps <= 120.0 { fps } else { 30.0 };
    let mut f = String::new();
    for (n, (inp, s, e)) in parts.iter().enumerate() {
        let (ss, ee, len) = (s / 1000.0, e / 1000.0, (e - s) / 1000.0);
        f.push_str(&format!(
            "[{inp}:v]trim=start={ss:.3}:end={ee:.3},setpts=PTS-STARTPTS,fps={fps:.3},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p[v{n}];"
        ));
        if any_audio {
            let has = if *inp == 0 { si.has_audio } else { ti.has_audio };
            if has {
                f.push_str(&format!(
                    "[{inp}:a]atrim=start={ss:.3}:end={ee:.3},asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad=whole_dur={len:.3},atrim=end={len:.3}[a{n}];"
                ));
            } else {
                f.push_str(&format!("anullsrc=r=48000:cl=stereo,atrim=end={len:.3},asetpts=PTS-STARTPTS[a{n}];"));
            }
        }
    }
    for n in 0..parts.len() {
        f.push_str(&format!("[v{n}]"));
        if any_audio {
            f.push_str(&format!("[a{n}]"));
        }
    }
    f.push_str(&format!("concat=n={}:v=1:a={}[v]", parts.len(), if any_audio { 1 } else { 0 }));
    if any_audio {
        f.push_str("[a]");
    }
    let tmp = out.with_extension("mp4.part");
    let mut c = crate::proc::command(ffmpeg_bin());
    c.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(ffmpeg::file_url(src)).arg("-i").arg(ffmpeg::file_url(take));
    c.args(["-filter_complex", &f, "-map", "[v]"]);
    if any_audio {
        c.args(["-map", "[a]", "-c:a", "aac", "-b:a", "192k"]);
    }
    c.args(best_encoder().export_args(w, h, fps)).args(["-pix_fmt", "yuv420p", "-r", &format!("{fps:.3}"), "-movflags", "+faststart", "-f", "mp4"]).arg(&tmp);
    c.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let o = c.output().context("start ffmpeg")?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("joining the recordings failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").trim());
    }
    std::fs::rename(&tmp, out)?;
    Ok(())
}

/// Result of a splice.
pub struct Spliced {
    pub project: Project,
    pub video: PathBuf,
    pub events: Option<PathBuf>,
}

/// Put `take` (a video, with its input log if it has one) into `project` at
/// `at_ms`. With `until_ms` the stretch [at_ms, until_ms) is recorded over.
/// The project given is not changed; the new one is returned.
pub fn splice_take(project: &Project, take_video: &Path, take_events: Option<&Path>, at_ms: f64, until_ms: Option<f64>) -> Result<Spliced> {
    let take = crate::new_project(take_video, take_events)?;
    let dur = project.duration_ms;
    let at = at_ms.clamp(0.0, dur);
    let until = until_ms.unwrap_or(at).clamp(at, dur);
    let sp = Splice { at_ms: at, until_ms: until, take_ms: take.duration_ms };
    if sp.take_ms < 100.0 {
        bail!("the new take is too short");
    }
    let (w, h) = (project.source_width & !1, project.source_height & !1);
    let fps = ffmpeg::probe(&project.source).map(|i| i.fps).unwrap_or(30.0);
    let out = next_name(&project.source, "take", ".mp4");
    join_media(&project.source, take_video, &sp, &out, w, h, fps)?;
    let new_info = ffmpeg::probe(&out).context("the joined recording does not open")?;

    // Input logs.
    let old_log = project.events.as_ref().and_then(|p| EventLog::load(p).ok());
    let take_log = take.events.as_ref().and_then(|p| EventLog::load(p).ok());
    let scale = match (&old_log, &take_log) {
        (Some(o), Some(t)) if o.header.width > 0 && t.header.width > 0 => o.header.width as f64 / t.header.width as f64,
        _ => w as f64 / take.source_width.max(1) as f64,
    };
    let merged = merge_events(old_log.as_ref(), take_log.as_ref(), &sp, w, h);
    let events_out = match &merged {
        Some(m) => {
            let base = project.events.clone().unwrap_or_else(|| project.source.with_extension("events.jsonl"));
            let p = next_name(&base, "take", ".jsonl");
            m.save(&p)?;
            Some(p)
        }
        None => None,
    };

    let mut np = project.clone();
    remap_project(&mut np, &sp, &take, scale);
    np.source = out.clone();
    np.events = events_out.clone();
    np.duration_ms = new_info.duration_ms;
    np.source_width = new_info.width;
    np.source_height = new_info.height;
    np.validate()?;
    Ok(Spliced { project: np, video: out, events: events_out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Event, EventKind};

    fn sp(at: f64, until: f64, take: f64) -> Splice {
        Splice { at_ms: at, until_ms: until, take_ms: take }
    }

    #[test]
    fn an_insert_moves_what_comes_after_and_stretches_what_it_lands_in() {
        let s = sp(1000.0, 1000.0, 500.0);
        assert_eq!(s.delta(), 500.0);
        assert_eq!(s.map_range(0.0, 900.0), Some((0.0, 900.0)));
        assert_eq!(s.map_range(0.0, 1000.0), Some((0.0, 1000.0)));
        assert_eq!(s.map_range(1000.0, 1200.0), Some((1500.0, 1700.0)));
        assert_eq!(s.map_range(800.0, 1200.0), Some((800.0, 1700.0)));
        assert_eq!(s.map_time(999.0), Some(999.0));
        assert_eq!(s.map_time(1000.0), Some(1500.0));
    }

    #[test]
    fn recording_over_a_stretch_removes_what_was_inside_it() {
        let s = sp(1000.0, 2000.0, 600.0);
        assert_eq!(s.delta(), -400.0);
        assert_eq!(s.map_range(1200.0, 1800.0), None);
        assert_eq!(s.map_range(500.0, 1500.0), Some((500.0, 1000.0)));
        assert_eq!(s.map_range(1500.0, 2500.0), Some((1600.0, 2100.0)));
        assert_eq!(s.map_range(500.0, 2500.0), Some((500.0, 2100.0)));
        assert_eq!(s.map_range(2000.0, 2200.0), Some((1600.0, 1800.0)));
        assert_eq!(s.map_time(1500.0), None);
    }

    fn proj(clips: Vec<(f64, f64, f64)>, dur: f64) -> Project {
        let mut p = Project::new("a.mp4".into(), None, 1920, 1080, dur);
        p.clips = clips.into_iter().map(|(a, b, s)| Clip { src_start_ms: a, src_end_ms: b, speed: s, mute: false }).collect();
        p
    }

    #[test]
    fn clips_split_around_the_take_and_keep_their_speed() {
        let mut p = proj(vec![(0.0, 4000.0, 1.0), (4000.0, 6000.0, 2.0)], 6000.0);
        p.callouts.push(crate::project::Callout { text: "hi".into(), start_ms: 5000.0, end_ms: 5500.0, x: 0.5, y: 0.5 });
        let take = proj(vec![(0.0, 1000.0, 1.0)], 1000.0);
        remap_project(&mut p, &sp(2000.0, 2000.0, 1000.0), &take, 1.0);
        let c: Vec<_> = p.clips.iter().map(|c| (c.src_start_ms, c.src_end_ms, c.speed)).collect();
        assert_eq!(c, vec![(0.0, 2000.0, 1.0), (2000.0, 3000.0, 1.0), (3000.0, 5000.0, 1.0), (5000.0, 7000.0, 2.0)]);
        assert!(p.validate().is_ok());
        assert_eq!(p.callouts[0].start_ms, 6000.0);
        assert_eq!(p.duration_ms, 7000.0);
    }

    #[test]
    fn recording_over_replaces_the_clips_in_the_stretch() {
        let mut p = proj(vec![(0.0, 6000.0, 1.0)], 6000.0);
        let take = proj(vec![(0.0, 500.0, 1.0)], 500.0);
        remap_project(&mut p, &sp(1000.0, 3000.0, 500.0), &take, 1.0);
        let c: Vec<_> = p.clips.iter().map(|c| (c.src_start_ms, c.src_end_ms)).collect();
        assert_eq!(c, vec![(0.0, 1000.0), (1000.0, 1500.0), (1500.0, 4500.0)]);
        assert_eq!(p.duration_ms, 4500.0);
    }

    #[test]
    fn the_take_log_lands_at_the_splice_scaled_to_the_recording() {
        let mut old = EventLog::new(1920, 1080);
        old.events.push(Event { t: 500.0, kind: EventKind::Click { x: 100.0, y: 100.0, button: crate::events::MouseButton::Left } });
        old.events.push(Event { t: 1500.0, kind: EventKind::Click { x: 200.0, y: 200.0, button: crate::events::MouseButton::Left } });
        let mut take = EventLog::new(960, 540);
        take.events.push(Event { t: 100.0, kind: EventKind::Click { x: 50.0, y: 50.0, button: crate::events::MouseButton::Left } });
        let m = merge_events(Some(&old), Some(&take), &sp(1000.0, 1000.0, 400.0), 1920, 1080).unwrap();
        let t: Vec<f64> = m.events.iter().map(|e| e.t).collect();
        assert_eq!(t, vec![500.0, 1100.0, 1900.0]);
        match &m.events[1].kind {
            EventKind::Click { x, y, .. } => assert_eq!((*x, *y), (100.0, 100.0)),
            _ => panic!(),
        }
    }

    #[test]
    fn names_count_up() {
        let d = std::env::temp_dir().join(format!("dl-splice-names-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let a = d.join("recording.mp4");
        let n1 = next_name(&a, "take", ".mp4");
        assert_eq!(n1.file_name().unwrap(), "recording-take1.mp4");
        std::fs::write(&n1, b"x").unwrap();
        assert_eq!(next_name(&n1, "take", ".mp4").file_name().unwrap(), "recording-take2.mp4");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn splicing_real_media_keeps_the_sound_with_the_picture() {
        use crate::syncprobe::{make_reference, measure};
        if !crate::ffmpeg::ffmpeg_found() {
            return;
        }
        let d = std::env::temp_dir().join(format!("dl-splice-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a.mp4"), d.join("b.mp4"));
        make_reference(&a, 8, true, true, 0.0).unwrap();
        make_reference(&b, 5, true, true, 0.0).unwrap();
        let p = crate::new_project(&a, None).unwrap();
        // Insert in the middle of a flash gap, and record over a stretch.
        let r = splice_take(&p, &b, None, 3500.0, None).unwrap();
        assert!((r.project.duration_ms - 13000.0).abs() < 250.0, "{}", r.project.duration_ms);
        let m = measure(&r.video).unwrap();
        assert!(m.offset_ms.abs() < 50.0 && m.drift_total_ms.abs() < 25.0, "{m:?}");
        let r2 = splice_take(&r.project, &b, None, 1500.0, Some(4500.0)).unwrap();
        assert!((r2.project.duration_ms - (13000.0 - 3000.0 + 5000.0)).abs() < 250.0, "{}", r2.project.duration_ms);
        let m2 = measure(&r2.video).unwrap();
        assert!(m2.offset_ms.abs() < 50.0 && m2.drift_total_ms.abs() < 25.0, "{m2:?}");
        assert!(r.video.exists() && p.source.exists(), "the old source stays for Undo");
        let _ = std::fs::remove_dir_all(&d);
    }
}
