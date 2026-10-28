//! Post-processing of a finished recording: MKV to MP4 (no re-encode) and a
//! sync report that shows where the video and audio tracks start.

use crate::ffmpeg::{self, ffmpeg_bin, ffprobe_bin};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Start time and duration of the first stream of a kind, in seconds.
pub(crate) fn stream_times(path: &Path, kind: &str) -> Option<(f64, f64)> {
    let out = Command::new(ffprobe_bin())
        .args(["-v", "error", "-select_streams", kind, "-show_entries", "stream=start_time,duration", "-show_entries", "format=duration", "-of", "json"])
        .arg(ffmpeg::file_url(path))
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let st = v["streams"].as_array()?.first()?;
    let start = st["start_time"].as_str().and_then(|s| s.parse::<f64>().ok())?;
    let dur = st["duration"].as_str().or(v["format"]["duration"].as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    Some((start, dur))
}

/// Where the audio starts relative to the video, in seconds (positive means
/// later), when the two tracks differ enough to be worth fixing.
const LEAD_TOLERANCE_S: f64 = 0.04;

/// The ffmpeg audio filter that lines the microphone up with the video, or
/// None when the tracks already match. The audio is first re-based to zero so
/// the result does not depend on the container timestamps:
///  * it starts later than the video: silence is put in front,
///  * it starts earlier: the extra start is trimmed,
///  * it ends before the video: it is padded with silence (the caller stops
///    at the end of the video).
pub fn audio_align_filter(video: (f64, f64), audio: (f64, f64)) -> Option<String> {
    let lead = audio.0 - video.0;
    let video_end = video.0 + video.1;
    let audio_end = audio.0 + audio.1;
    let short = video_end - audio_end > 0.1;
    if lead.abs() <= LEAD_TOLERANCE_S && !short {
        return None;
    }
    let mut f = String::from("asetpts=PTS-STARTPTS");
    if lead > LEAD_TOLERANCE_S {
        f.push_str(&format!(",adelay={:.0}:all=1", lead * 1000.0));
    } else if lead < -LEAD_TOLERANCE_S {
        f.push_str(&format!(",atrim=start={:.3},asetpts=PTS-STARTPTS", -lead));
    }
    f.push_str(",apad");
    Some(f)
}

/// The result of a remux: the MP4 and how long after the recording clock
/// started the first video frame was stamped.
#[derive(Debug, Clone)]
pub struct Remuxed {
    pub path: PathBuf,
    /// Seconds between the start of the shared recording clock and the first
    /// video frame. The MP4's video starts at 0, so an event logged at `t`
    /// on that clock belongs at `t - video_start_s` in the video.
    pub video_start_s: f64,
}

pub fn remux_to_mp4(mkv: &Path) -> Result<PathBuf> {
    Ok(remux_to_mp4_info(mkv)?.path)
}

/// Remux `recording.mkv` into `recording.mp4` next to it (video is copied,
/// with `+faststart`) and remove the MKV once the MP4 is verified. On any
/// problem the MKV is left alone and an error is returned.
///
/// The microphone and the video are stamped with the same clock. A microphone
/// that took a moment to start is lined up by putting silence in front (only
/// the audio is re-encoded, and only when it needs it), so the MP4 has both
/// tracks starting together and ending together. A microphone whose clock is
/// not on the same timeline (start far outside the recording) is shifted to
/// start with the video.
pub fn remux_to_mp4_info(mkv: &Path) -> Result<Remuxed> {
    let mp4 = mkv.with_extension("mp4");
    let tmp = mkv.with_extension("mp4.part");
    let _ = std::fs::remove_file(&tmp);
    let video = stream_times(mkv, "v:0");
    let audio = stream_times(mkv, "a:0");
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(["-v", "error", "-nostdin", "-y"]);
    let mut video_start_s = 0.0;
    if let Some((vs, _)) = video {
        video_start_s = vs.max(0.0);
    }
    let mut align: Option<String> = None;
    if let (Some(v), Some(a)) = (video, audio) {
        let (vs, vd) = v;
        let a_start = a.0;
        // An audio clock far away from the video's means it was not aligned.
        if (a_start - vs).abs() > 30.0 + vd {
            eprintln!("demolens: audio starts at {a_start:.1}s but video at {vs:.1}s, realigning");
            cmd.arg("-i").arg(ffmpeg::file_url(mkv));
            cmd.args(["-itsoffset", &format!("{:.3}", vs - a_start), "-i"]).arg(ffmpeg::file_url(mkv));
            cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
        } else {
            align = audio_align_filter(v, a);
            // ffmpeg starts the output at the earliest stream. When the audio
            // starts before the video, hold the video back so it still begins at 0.
            if a_start < vs {
                cmd.args(["-itsoffset", &format!("-{:.3}", vs - a_start)]);
            }
            cmd.arg("-i").arg(ffmpeg::file_url(mkv));
            cmd.args(["-map", "0:v:0", "-map", "0:a:0?"]);
        }
    } else {
        cmd.arg("-i").arg(ffmpeg::file_url(mkv));
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?"]);
    }
    if let Some(f) = &align {
        // Video is copied, audio is re-encoded with the alignment. `-shortest`
        // ends the file with the video, as the padded audio never ends.
        cmd.args(["-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-af", f, "-shortest"]);
    } else {
        cmd.args(["-c", "copy"]);
    }
    cmd.args(["-movflags", "+faststart", "-f", "mp4"]).arg(ffmpeg::file_url(&tmp));
    let out = cmd.stdin(Stdio::null()).output().context("failed to start ffmpeg for the remux")?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("ffmpeg remux failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    // Verify before the MKV goes away.
    let before = ffmpeg::probe(mkv).map(|i| i.duration_ms).unwrap_or(0.0);
    let after = ffmpeg::probe(&tmp).context("the remuxed file does not open")?;
    if after.width == 0 || after.duration_ms <= 0.0 || (before > 0.0 && (after.duration_ms - before).abs() > 1500.0) {
        let _ = std::fs::remove_file(&tmp);
        bail!("the remuxed file looks wrong ({:.1}s, expected {:.1}s)", after.duration_ms / 1000.0, before / 1000.0);
    }
    std::fs::rename(&tmp, &mp4)?;
    let _ = std::fs::remove_file(mkv);
    Ok(Remuxed { path: mp4, video_start_s })
}

/// How a recording's tracks line up. Used by `demolens verify`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub video_start_ms: f64,
    pub video_duration_ms: f64,
    pub audio_start_ms: Option<f64>,
    pub audio_duration_ms: Option<f64>,
    /// Audio start minus video start (positive = audio starts later).
    pub audio_offset_ms: Option<f64>,
    pub events: Option<usize>,
    pub last_event_ms: Option<f64>,
    pub clicks: Option<usize>,
    /// Plain language findings, empty when everything lines up.
    pub issues: Vec<String>,
}

pub fn sync_report(video: &Path, events: Option<&Path>) -> Result<SyncReport> {
    let v = stream_times(video, "v:0").context("no video stream")?;
    let a = stream_times(video, "a:0");
    let (vs, vd) = (v.0 * 1000.0, v.1 * 1000.0);
    let mut r = SyncReport {
        video_start_ms: vs,
        video_duration_ms: vd,
        audio_start_ms: a.map(|a| a.0 * 1000.0),
        audio_duration_ms: a.map(|a| a.1 * 1000.0),
        audio_offset_ms: a.map(|a| (a.0 - v.0) * 1000.0),
        events: None,
        last_event_ms: None,
        clicks: None,
        issues: Vec::new(),
    };
    if vs.abs() > 100.0 {
        r.issues.push(format!("The video track starts at {vs:.0} ms instead of 0."));
    }
    match r.audio_offset_ms {
        Some(off) if off.abs() > 250.0 => r.issues.push(format!("The audio starts {off:.0} ms {} the video. It should line up within about 250 ms.", if off > 0.0 { "after" } else { "before" })),
        None => r.issues.push("There is no audio track.".into()),
        _ => {}
    }
    if let (Some(ad), true) = (r.audio_duration_ms, vd > 0.0) {
        let end_gap = (vd - (r.audio_start_ms.unwrap_or(0.0) + ad)).abs();
        if end_gap > 500.0 {
            r.issues.push(format!("The audio and video end {end_gap:.0} ms apart."));
        }
    }
    if let Some(ev) = events.filter(|p| p.exists()) {
        let log = crate::events::EventLog::load(ev)?;
        r.events = Some(log.events.len());
        r.clicks = Some(log.clicks().count());
        let last = log.events.iter().map(|e| e.t).fold(0.0, f64::max);
        r.last_event_ms = Some(last);
        if vd > 0.0 && last > vd + 750.0 {
            r.issues.push(format!("The event log runs {:.0} ms past the end of the video, so zooms would drift.", last - vd));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dl-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn remux_keeps_streams_and_removes_the_mkv() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("remux");
        let mkv = d.join("recording.mkv");
        let ok = Command::new(ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=1.5", "-f", "lavfi", "-i", "sine=frequency=440:d=1.5", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&mkv)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mp4 = remux_to_mp4(&mkv).unwrap();
        assert_eq!(mp4.extension().unwrap(), "mp4");
        assert!(mp4.exists() && !mkv.exists());
        let info = ffmpeg::probe(&mp4).unwrap();
        assert!(info.has_audio);
        assert!((info.duration_ms - 1500.0).abs() < 200.0, "{}", info.duration_ms);
        let rep = sync_report(&mp4, None).unwrap();
        assert!(rep.issues.is_empty(), "{:?}", rep.issues);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_align_filter_pads_a_late_start_and_trims_an_early_one() {
        // Video 0 to 6 s, audio starting at 0.78 s and ending at 6.7 s.
        let f = audio_align_filter((0.0, 6.0), (0.778, 5.92)).unwrap();
        assert!(f.contains("adelay=778:all=1") && f.ends_with(",apad"), "{f}");
        let f = audio_align_filter((0.5, 5.5), (0.1, 5.9)).unwrap();
        assert!(f.contains("atrim=start=0.400"), "{f}");
        // Aligned tracks are left alone, and so is a track that is only a few ms off.
        assert!(audio_align_filter((0.0, 5.0), (0.02, 4.99)).is_none());
        // An audio track that stops early is padded even when it starts on time.
        assert!(audio_align_filter((0.0, 5.0), (0.0, 4.0)).unwrap().ends_with(",apad"));
    }

    #[test]
    fn a_microphone_that_started_late_is_padded_with_silence_in_the_remux() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("remux-late");
        let mkv = d.join("recording.mkv");
        // 3 s of picture, sound that starts at 0.8 s and stops 0.3 s early.
        let ok = Command::new(ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=3", "-itsoffset", "0.8", "-f", "lavfi", "-i", "sine=frequency=440:d=1.9", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&mkv)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let before = sync_report(&mkv, None).unwrap();
        assert!(before.audio_offset_ms.unwrap() > 700.0, "{before:?}");
        let r = remux_to_mp4_info(&mkv).unwrap();
        let rep = sync_report(&r.path, None).unwrap();
        assert!(rep.audio_offset_ms.unwrap().abs() < 100.0, "{rep:?}");
        assert!(rep.issues.is_empty(), "{:?}", rep.issues);
        assert!((rep.video_duration_ms - 3000.0).abs() < 200.0, "{rep:?}");
        assert!((rep.audio_duration_ms.unwrap() - rep.video_duration_ms).abs() < 200.0, "{rep:?}");
        // The sound is still where it was recorded: quiet first, then the tone.
        let x = crate::audio::decode_mono(&r.path, 8000).unwrap();
        let rms = |a: f64, b: f64| ((x[(a * 8000.0) as usize..(b * 8000.0) as usize].iter().map(|v| v * v).sum::<f32>()) / ((b - a) * 8000.0) as f32).sqrt();
        assert!(rms(0.1, 0.7) < 0.001 && rms(1.0, 2.5) > 0.05);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_remux_keeps_the_original() {
        let d = tmpdir("remux-bad");
        let mkv = d.join("recording.mkv");
        std::fs::write(&mkv, b"not a video").unwrap();
        assert!(remux_to_mp4(&mkv).is_err());
        assert!(mkv.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
