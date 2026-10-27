//! Post-processing of a finished recording: MKV to MP4 (no re-encode) and a
//! sync report that shows where the video and audio tracks start.

use crate::ffmpeg::{self, ffmpeg_bin, ffprobe_bin};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Start time and duration of the first stream of a kind, in seconds.
fn stream_times(path: &Path, kind: &str) -> Option<(f64, f64)> {
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

/// Remux `recording.mkv` into `recording.mp4` next to it (stream copy, with
/// `+faststart`) and remove the MKV once the MP4 is verified. Returns the MP4
/// path. On any problem the MKV is left alone and an error is returned.
///
/// The audio and video timestamps are kept as recorded, so the sync between
/// them is preserved. A microphone track whose clock is not on the same
/// timeline (start far outside the recording) is shifted to start with the video.
pub fn remux_to_mp4(mkv: &Path) -> Result<PathBuf> {
    let mp4 = mkv.with_extension("mp4");
    let tmp = mkv.with_extension("mp4.part");
    let _ = std::fs::remove_file(&tmp);
    let video = stream_times(mkv, "v:0");
    let audio = stream_times(mkv, "a:0");
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(ffmpeg::file_url(mkv));
    if let (Some((vs, vd)), Some((a_start, _))) = (video, audio) {
        // An audio clock far away from the video's means it was not aligned.
        if (a_start - vs).abs() > 30.0 + vd {
            eprintln!("demolens: audio starts at {a_start:.1}s but video at {vs:.1}s, realigning");
            cmd.args(["-itsoffset", &format!("{:.3}", vs - a_start), "-i"]).arg(ffmpeg::file_url(mkv));
            cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
        } else {
            cmd.args(["-map", "0:v:0", "-map", "0:a:0?"]);
        }
    } else {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?"]);
    }
    cmd.args(["-c", "copy", "-movflags", "+faststart", "-f", "mp4"]).arg(ffmpeg::file_url(&tmp));
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
    Ok(mp4)
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
    fn a_failed_remux_keeps_the_original() {
        let d = tmpdir("remux-bad");
        let mkv = d.join("recording.mkv");
        std::fs::write(&mkv, b"not a video").unwrap();
        assert!(remux_to_mp4(&mkv).is_err());
        assert!(mkv.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
