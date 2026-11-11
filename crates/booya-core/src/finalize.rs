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
    let dur = stream_duration(st, &v["format"], start);
    Some((start, dur))
}

fn stream_duration(stream: &serde_json::Value, format: &serde_json::Value, start: f64) -> f64 {
    stream["duration"].as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| {
        Some((format["duration"].as_str()?.parse::<f64>().ok()? - start).max(0.0))
    }).unwrap_or(0.0)
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

/// The length of the audio as its samples add up (count over rate), which
/// differs from the container's timestamp span when the capture device lost
/// samples while the packet stamps kept following the wall clock.
pub(crate) fn sample_duration(path: &Path, stream: &str) -> Option<f64> {
    let out = crate::proc::command(ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(ffmpeg::file_url(path))
        .args(["-map", stream, "-af", "aresample=48000,asetpts=N/SR/TB", "-f", "null", "-progress", "pipe:1", "-"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let us = text.lines().rev().find_map(|l| l.strip_prefix("out_time_us=").or_else(|| l.strip_prefix("out_time_ms=")).and_then(|v| v.trim().parse::<f64>().ok()))?;
    (us > 0.0).then_some(us / 1e6)
}

/// The atempo factor that stretches audio whose samples add up to `samples`
/// seconds back over the `span` seconds it really took. Only a substantial
/// shortfall (2.5% to 20%) is corrected; smaller differences are left alone.
pub fn drift_tempo(samples: f64, span: f64) -> Option<f64> {
    if samples <= 0.0 || span < 5.0 {
        return None;
    }
    let ratio = samples / span;
    (0.8..=0.975).contains(&ratio).then_some(ratio)
}

fn track_drift_tempo(samples: f64, video: (f64, f64), audio_start: f64) -> Option<f64> {
    let trim = (video.0 - audio_start).max(0.0);
    let remaining_samples = (samples - trim).max(0.0);
    let span = (video.0 + video.1 - audio_start.max(video.0)).max(0.0);
    drift_tempo(remaining_samples, span)
}

fn with_rate(filter: String, rate: Option<String>) -> String {
    match rate {
        Some(r) => filter.replacen(ALIGN_BASE, &format!("{ALIGN_BASE},{r},{ALIGN_REBASE}"), 1),
        None => filter,
    }
}

fn with_tempo(filter: String, tempo: Option<f64>) -> String {
    match tempo {
        Some(t) => filter.replacen(ALIGN_BASE, &format!("{ALIGN_BASE},atempo={t:.6},{ALIGN_REBASE}"), 1),
        None => filter,
    }
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

/// The packet log written next to a microphone file (see `capture::mic`).
pub fn mic_timing(mic: &Path) -> PathBuf {
    mic.with_extension("timing")
}

/// Where a microphone recording really sits on the recording clock.
#[derive(Debug, Clone, PartialEq)]
pub struct MicClock {
    /// Seconds after the recording clock started at which the first sample
    /// was taken.
    pub start_s: f64,
    /// Seconds of samples per second of wall clock: below 1 the microphone
    /// delivered fewer samples than the time that passed (a slow clock or
    /// lost buffers), above 1 more.
    pub ratio: f64,
    /// Wall-clock seconds the packets cover.
    pub span_s: f64,
    /// Stretches where the microphone delivered nothing and the sound was
    /// lost (a stalled device, or buffers dropped while the reader was
    /// busy): where in the samples each one sits (seconds of samples from
    /// the first one) and how long it lasted on the wall clock. The file is
    /// laid out by sample count, so without putting these back every word
    /// after a stall plays that much early.
    pub gaps: Vec<(f64, f64)>,
}

impl MicClock {
    /// Wall-clock seconds lost to stalls in total.
    pub fn lost_s(&self) -> f64 {
        self.gaps.iter().map(|g| g.1).sum()
    }
}

/// A stall shorter than this is read jitter, not lost sound (the same limit
/// the in-process recorders use, see `capture::audiosink::GapFiller`).
const MIC_GAP_S: f64 = 0.15;
/// At most this many stalls are put back; more means a broken device and
/// the filter would get unwieldy.
const MIC_MAX_GAPS: usize = 200;

/// Stalls in a packet log, from `(seconds of samples up to the end of the
/// packet, read time)` points. `read - samples` is the start of the
/// recording plus that packet's delay plus all the sound lost so far. Delays
/// come and go, lost sound stays: the smallest value from each packet on
/// (a lower envelope from the right) only rises where sound was lost, and
/// each rise of more than [`MIC_GAP_S`] between two packets is one stall
/// that sits just before the later packet's samples. A rise in the last
/// second is not trusted (the reads on stop are late, nothing follows them).
fn find_mic_gaps(xy: &[(f64, f64)], sizes_s: &[f64]) -> Vec<(f64, f64)> {
    let n = xy.len();
    if n < 2 {
        return Vec::new();
    }
    let mut env = vec![0.0; n];
    let mut m = f64::INFINITY;
    for i in (0..n).rev() {
        m = m.min(xy[i].1 - xy[i].0);
        env[i] = m;
    }
    let x_end = xy[n - 1].0;
    let mut gaps = Vec::new();
    let mut seg = 0;
    for i in 1..n {
        let at = xy[i].0 - sizes_s[i];
        if env[i] - env[i - 1] > MIC_GAP_S && x_end - at >= 1.0 {
            // How much was lost is measured from the quickest read in the
            // second before the stall, not from the one packet right before
            // it, which may have been read a little late itself.
            let before = (seg..i).rev().take_while(|&j| at - xy[j].0 < 1.0).map(|j| xy[j].1 - xy[j].0).fold(f64::INFINITY, f64::min);
            gaps.push((at, env[i] - before.min(env[i - 1])));
            seg = i;
        }
    }
    if gaps.len() > MIC_MAX_GAPS {
        gaps.sort_by(|a, b| b.1.total_cmp(&a.1));
        gaps.truncate(MIC_MAX_GAPS);
        gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    gaps
}

/// Wall-clock seconds lost before `x` seconds of samples.
fn lost_before(gaps: &[(f64, f64)], x: f64) -> f64 {
    gaps.iter().filter(|g| g.0 < x).map(|g| g.1).sum()
}

/// The filter that puts the stalls of a microphone back as silence, for
/// audio laid out by sample count from 0 at 48 kHz (after `ALIGN_BASE`).
/// Every sample after a stall is moved later by the sound lost before it,
/// and the resampler fills the holes that leaves with silence.
pub fn mic_gap_filter(gaps: &[(f64, f64)]) -> Option<String> {
    if gaps.is_empty() {
        return None;
    }
    let terms: String = gaps.iter().map(|(at, len)| format!("+{len:.4}*gte(N/SR,{at:.4})")).collect();
    Some(format!("asetpts='(N/SR{terms})/TB',aresample=48000:async=1:first_pts=0,{ALIGN_REBASE}"))
}

/// Everything the packet log says to fix in a microphone file, in order:
/// its stalls, then its clock rate. None when nothing needs fixing.
pub fn mic_fix_filter(c: &MicClock) -> Option<String> {
    let parts: Vec<String> = [mic_gap_filter(&c.gaps), clock_rate_filter(c.ratio)].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

fn pcm_bytes(codec: &str) -> Option<usize> {
    let c = codec.trim().strip_prefix("pcm_")?;
    let digits: String = c.chars().skip_while(|ch| !ch.is_ascii_digit()).take_while(|ch| ch.is_ascii_digit()).collect();
    let bits: usize = digits.parse().ok()?;
    (bits > 0 && bits.is_multiple_of(8)).then_some(bits / 8)
}

fn layout_channels(name: &str) -> Option<usize> {
    let n = name.trim();
    if let Some(k) = n.strip_suffix(" channels").and_then(|k| k.trim().parse().ok()) {
        return Some(k);
    }
    Some(match n {
        "mono" => 1,
        "stereo" | "downmix" => 2,
        "2.1" | "3.0" | "3.0(back)" => 3,
        "4.0" | "quad" | "quad(side)" | "3.1" => 4,
        "5.0" | "5.0(side)" | "4.1" => 5,
        "5.1" | "5.1(side)" | "6.0" | "hexagonal" => 6,
        "6.1" | "7.0" => 7,
        "7.1" | "7.1(wide)" | "octagonal" => 8,
        _ => return None,
    })
}

/// Reads a microphone's packet log (ffmpeg `framecrc`: read time and size of
/// every packet) into where its first sample was taken and how its sample
/// clock ran against the wall clock.
///
/// A packet is read some time after its last sample was captured, never
/// before. So `read time - samples so far / rate` is the start of the
/// recording plus that packet's delay. A straight line through those points
/// gives the rate, and the smallest delay gives the start. The first
/// packet's read time alone (what the file carries) is late by at least that
/// packet's length and by whatever the device had buffered before the first
/// read, which differs from one recording to the next.
pub fn parse_mic_timing(text: &str) -> Option<MicClock> {
    let (mut tb, mut rate, mut bytes, mut channels) = (None, None, None, None);
    let mut pts: Vec<f64> = Vec::new();
    let mut sizes: Vec<f64> = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if let Some(h) = l.strip_prefix('#') {
            let Some((k, v)) = h.split_once(':') else { continue };
            let v = v.trim();
            match k.trim() {
                "tb 0" => {
                    let (a, b) = v.split_once('/')?;
                    tb = Some(a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?);
                }
                "sample_rate 0" => rate = v.parse::<f64>().ok(),
                "codec_id 0" => bytes = pcm_bytes(v),
                "channel_layout_name 0" => channels = layout_channels(v),
                _ => {}
            }
            continue;
        }
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        if f.len() < 5 || f[0] != "0" {
            continue;
        }
        let (Ok(p), Ok(sz)) = (f[1].parse::<f64>(), f[4].parse::<f64>()) else { continue };
        pts.push(p);
        sizes.push(sz);
    }
    let (tb, rate, frame) = (tb?, rate?, (bytes? * channels?) as f64);
    if pts.len() < 8 || rate <= 0.0 || frame <= 0.0 {
        return None;
    }
    // x: seconds of samples up to the end of each packet; y: its read time.
    let mut cum = 0.0;
    let sizes_s: Vec<f64> = sizes.iter().map(|sz| sz / frame / rate).collect();
    let raw: Vec<(f64, f64)> = pts.iter().zip(&sizes_s).map(|(p, sz)| {
        cum += sz;
        (cum, p * tb)
    }).collect();
    let span = raw[raw.len() - 1].1 - raw[0].1;
    // Sound lost to stalls comes out first: it is a step, not a rate, and a
    // straight line through a step is wrong on both sides of it.
    let gaps = find_mic_gaps(&raw, &sizes_s);
    let xy: Vec<(f64, f64)> = raw.iter().map(|&(x, y)| (x, y - lost_before(&gaps, x))).collect();
    // Wall seconds per second of samples, from the lower edge of the delays:
    // the quickest read in each tenth of the take. Read delays come and go
    // (a busy machine adds tens of ms), the quickest read in a stretch does
    // not, so a line through those is the clock and not the load. A take
    // under 30 s cannot tell a rate apart from that noise and is taken as
    // exact.
    let mut slope = 1.0;
    if span >= 30.0 {
        let x_end = xy[xy.len() - 1].0;
        let mut lows: Vec<(f64, f64)> = Vec::new();
        for b in 0..10 {
            let (lo, hi) = (x_end * b as f64 / 10.0, x_end * (b + 1) as f64 / 10.0);
            if let Some(p) = xy.iter().filter(|v| v.0 > lo && v.0 <= hi).min_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0))) {
                lows.push(*p);
            }
        }
        let n = lows.len() as f64;
        if n >= 6.0 {
            let (mx, my) = (lows.iter().map(|v| v.0).sum::<f64>() / n, lows.iter().map(|v| v.1).sum::<f64>() / n);
            let sxx: f64 = lows.iter().map(|v| (v.0 - mx).powi(2)).sum();
            if sxx > 0.0 {
                slope = lows.iter().map(|v| (v.0 - mx) * (v.1 - my)).sum::<f64>() / sxx;
            }
        }
    }
    if !(0.8..=1.25).contains(&slope) {
        slope = 1.0;
    }
    let start = xy.iter().map(|(x, y)| y - x * slope).fold(f64::INFINITY, f64::min);
    start.is_finite().then_some(MicClock { start_s: start, ratio: 1.0 / slope, span_s: span, gaps })
}

/// The clock of a microphone file from its packet log, when there is one.
pub fn mic_clock(mic: &Path) -> Option<MicClock> {
    parse_mic_timing(&std::fs::read_to_string(mic_timing(mic)).ok()?)
}

/// The filter that plays samples recorded at `ratio` seconds of samples per
/// wall-clock second back over the wall-clock time they really took. A
/// difference under 0.05% is left alone (under 30 ms in a minute). Up to 1%
/// the samples are only relabelled (a pitch change nobody hears, and no
/// artefacts); beyond that the tempo is changed without changing the pitch.
pub fn clock_rate_filter(ratio: f64) -> Option<String> {
    if !(0.8..=1.25).contains(&ratio) || (ratio - 1.0).abs() < 0.0005 {
        return None;
    }
    if (ratio - 1.0).abs() <= 0.01 {
        Some(format!("asetrate={:.3},aresample=48000", 48_000.0 * ratio))
    } else {
        Some(format!("atempo={ratio:.6}"))
    }
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
    system_mix_filter_rate(mic_start, sys_start, None)
}

/// The microphone and system audio starts the mix uses. Starts more than
/// 30 s apart are treated as unaligned and both take the earlier one, so the
/// two tracks are overlaid from the start of each file.
fn system_mix_starts(mic_start: f64, sys_start: f64) -> (f64, f64) {
    if (mic_start - sys_start).abs() > 30.0 {
        let t = mic_start.min(sys_start);
        (t, t)
    } else {
        (mic_start, sys_start)
    }
}

/// Where the mixed file starts on the recording clock: the earlier of the
/// two starts the filter used. The mixed file is stamped with this, so a
/// mix that collapsed unaligned starts is not then pushed to the later one.
pub fn system_mix_start(mic_start: f64, sys_start: f64) -> f64 {
    let (m, s) = system_mix_starts(mic_start, sys_start);
    m.min(s)
}

/// Like [`system_mix_filter`], with an optional rate correction applied to
/// the microphone (input 0) before it is delayed and mixed. The correction
/// is the same filter [`clock_rate_filter`] builds, so a slow or fast
/// microphone clock is stretched over the wall-clock time it really took.
pub fn system_mix_filter_rate(mic_start: f64, sys_start: f64, mic_rate: Option<&str>) -> String {
    let (m, s) = system_mix_starts(mic_start, sys_start);
    let base = system_mix_start(mic_start, sys_start);
    let ms = |t: f64| ((t - base).max(0.0) * 1000.0).round() as i64;
    let fmt = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB";
    let rate = mic_rate.filter(|r| !r.is_empty()).map(|r| format!(",{r}")).unwrap_or_default();
    format!("[0:a]{fmt}{rate},adelay={}:all=1[m];[1:a]{fmt},volume=0.8,adelay={}:all=1[s];[m][s]amix=inputs=2:duration=longest:normalize=0,alimiter=limit=0.97[o]", ms(m), ms(s))
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
        // Move the system audio in first. If that fails, the microphone file
        // and its log are left as they were.
        std::fs::rename(&sys, &mic).context("could not move the system audio into place")?;
        let _ = std::fs::remove_file(mic_timing(&mic));
        return Ok(true);
    }
    let (m, s) = (stream_times(&mic, "a:0"), stream_times(&sys, "a:0"));
    let (Some(mut m), Some(s)) = (m, s) else { bail!("the {} file could not be read", if m.is_none() { "microphone" } else { "system audio" }) };
    // The voice's real start and rate from its packet log. The mixed file
    // replaces the microphone file, so the log no longer describes it
    // afterwards. The rate used to be ignored here, and the log was
    // deleted, so the remux could not correct it either.
    let mut rate = None;
    let mut clock_used = false;
    if let Some(c) = mic_clock(&mic).filter(|c| (c.start_s - m.0).abs() < 5.0) {
        m.0 = c.start_s;
        rate = mic_fix_filter(&c);
        clock_used = true;
    }
    let tmp = mic.with_extension("mkv.part");
    let out = crate::proc::command(ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(&mic)
        .arg("-i")
        .arg(&sys)
        .args(["-filter_complex", &system_mix_filter_rate(m.0, s.0, rate.as_deref()), "-map", "[o]", "-c:a", "pcm_f32le", "-output_ts_offset", &format!("{:.3}", system_mix_start(m.0, s.0)), "-f", "matroska"])
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
    // The log describes the raw microphone file. Once its start, gaps and
    // rate are in the mix it no longer matches, so it goes. When it was
    // rejected (its start too far from the file's), it is kept, so it is
    // still there for `audio.log` and for the remux to judge again.
    if clock_used {
        let _ = std::fs::remove_file(mic_timing(&mic));
    }
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
            eprintln!("booya: joining the microphone failed: {e:#}");
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
        // The packet log knows where the first sample really was and how
        // the microphone's clock ran; the file's own start is only the
        // moment its first packet was read.
        let clock = mic_clock(&side).filter(|c| (c.start_s - a.0).abs() < 5.0);
        let a = match &clock { Some(c) => (c.start_s, a.1), None => a };
        let a_start = if (a.0 - v.0).abs() > 30.0 + v.1 { v.0 } else { a.0 };
        let mut f = audio_align_filter(v, (a_start, a.1)).unwrap_or_else(audio_plain_filter);
        if join == Join::Plain {
            f = audio_plain_filter();
        } else if a_start < v.0 && !f.contains("atrim") {
            f = format!("{ALIGN_BASE},atrim=start={:.3},{ALIGN_REBASE},apad", v.0 - a_start);
        }
        align = Some(match clock {
            Some(c) if join == Join::Aligned => with_rate(f, mic_fix_filter(&c)),
            Some(_) => f,
            None => {
                let tempo = sample_duration(&side, "0:a:0").and_then(|n| track_drift_tempo(n, v, a_start));
                with_tempo(f, tempo)
            }
        });
        cmd.arg("-i").arg(ffmpeg::file_url(mkv)).arg("-i").arg(ffmpeg::file_url(&side));
        cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
    } else if let (Some(v), Some(a)) = (video, audio) {
        let (vs, vd) = v;
        let a_start = a.0;
        // An audio clock far away from the video's means it was not aligned.
        if (a_start - vs).abs() > 30.0 + vd {
            eprintln!("booya: audio starts at {a_start:.1}s but video at {vs:.1}s, realigning");
            cmd.arg("-i").arg(ffmpeg::file_url(mkv));
            cmd.args(["-itsoffset", &format!("{:.3}", vs - a_start), "-i"]).arg(ffmpeg::file_url(mkv));
            cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
        } else {
            let tempo = sample_duration(mkv, "0:a:0").and_then(|n| track_drift_tempo(n, v, a_start));
            align = audio_align_filter(v, a);
            if align.is_none() && tempo.is_some() {
                align = Some(audio_plain_filter());
            }
            align = align.map(|f| with_tempo(f, tempo));
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
        let _ = std::fs::remove_file(mic_timing(&side));
    }
    Ok(Remuxed { path: mp4, video_start_s, warnings: Vec::new() })
}

/// How a recording's tracks line up. Used by `booya verify`.
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

    #[test]
    fn a_container_duration_fallback_excludes_a_late_stream_start() {
        let stream = serde_json::json!({});
        let format = serde_json::json!({"duration": "10.0"});
        assert!((stream_duration(&stream, &format, 0.35) - 9.65).abs() < 1e-9);

        let format = serde_json::json!({"duration": "4.0"});
        assert!((stream_duration(&stream, &format, 0.5) - 3.5).abs() < 1e-9);
    }

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
    fn drift_is_stretched_back_over_the_recorded_span() {
        // 61 s of samples that took 67 s: slowed down to fit.
        let slow = drift_tempo(61.0, 67.0).unwrap();
        assert!((slow - 0.9104).abs() < 0.001);
        assert!(drift_tempo(65.5, 67.0).is_none());
        assert!(drift_tempo(30.0, 67.0).is_none());
        assert!(drift_tempo(53.5, 54.0).is_none());
        let f = with_tempo(audio_plain_filter(), Some(slow));
        assert!(f.starts_with("aresample=48000,asetpts=N/SR/TB,atempo=0.91"), "{f}");
    }

    #[test]
    fn drift_uses_the_video_span_after_alignment() {
        let t = track_drift_tempo(48.4, (0.0, 54.0), 0.0).unwrap();
        assert!((t - 48.4 / 54.0).abs() < 1e-9);

        let t = track_drift_tempo(48.4, (0.0, 54.0), 0.35).unwrap();
        assert!((t - 48.4 / 53.65).abs() < 1e-9);

        assert!(track_drift_tempo(54.4, (0.0, 54.0), -0.4).is_none());
    }

    #[test]
    fn a_short_mic_stream_is_stretched_to_the_video_span() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("remux-short-mic");
        let mkv = d.join("recording.mkv");
        let reference = d.join("reference.mkv");
        crate::syncprobe::make_reference(&mkv, 54, true, false, 0.0).unwrap();
        crate::syncprobe::make_reference(&reference, 54, false, true, 0.0).unwrap();
        let side = mic_sidecar(&mkv);
        let ok = crate::proc::command(ffmpeg_bin())
            .args(["-v", "error", "-y", "-i"])
            .arg(&reference)
            .args(["-vn", "-af", "atempo=1.1157024793", "-c:a", "pcm_f32le", "-f", "matroska"])
            .arg(&side)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let raw_duration = sample_duration(&side, "0:a:0").unwrap();
        assert!((raw_duration - 48.4).abs() < 0.3, "{raw_duration}");

        let joined = remux_to_mp4_info(&mkv).unwrap();
        let result = crate::syncprobe::measure(&joined.path).unwrap();
        assert!(result.pairs >= 50, "{result:?}");
        assert!(result.offset_ms.abs() < 40.0, "{result:?}");
        assert!(result.worst_ms < 50.0, "{result:?}");
        assert!(result.drift_total_ms.abs() < 75.0, "{result:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Packets of 1024 stereo 16 bit frames at 48 kHz from a microphone whose
    /// first sample was taken at 1.000 s and whose clock runs `slope` wall
    /// seconds per second of samples. The first 300 ms were buffered by the
    /// device and read in one go; every later read comes 2 to 40 ms late.
    fn timing_text(slope: f64, secs: f64) -> String {
        let mut t = String::from("#software: Lavf61\n#tb 0: 1/1000000\n#media_type 0: audio\n#codec_id 0: pcm_s16le\n#sample_rate 0: 48000\n#channel_layout_name 0: stereo\n");
        let (mut cum, mut k) = (0.0f64, 0u64);
        while cum < secs {
            cum += 1024.0 / 48_000.0;
            k += 1;
            let jitter = 0.002 + 0.038 * (((k * 7919) % 101) as f64 / 100.0);
            let read = (1.0 + cum * slope + jitter).max(1.0 + 0.3 * slope + 0.002);
            t.push_str(&format!("0, {:>10}, {:>10}, 21333, 4096, 0x00000000\n", (read * 1e6).round() as i64, (read * 1e6).round() as i64));
        }
        t
    }

    #[test]
    fn the_packet_log_finds_the_first_sample_and_the_clock_rate() {
        let c = parse_mic_timing(&timing_text(1.0, 8.0)).unwrap();
        assert!((c.start_s - 1.002).abs() < 0.003, "{c:?}");
        assert_eq!(c.ratio, 1.0, "a short take is not given a rate: {c:?}");
        // A microphone clock 0.2% slow over two minutes is 240 ms by the end.
        let c = parse_mic_timing(&timing_text(1.002, 120.0)).unwrap();
        assert!((c.ratio - 1.0 / 1.002).abs() < 0.0002, "{c:?}");
        assert!((c.start_s - 1.0).abs() < 0.02, "{c:?}");
        assert!(parse_mic_timing("#tb 0: 1/1000000\n").is_none());
        assert_eq!(pcm_bytes("pcm_f32le"), Some(4));
        assert_eq!(pcm_bytes("pcm_s24le"), Some(3));
        assert_eq!(layout_channels("mono"), Some(1));
        assert_eq!(layout_channels("3 channels"), Some(3));
    }

    #[test]
    fn a_drifting_microphone_clock_is_played_back_over_the_real_time() {
        assert_eq!(clock_rate_filter(1.0002), None);
        assert_eq!(clock_rate_filter(0.998).as_deref(), Some("asetrate=47904.000,aresample=48000"));
        assert_eq!(clock_rate_filter(0.95).as_deref(), Some("atempo=0.950000"));
        assert_eq!(clock_rate_filter(0.5), None);
    }

    /// The whole join with a packet log: the file says the voice starts at
    /// 0.8 s, the log says its first sample was at 0.5 s, so a beep at 1.0 s
    /// into the voice must sit at 1.5 s of the video.
    #[test]
    fn the_remux_places_the_voice_where_the_packet_log_says() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("miclog");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(run(&["-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=4", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"], &mkv));
        assert!(run(&["-f", "lavfi", "-i", "aevalsrc='0.6*sin(2*PI*1000*t)*between(t,1,1.1)':s=48000:d=3", "-output_ts_offset", "0.8", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        let mut log = String::from("#tb 0: 1/1000000\n#codec_id 0: pcm_f32le\n#sample_rate 0: 48000\n#channel_layout_name 0: mono\n");
        for k in 1..=140u32 {
            let read = 0.5 + k as f64 * 1024.0 / 48_000.0 + if k < 15 { 0.3 } else { 0.004 };
            log.push_str(&format!("0, {0}, {0}, 21333, 4096, 0x0\n", (read * 1e6) as i64));
        }
        std::fs::write(mic_timing(&mic_sidecar(&mkv)), log).unwrap();
        let r = remux_to_mp4_info(&mkv).unwrap();
        assert!(!mic_timing(&mic_sidecar(&mkv)).exists(), "the log goes with the microphone file");
        let beeps = crate::syncprobe::beep_times(&r.path).unwrap();
        assert_eq!(beeps.len(), 1, "{beeps:?}");
        assert!((beeps[0] - 1.5).abs() < 0.03, "the beep belongs at 1.5 s, found {beeps:?}");
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
        assert!((dur - 3.5).abs() < 0.2, "ends with the system sound: {dur}");
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

    #[test]
    fn an_unaligned_mix_is_stamped_where_the_filter_put_it() {
        assert_eq!(system_mix_start(0.5, 1.0), 0.5);
        assert_eq!(system_mix_start(2.0, 1.0), 1.0);
        // More than 30 s apart: the filter overlays both from their file
        // starts, so the mix starts at the earlier start, whichever track
        // has the far one. It used to take the microphone's start here.
        assert_eq!(system_mix_start(40.0, 0.0), 0.0);
        assert_eq!(system_mix_start(0.0, 40.0), 0.0);
    }

    #[test]
    fn an_unaligned_mix_is_not_pushed_to_the_late_microphone_start() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("sysmix-far");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(run(&["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:d=2", "-output_ts_offset", "40", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        assert!(run(&["-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000:d=2", "-c:a", "pcm_f32le"], &sys_sidecar(&mkv)));
        assert!(merge_system_audio(&mkv).unwrap());
        let (start, _) = stream_times(&mic_sidecar(&mkv), "a:0").unwrap();
        assert!(start.abs() < 0.06, "the collapsed mix starts at 0 s, not 40 s: {start}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_rejected_microphone_log_survives_the_mix() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("sysmix-keeplog");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(run(&["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:d=2", "-output_ts_offset", "1.0", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        assert!(run(&["-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000:d=2", "-output_ts_offset", "1.0", "-c:a", "pcm_f32le"], &sys_sidecar(&mkv)));
        // The log says the first sample was at 9 s, 8 s from the file's own
        // start, so the mix does not use it and must not delete it.
        std::fs::write(mic_timing(&mic_sidecar(&mkv)), stall_log(9.0, 18.0, &[])).unwrap();
        assert!(merge_system_audio(&mkv).unwrap());
        assert!(mic_timing(&mic_sidecar(&mkv)).exists(), "an unused log is kept");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_tiny_microphone_keeps_its_log_when_the_system_audio_cannot_move_in() {
        let d = tmpdir("sysmix-tiny");
        let mkv = d.join("recording.mkv");
        let mic = mic_sidecar(&mkv);
        std::fs::write(mic_timing(&mic), "log").unwrap();
        std::fs::write(sys_sidecar(&mkv), vec![9u8; 20_000]).unwrap();
        // A non-empty folder where the microphone file goes: it is not a big
        // audio file, and a file cannot be renamed over it on any platform.
        std::fs::create_dir_all(mic.join("busy")).unwrap();
        assert!(merge_system_audio(&mkv).is_err());
        assert!(mic_timing(&mic).exists(), "the log stays when the move failed");
        assert!(sys_sidecar(&mkv).exists(), "the system audio is left where it was");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_mix_stretches_a_drifting_microphone_clock() {
        let rate = clock_rate_filter(0.98).unwrap();
        let f = system_mix_filter_rate(0.5, 1.0, Some(&rate));
        assert!(f.contains(&rate) && f.contains("adelay=0:all=1[m]") && f.contains("adelay=500:all=1[s]"), "{f}");
        // The rate sits on the microphone branch only, before the delay.
        let mic_branch = f.split("[m];").next().unwrap();
        assert!(mic_branch.contains(&rate), "{f}");
        assert!(!f.split("[m];").nth(1).unwrap().contains(&rate), "the system sound is not stretched: {f}");
        let exact = system_mix_filter_rate(0.5, 1.0, clock_rate_filter(1.0).as_deref());
        assert!(!exact.contains("atempo") && !exact.contains("asetrate"), "{exact}");
    }

    #[test]
    fn a_slow_microphone_is_stretched_when_mixed_with_system_audio() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("sysmix-rate");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| {
            crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success()
        };
        // Voice and system sound both stamped from 1.0 s, matching the
        // packet log's start. 2 s of voice at a 10% slow clock really took
        // 2 / 0.9 s of wall time; the 1 s of system sound is shorter, so
        // the mix should last that stretched voice.
        assert!(run(&["-itsoffset", "1.0", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:d=2", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        assert!(run(&["-itsoffset", "1.0", "-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000:d=1", "-c:a", "pcm_f32le"], &sys_sidecar(&mkv)));
        // A 40 s log so the rate fit is used (takes under 30 s are treated as exact).
        let c = parse_mic_timing(&timing_text(1.0 / 0.9, 40.0)).unwrap();
        assert!((c.ratio - 0.9).abs() < 0.005, "{c:?}");
        assert!((c.start_s - 1.0).abs() < 0.05, "{c:?}");
        std::fs::write(mic_timing(&mic_sidecar(&mkv)), timing_text(1.0 / 0.9, 40.0)).unwrap();
        assert!(merge_system_audio(&mkv).unwrap());
        assert!(!mic_timing(&mic_sidecar(&mkv)).exists(), "the log goes with the raw microphone file");
        let (start, dur) = stream_times(&mic_sidecar(&mkv), "a:0").unwrap();
        assert!((start - 1.0).abs() < 0.06, "keeps the shared start: {start}");
        assert!((dur - (2.0 / 0.9)).abs() < 0.12, "2 s of voice at 0.9 samples/s becomes {:.3}s, got {dur}", 2.0 / 0.9);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A mono float microphone packet log (1024 samples a packet at 48 kHz)
    /// whose first sample is at `start` and that lost `stalls` (where in the
    /// samples, wall seconds lost), with read jitter of up to 40 ms.
    fn stall_log(start: f64, secs: f64, stalls: &[(f64, f64)]) -> String {
        let mut t = String::from("#tb 0: 1/1000000\n#codec_id 0: pcm_f32le\n#sample_rate 0: 48000\n#channel_layout_name 0: mono\n");
        let (mut cum, mut k) = (0.0f64, 0u64);
        while cum < secs {
            let begin = cum;
            cum += 1024.0 / 48_000.0;
            k += 1;
            let lost: f64 = stalls.iter().filter(|s| s.0 <= begin + 1e-9).map(|s| s.1).sum();
            let jitter = 0.002 + 0.038 * (((k * 7919) % 101) as f64 / 100.0);
            let read = start + cum + lost + jitter;
            t.push_str(&format!("0, {0}, {0}, 21333, 4096, 0x0\n", (read * 1e6).round() as i64));
        }
        t
    }

    /// Issue #7 on a Mac: the avfoundation microphone stopped delivering for
    /// 9 s a few seconds in. The file is laid out by sample count, so every
    /// word after that played 9 s early. A straight line through the packet
    /// log cannot describe a step: it gave a wrong rate and a wrong start,
    /// and a seconds-long offset was left on both sides of the stall.
    #[test]
    fn a_stalled_microphone_is_found_in_the_packet_log() {
        let c = parse_mic_timing(&stall_log(1.5, 62.0, &[(12.0, 9.0)])).unwrap();
        assert_eq!(c.gaps.len(), 1, "{:?}", c.gaps);
        let (at, len) = c.gaps[0];
        assert!((at - 12.0).abs() < 0.03 && (len - 9.0).abs() < 0.05, "{:?}", c.gaps);
        assert!((c.ratio - 1.0).abs() < 0.001, "the stall is not mistaken for a slow clock: {c:?}");
        assert!((c.start_s - 1.502).abs() < 0.01, "{c:?}");
        // Many short drops (a busy reader) add up the same way.
        let drops: Vec<(f64, f64)> = (1..30).map(|i| (i as f64 * 2.0, 0.25)).collect();
        let c = parse_mic_timing(&stall_log(0.8, 62.0, &drops)).unwrap();
        assert_eq!(c.gaps.len(), drops.len(), "{:?}", c.gaps);
        assert!((c.lost_s() - 7.25).abs() < 0.05, "{}", c.lost_s());
        assert!((c.ratio - 1.0).abs() < 0.001, "{c:?}");
        // Read jitter alone, a slow clock, and a slow first read are not stalls.
        assert!(parse_mic_timing(&stall_log(1.0, 62.0, &[])).unwrap().gaps.is_empty());
        assert!(parse_mic_timing(&timing_text(1.002, 120.0)).unwrap().gaps.is_empty());
        assert!(parse_mic_timing(&timing_text(1.0, 8.0)).unwrap().gaps.is_empty());
    }

    #[test]
    fn the_fix_filter_puts_stalls_back_before_the_rate() {
        let c = MicClock { start_s: 1.0, ratio: 0.98, span_s: 60.0, gaps: vec![(12.0, 9.0), (30.5, 0.25)] };
        let f = mic_fix_filter(&c).unwrap();
        assert!(f.starts_with("asetpts='(N/SR+9.0000*gte(N/SR,12.0000)+0.2500*gte(N/SR,30.5000))/TB',aresample=48000:async=1"), "{f}");
        assert!(f.ends_with(&clock_rate_filter(0.98).unwrap()), "{f}");
        assert_eq!(mic_fix_filter(&MicClock { start_s: 1.0, ratio: 1.0, span_s: 60.0, gaps: vec![] }), None);
    }

    /// The whole path, with the system audio mixed in (as on Ashish's Mac):
    /// a beep before the stall and one after it both land where they were
    /// said, not 6 s early.
    #[test]
    fn a_stalled_microphone_is_lined_up_after_the_system_audio_mix() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("micstall");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(run(&["-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=25", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"], &mkv));
        // 18 s of samples, beeps at 2 s and 14 s of them; 6 s were lost at 8 s.
        assert!(run(&["-f", "lavfi", "-i", "aevalsrc='0.6*sin(2*PI*1000*t)*(between(t,2,2.1)+between(t,14,14.1))':s=48000:d=18", "-output_ts_offset", "1.03", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        assert!(run(&["-f", "lavfi", "-i", "anoisesrc=a=0.001:r=48000:d=24", "-output_ts_offset", "1.0", "-c:a", "pcm_f32le"], &sys_sidecar(&mkv)));
        std::fs::write(mic_timing(&mic_sidecar(&mkv)), stall_log(1.0, 18.0, &[(8.0, 6.0)])).unwrap();
        assert!(merge_system_audio(&mkv).unwrap());
        let r = remux_to_mp4_info(&mkv).unwrap();
        let beeps = crate::syncprobe::beep_times(&r.path).unwrap();
        assert_eq!(beeps.len(), 2, "{beeps:?}");
        assert!((beeps[0] - 3.0).abs() < 0.06, "the beep before the stall belongs at 3 s: {beeps:?}");
        assert!((beeps[1] - 21.0).abs() < 0.06, "the beep after the stall belongs at 21 s, not 15 s: {beeps:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The same stall with only the microphone.
    #[test]
    fn a_stalled_microphone_is_lined_up_in_the_remux() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = tmpdir("micstall-only");
        let mkv = d.join("recording.mkv");
        let run = |args: &[&str], out: &Path| crate::proc::command(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
        assert!(run(&["-f", "lavfi", "-i", "testsrc=s=160x90:r=30:d=25", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"], &mkv));
        assert!(run(&["-f", "lavfi", "-i", "aevalsrc='0.6*sin(2*PI*1000*t)*(between(t,2,2.1)+between(t,14,14.1))':s=48000:d=18", "-output_ts_offset", "1.03", "-c:a", "pcm_f32le"], &mic_sidecar(&mkv)));
        std::fs::write(mic_timing(&mic_sidecar(&mkv)), stall_log(1.0, 18.0, &[(8.0, 6.0)])).unwrap();
        let r = remux_to_mp4_info(&mkv).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let beeps = crate::syncprobe::beep_times(&r.path).unwrap();
        assert_eq!(beeps.len(), 2, "{beeps:?}");
        assert!((beeps[0] - 3.0).abs() < 0.06 && (beeps[1] - 21.0).abs() < 0.06, "{beeps:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
