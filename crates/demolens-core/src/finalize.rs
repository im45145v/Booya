//! Post-processing of a finished recording: MKV to MP4 (no re-encode) and a
//! sync report that shows where the video and audio tracks start.

use crate::ffmpeg::{self, ffmpeg_bin, ffprobe_bin};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Start time and duration of the first stream of a kind, in seconds.
pub(crate) fn stream_times(path: &Path, kind: &str) -> Option<(f64, f64)> {
    let out = crate::proc::command(ffprobe_bin())
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
    let mut f = String::from(ALIGN_BASE);
    if lead > LEAD_TOLERANCE_S {
        f.push_str(&format!(",adelay={:.0}:all=1", lead * 1000.0));
    } else if lead < -LEAD_TOLERANCE_S {
        f.push_str(&format!(",atrim=start={:.3},{ALIGN_REBASE}", -lead));
    }
    f.push_str(",apad");
    Some(f)
}

/// Samples are laid out by their count, never by their (jittery) packet
/// stamps, at the 48 kHz every export and player handles well.
const ALIGN_REBASE: &str = "asetpts=N/SR/TB";
const ALIGN_BASE: &str = "aresample=48000,asetpts=N/SR/TB";

/// The filter for audio that needs no alignment but still has to be brought
/// to 48 kHz and padded to the end of the video.
fn audio_plain_filter() -> String {
    format!("{ALIGN_BASE},apad")
}

/// Sample rate of the first audio stream.
pub(crate) fn audio_rate(path: &Path) -> Option<u32> {
    let out = crate::proc::command(ffprobe_bin())
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=sample_rate", "-of", "csv=p=0"])
        .arg(ffmpeg::file_url(path))
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
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
    /// Things the user should hear about (a microphone that could not be
    /// joined, or only roughly).
    pub warnings: Vec<String>,
}

pub fn remux_to_mp4(mkv: &Path) -> Result<PathBuf> {
    Ok(remux_to_mp4_info(mkv)?.path)
}

/// The separate microphone file that goes with `recording.mkv`, when the
/// capture recorded one.
pub fn mic_sidecar(mkv: &Path) -> PathBuf {
    mkv.with_extension("mic.mkv")
}

/// The separate system audio file that goes with `recording.mkv`, when the
/// capture recorded one (Windows and macOS; Linux mixes it while recording).
pub fn sys_sidecar(mkv: &Path) -> PathBuf {
    mkv.with_extension("sys.mkv")
}

/// The ffmpeg filter graph that mixes the microphone (input 0) and the system
/// audio (input 1) on their shared clock: the one that started later is
/// delayed by the difference, so both stay where they happened. The system
/// sound is a little lower than the voice, and a limiter keeps the sum from
/// clipping. A start far outside the recording's clock (over 30 s apart) is
/// treated as unaligned and starts with the other.
pub fn system_mix_filter(mic_start: f64, sys_start: f64) -> String {
    let (mut m, mut s) = (mic_start, sys_start);
    if (m - s).abs() > 30.0 {
        if m < s { s = m } else { m = s }
    }
    let base = m.min(s);
    let ms = |t: f64| ((t - base).max(0.0) * 1000.0).round() as i64;
    let fmt = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB";
    format!("[0:a]{fmt},adelay={}:all=1[m];[1:a]{fmt},volume=0.8,adelay={}:all=1[s];[m][s]amix=inputs=2:duration=longest:normalize=0,alimiter=limit=0.97[o]", ms(m), ms(s))
}

/// Folds the system audio sidecar into the microphone sidecar, so the remux
/// only ever sees one audio file. With no microphone, the system audio takes
/// its place. Returns false when there was no system audio to add (nothing
/// played). On an error the microphone file is left as it was.
pub fn merge_system_audio(mkv: &Path) -> Result<bool> {
    let sys = sys_sidecar(mkv);
    let big = |p: &Path| std::fs::metadata(p).map(|m| m.len() > 4096).unwrap_or(false);
    if !big(&sys) {
        let _ = std::fs::remove_file(&sys);
        return Ok(false);
    }
    let mic = mic_sidecar(mkv);
    if !big(&mic) {
        std::fs::rename(&sys, &mic).context("could not move the system audio into place")?;
        return Ok(true);
    }
    let (m, s) = (stream_times(&mic, "a:0"), stream_times(&sys, "a:0"));
    let (Some(m), Some(s)) = (m, s) else { bail!("the {} file could not be read", if m.is_none() { "microphone" } else { "system audio" }) };
    let tmp = mic.with_extension("mkv.part");
    let base = m.0.min(s.0);
    let out = crate::proc::command(ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(&mic)
        .arg("-i")
        .arg(&sys)
        .args(["-filter_complex", &system_mix_filter(m.0, s.0), "-map", "[o]", "-c:a", "pcm_f32le", "-output_ts_offset", &format!("{:.3}", if (m.0 - s.0).abs() > 30.0 { m.0 } else { base }), "-f", "matroska"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .output()
        .context("failed to start ffmpeg to mix the system audio")?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("ffmpeg could not mix the system audio: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    std::fs::rename(&tmp, &mic)?;
    let _ = std::fs::remove_file(&sys);
    Ok(true)
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
    match remux_inner(mkv, Join::Aligned) {
        Err(e) if has_sidecar(mkv) => {
            // Never lose the voice silently: first try again without the
            // fine alignment, then without the microphone (keeping its file).
            eprintln!("demolens: joining the microphone failed: {e:#}");
            match remux_inner(mkv, Join::Plain) {
                Ok(mut r) => {
                    r.warnings.push(format!("The microphone could not be lined up with the video exactly ({}), so it was added as it was recorded and may be a little off.", short(&e)));
                    Ok(r)
                }
                Err(e2) => match remux_inner(mkv, Join::Skip) {
                    Ok(mut r) => {
                        r.warnings.push(format!("Mic not recorded in the video: it could not be joined ({}). The raw microphone file is kept at {}.", short(&e2), mic_sidecar(mkv).display()));
                        Ok(r)
                    }
                    Err(_) => Err(e),
                },
            }
        }
        r => r,
    }
}

fn short(e: &anyhow::Error) -> String {
    let t = format!("{e:#}");
    t.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error").chars().take(160).collect()
}

fn has_sidecar(mkv: &Path) -> bool {
    std::fs::metadata(mic_sidecar(mkv)).map(|m| m.len() > 4096).unwrap_or(false)
}

#[derive(Clone, Copy, PartialEq)]
enum Join {
    /// The microphone is lined up on the shared clock.
    Aligned,
    /// The microphone is resampled and added with no alignment.
    Plain,
    /// Video only; the microphone file stays where it is.
    Skip,
}

fn remux_inner(mkv: &Path, join: Join) -> Result<Remuxed> {
    let mp4 = mkv.with_extension("mp4");
    let tmp = mkv.with_extension("mp4.part");
    let _ = std::fs::remove_file(&tmp);
    let video = stream_times(mkv, "v:0");
    let side = mic_sidecar(mkv);
    let has_side = has_sidecar(mkv) && join != Join::Skip;
    let audio = if has_side { stream_times(&side, "a:0") } else { stream_times(mkv, "a:0") };
    if has_side && audio.is_none() {
        // Never drop the voice without saying so: the caller reports this.
        bail!("the microphone file could not be read");
    }
    let mut cmd = crate::proc::command(ffmpeg_bin());
    cmd.args(["-v", "error", "-nostdin", "-y"]);
    let mut video_start_s = 0.0;
    if let Some((vs, _)) = video {
        video_start_s = vs.max(0.0);
    }
    let mut align: Option<String> = None;
    if let (true, Some(v), Some(a)) = (has_side, video, audio) {
        // Video and microphone are separate files on the same clock. Silence
        // goes in front of a microphone that opened late; the video is not
        // touched.
        let a_start = if (a.0 - v.0).abs() > 30.0 + v.1 { v.0 } else { a.0 };
        let mut f = audio_align_filter(v, (a_start, a.1)).unwrap_or_else(audio_plain_filter);
        if join == Join::Plain {
            f = audio_plain_filter();
        } else if a_start < v.0 && !f.contains("atrim") {
            f = format!("{ALIGN_BASE},atrim=start={:.3},{ALIGN_REBASE},apad", v.0 - a_start);
        }
        align = Some(f);
        cmd.arg("-i").arg(ffmpeg::file_url(mkv)).arg("-i").arg(ffmpeg::file_url(&side));
        cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
    } else if let (Some(v), Some(a)) = (video, audio) {
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
            if align.is_none() && audio_rate(mkv).is_some_and(|r| r != 48_000) {
                align = Some(audio_plain_filter());
            }
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
    if join != Join::Skip {
        let _ = std::fs::remove_file(&side);
    }
    Ok(Remuxed { path: mp4, video_start_s, warnings: Vec::new() })
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
    /// Sample rate of the audio track.
    pub audio_rate: Option<u32>,
    /// Stretches of exact digital silence in the middle of the sound (lost samples).
    pub dropouts: Option<usize>,
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
        audio_rate: None,
        dropouts: None,
        issues: Vec::new(),
    };
    if a.is_some() {
        r.audio_rate = audio_rate(video);
        if let Ok(samples) = crate::audio::decode_mono(video, 16_000) {
            let d = crate::audio::dropouts(&samples, 16_000, 20.0);
            if d.len() >= 2 {
                let worst = d.iter().map(|x| x.1).fold(0.0, f64::max);
                r.issues.push(format!("The audio has {} dropouts (the longest {worst:.0} ms, the first at {:.1} s). Speech will sound broken up.", d.len(), d[0].0 / 1000.0));
            }
            r.dropouts = Some(d.len());
        }
    }
    if let Some(rate) = r.audio_rate.filter(|r| *r != 48_000 && *r != 44_100) {
        r.issues.push(format!("The audio is {rate} Hz, not 48000 Hz."));
    }
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
        let ok = crate::proc::command(ffmpeg_bin())
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
        let ok = crate::proc::command(ffmpeg_bin())
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
    fn a_separate_microphone_file_is_joined_at_48k_without_gaps() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("remux-sidecar");
        let mkv = d.join("recording.mkv");
        // The screen has no audio. The microphone was recorded on its own at
        // 96 kHz, opened 0.8 s late, with jittery packet stamps like a wall
        // clock stamped device gives.
        let ok = crate::proc::command(ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an"])
            .arg(&mkv)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let ok = crate::proc::command(ffmpeg_bin())
            .args(["-v", "error", "-y", "-itsoffset", "0.8", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=96000:d=2.1", "-af", "asetpts='PTS+0.003*sin(N)/TB'", "-c:a", "pcm_f32le"])
            .arg(mic_sidecar(&mkv))
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let r = remux_to_mp4_info(&mkv).unwrap();
        assert!(!mic_sidecar(&mkv).exists(), "the sidecar is removed once it is joined");
        let rep = sync_report(&r.path, None).unwrap();
        assert_eq!(rep.audio_rate, Some(48_000));
        assert!(rep.audio_offset_ms.unwrap().abs() < 100.0, "{rep:?}");
        assert_eq!(rep.dropouts, Some(0), "{rep:?}");
        assert!(rep.issues.is_empty(), "{:?}", rep.issues);
        let x = crate::audio::decode_mono(&r.path, 16_000).unwrap();
        let rms = |a: f64, b: f64| ((x[(a * 16000.0) as usize..(b * 16000.0) as usize].iter().map(|v| v * v).sum::<f32>()) / ((b - a) * 16000.0) as f32).sqrt();
        assert!(rms(0.1, 0.7) < 0.001, "silence in front of the late microphone");
        assert!(rms(1.0, 2.8) > 0.05, "the sound is where it was recorded");
        assert!(crate::audio::dropouts(&x, 16_000, 5.0).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_microphone_file_that_cannot_be_joined_is_reported_and_kept() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("remux-badmic");
        let mkv = d.join("recording.mkv");
        let ok = crate::proc::command(ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an"])
            .arg(&mkv)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        // Big enough to look like a recording, but not audio.
        std::fs::write(mic_sidecar(&mkv), vec![7u8; 20_000]).unwrap();
        let r = remux_to_mp4_info(&mkv).unwrap();
        assert!(r.path.exists(), "the video still comes out");
        assert!(r.warnings.iter().any(|w| w.starts_with("Mic not recorded")), "{:?}", r.warnings);
        assert!(mic_sidecar(&mkv).exists(), "the raw microphone file is kept for recovery");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_plain_join_adds_the_microphone_without_alignment() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("remux-plain");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(run(&["-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an"], &mkv));
        assert!(run(&["-itsoffset", "0.5", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:d=2", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        let r = remux_inner(&mkv, Join::Plain).unwrap();
        let rep = sync_report(&r.path, None).unwrap();
        assert_eq!(rep.audio_rate, Some(48_000));
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

    #[test]
    fn system_audio_is_mixed_onto_the_voice_on_the_shared_clock() {
        let d = tmpdir("sysmix");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| {
            crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success()
        };
        // Voice: 2 s of 440 Hz stamped from 0.5 s. System sound: 3 s of 880 Hz stamped from 1.0 s.
        assert!(run(&["-itsoffset", "0.5", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:d=2", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        assert!(run(&["-itsoffset", "1.0", "-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000:d=3", "-c:a", "pcm_f32le"], &sys_sidecar(&mkv)));
        assert!(merge_system_audio(&mkv).unwrap());
        assert!(!sys_sidecar(&mkv).exists(), "the system file is folded in");
        let (start, dur) = stream_times(&mic_sidecar(&mkv), "a:0").unwrap();
        assert!((start - 0.5).abs() < 0.06, "starts with the voice: {start}");
        // (Matroska reports the end time as the duration of a file that starts late.)
        assert!((dur - 4.0).abs() < 0.2, "ends with the system sound: {dur}");
        // Both are really in the mix, where they belong (times here count from the file start). Mean level: the voice alone first (a mono sine
        // placed in a stereo mix reads about -24 dB), both together while they overlap (about 2 dB more),
        // then the system sound alone (0.8 of the level, about -26 dB).
        let level = |from: &str, to: &str| {
            let o = crate::proc::command(ffmpeg_bin())
                .args(["-v", "info", "-i"]).arg(mic_sidecar(&mkv))
                .args(["-af", &format!("atrim={from}:{to},volumedetect"), "-f", "null", "-"]).output().unwrap();
            let text = String::from_utf8_lossy(&o.stderr).to_string();
            let v = text.lines().find_map(|l| l.split("mean_volume:").nth(1)).map(|r| r.trim().trim_end_matches(" dB").parse::<f32>().unwrap());
            v.unwrap_or(-99.0)
        };
        let (early, both, late) = (level("0.1", "0.45"), level("0.7", "1.8"), level("2.2", "3.2"));
        assert!((early + 24.1).abs() < 1.2, "the voice alone: {early}");
        assert!(both > early + 1.2, "voice and system sound together: {both} vs {early}");
        assert!((late + 26.0).abs() < 1.2, "the system sound alone, a little lower: {late}");
    }

    #[test]
    fn system_audio_alone_becomes_the_audio_and_an_empty_one_is_dropped() {
        let d = tmpdir("sysonly");
        let mkv = d.join("recording.mkv");
        std::fs::write(sys_sidecar(&mkv), vec![9u8; 20_000]).unwrap();
        assert!(merge_system_audio(&mkv).unwrap());
        assert_eq!(std::fs::metadata(mic_sidecar(&mkv)).unwrap().len(), 20_000);
        assert!(!sys_sidecar(&mkv).exists());
        // Nothing played: a stub file is removed and the voice is untouched.
        std::fs::write(sys_sidecar(&mkv), vec![1u8; 100]).unwrap();
        assert!(!merge_system_audio(&mkv).unwrap());
        assert!(!sys_sidecar(&mkv).exists() && mic_sidecar(&mkv).exists());
    }

    #[test]
    fn the_mix_delays_whichever_started_later() {
        let f = system_mix_filter(0.5, 1.0);
        assert!(f.contains("adelay=0:all=1[m]") && f.contains("adelay=500:all=1[s]"), "{f}");
        let f = system_mix_filter(2.0, 1.0);
        assert!(f.contains("adelay=1000:all=1[m]") && f.contains("adelay=0:all=1[s]"), "{f}");
        let far = system_mix_filter(1.0, 5000.0);
        assert!(far.contains("adelay=0:all=1[m]") && far.contains("adelay=0:all=1[s]"), "an unaligned clock starts with the other: {far}");
    }
}
