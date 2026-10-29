//! Audio/video sync measurement.
//!
//! A synthetic recording flashes the whole picture white for 100 ms and plays
//! a 1 kHz beep for the same 100 ms, once every second, starting together.
//! [`measure`] finds every flash and every beep in any file and reports how
//! far the beeps are from the flashes and whether that gap grows over time.
//! Running it on the output of each stage (recording, proxy, editor sound,
//! export) shows exactly which stage moves the sound against the picture.

use crate::ffmpeg::{self, ffmpeg_bin};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;

/// Rate the beep detector decodes at.
const RATE: u32 = 16_000;

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    /// Flash/beep pairs that were matched.
    pub pairs: usize,
    /// Mean of (beep time - flash time), ms. Positive: sound is late.
    pub offset_ms: f64,
    /// Standard deviation of the per-pair offsets, ms.
    pub jitter_ms: f64,
    /// Growth of the offset, ms per minute (clock drift).
    pub drift_ms_per_min: f64,
    /// Total growth of the offset over the measured stretch, ms.
    pub drift_total_ms: f64,
    /// Largest |offset| of any pair, ms.
    pub worst_ms: f64,
    /// Average time between flashes, s (1.0 for the reference signal).
    pub flash_period_s: f64,
}

/// Make a reference recording: `secs` seconds, a flash and a beep at every
/// whole second from the first (the very start is left quiet so a fade-in
/// cannot hide the first beep). `audio_start_ms` makes the audio file start that much later
/// (its content is shifted so each beep still belongs at its whole second),
/// which is what a microphone that opened late looks like.
pub fn make_reference(path: &Path, secs: u32, with_video: bool, with_audio: bool, audio_start_ms: f64) -> Result<()> {
    if !with_video && !with_audio {
        bail!("nothing to make");
    }
    let mut c = crate::proc::command(ffmpeg_bin());
    c.args(["-v", "error", "-y"]);
    if with_video {
        c.args(["-f", "lavfi", "-i"]).arg(format!(
            "color=c=black:s=320x180:r=30:d={secs},drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='gte(t,1)*lt(mod(t,1),0.1)'"
        ));
    }
    if with_audio {
        if audio_start_ms > 0.0 {
            c.args(["-itsoffset", &format!("{:.3}", audio_start_ms / 1000.0)]);
        }
        let skip = audio_start_ms / 1000.0;
        c.args(["-f", "lavfi", "-i"]).arg(format!("aevalsrc='0.8*sin(2*PI*1000*(t+{skip}))*gte(t+{skip},1)*lt(mod(t+{skip},1),0.1)':s=48000:d={}", secs as f64 - skip));
    }
    if with_video {
        c.args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-g", "15"]);
    } else {
        c.arg("-vn");
    }
    if with_audio {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        c.args(["-c:a", if ext == "mkv" && !with_video { "pcm_f32le" } else { "aac" }]);
        if with_video && audio_start_ms == 0.0 {
            c.arg("-shortest");
        }
    } else {
        c.arg("-an");
    }
    c.arg(path);
    let o = c.output().context("run ffmpeg")?;
    if !o.status.success() {
        bail!("could not make the reference: {}", String::from_utf8_lossy(&o.stderr));
    }
    Ok(())
}

/// Times (s) at which the picture turns from dark to bright.
pub fn flash_times(path: &Path) -> Result<Vec<f64>> {
    let o = crate::proc::command(ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(ffmpeg::file_url(path))
        .args(["-an", "-vf", "scale=8:8:flags=area,format=gray,signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-", "-f", "null", "-"])
        .output()
        .context("run ffmpeg")?;
    if !o.status.success() {
        bail!("flash scan failed: {}", String::from_utf8_lossy(&o.stderr));
    }
    Ok(parse_flashes(&String::from_utf8_lossy(&o.stdout)))
}

/// Parses the `metadata=print` output: `frame:N pts:P pts_time:T` lines, each
/// followed by `lavfi.signalstats.YAVG=V`.
pub fn parse_flashes(text: &str) -> Vec<f64> {
    let mut t = 0.0;
    let mut prev_bright = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(i) = line.find("pts_time:") {
            t = line[i + 9..].split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(t);
        } else if let Some(v) = line.strip_prefix("lavfi.signalstats.YAVG=").and_then(|v| v.trim().parse::<f64>().ok()) {
            let bright = v > 128.0;
            if bright && !prev_bright {
                out.push(t);
            }
            prev_bright = bright;
        }
    }
    out
}

/// Times (s) at which a beep starts, by the audio's own timeline (the file's
/// start offset counts: audio that begins late is padded with silence).
pub fn beep_times(path: &Path) -> Result<Vec<f64>> {
    let o = crate::proc::command(ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(ffmpeg::file_url(path))
        .args(["-vn", "-ac", "1", "-af", &format!("aresample={RATE}:async=1:first_pts=0"), "-f", "f32le", "-"])
        .output()
        .context("run ffmpeg")?;
    if !o.status.success() {
        bail!("beep scan failed: {}", String::from_utf8_lossy(&o.stderr));
    }
    let x: Vec<f32> = o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    Ok(find_beeps(&x, RATE))
}

/// Onsets of tones in `x`: the first 2 ms window above 25% of the loudest
/// window that follows at least 400 ms of quiet.
pub fn find_beeps(x: &[f32], rate: u32) -> Vec<f64> {
    let win = (rate as usize / 500).max(1);
    let peaks: Vec<f32> = x.chunks(win).map(|w| w.iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect();
    let max = peaks.iter().cloned().fold(0.0f32, f32::max);
    if max < 0.01 {
        return Vec::new();
    }
    let thr = max * 0.25;
    let quiet_windows = (0.4 * rate as f64 / win as f64) as usize;
    let mut out = Vec::new();
    let mut quiet = quiet_windows; // the start of the file counts as quiet
    for (i, p) in peaks.iter().enumerate() {
        if *p > thr {
            if quiet >= quiet_windows {
                out.push(i as f64 * win as f64 / rate as f64);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

/// Pair flashes and beeps (each beep with the nearest flash within 500 ms)
/// and summarise.
pub fn summarize(flashes: &[f64], beeps: &[f64]) -> Result<SyncResult> {
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for &b in beeps {
        if let Some(&f) = flashes.iter().min_by(|a, c| (b - **a).abs().total_cmp(&(b - **c).abs())) {
            if (b - f).abs() < 0.5 {
                pairs.push((f, (b - f) * 1000.0));
            }
        }
    }
    if pairs.len() < 3 {
        bail!("found {} flashes and {} beeps, {} pairs (need 3)", flashes.len(), beeps.len(), pairs.len());
    }
    let n = pairs.len() as f64;
    let mean = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let jitter = (pairs.iter().map(|p| (p.1 - mean).powi(2)).sum::<f64>() / n).sqrt();
    let mx = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let den: f64 = pairs.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let slope = if den > 0.0 { pairs.iter().map(|p| (p.0 - mx) * (p.1 - mean)).sum::<f64>() / den } else { 0.0 };
    let worst = pairs.iter().map(|p| p.1.abs()).fold(0.0, f64::max);
    let period = if flashes.len() > 1 { (flashes[flashes.len() - 1] - flashes[0]) / (flashes.len() - 1) as f64 } else { 0.0 };
    let span = pairs.iter().map(|p| p.0).fold(f64::MIN, f64::max) - pairs.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    Ok(SyncResult { pairs: pairs.len(), offset_ms: mean, jitter_ms: jitter, drift_ms_per_min: slope * 60.0, drift_total_ms: slope * span, worst_ms: worst, flash_period_s: period })
}

/// Measure a file.
pub fn measure(path: &Path) -> Result<SyncResult> {
    summarize(&flash_times(path)?, &beep_times(path)?)
}

/// What the sound-versus-picture offset of the *editor preview* is: the proxy
/// carries the sound the editor plays, the video frames come from the proxy
/// too, so the proxy file is what the editor shows.
pub fn measure_proxy(src: &Path, cache: &Path) -> Result<SyncResult> {
    let st = crate::proxy::build(cache, src, |_| {})?;
    measure(&st.path)
}

/// One stage of the pipeline, measured.
#[derive(Debug, Clone, Serialize)]
pub struct StageRow {
    pub stage: String,
    /// Offset of the file this stage was given, ms.
    pub input_offset_ms: f64,
    pub result: Option<SyncResult>,
    pub error: Option<String>,
    /// Offset added by this stage (output minus input), ms.
    pub moved_ms: Option<f64>,
    pub ok: bool,
}

/// How far a stage may move the sound against the picture, ms. One video
/// frame at 30 fps is 33 ms, which is also how finely a flash can be placed.
pub const BUDGET_MOVE_MS: f64 = 50.0;
/// Allowed growth of the offset over the whole measured stretch, ms.
pub const BUDGET_DRIFT_TOTAL_MS: f64 = 25.0;
/// Allowed spread of the offset between beeps, ms.
pub const BUDGET_JITTER_MS: f64 = 35.0;

fn row(stage: &str, input: &SyncResult, out: Result<SyncResult>) -> StageRow {
    match out {
        Ok(r) => {
            let moved = r.offset_ms - input.offset_ms;
            let ok = moved.abs() <= BUDGET_MOVE_MS && r.drift_total_ms.abs() <= BUDGET_DRIFT_TOTAL_MS && r.jitter_ms <= BUDGET_JITTER_MS;
            StageRow { stage: stage.into(), input_offset_ms: input.offset_ms, result: Some(r), error: None, moved_ms: Some(moved), ok }
        }
        Err(e) => StageRow { stage: stage.into(), input_offset_ms: input.offset_ms, result: None, error: Some(format!("{e:#}")), moved_ms: None, ok: false },
    }
}

/// Run the synthetic recording through each stage of the pipeline that works
/// on files (everything after the capture itself) and measure the sync after
/// each: finishing a recording with a late microphone, the editing proxy
/// (the picture and sound the editor plays), and exports (styled and copy).
pub fn run_file_stages(dir: &Path) -> Result<Vec<StageRow>> {
    std::fs::create_dir_all(dir)?;
    let secs = 10;
    let mut rows = Vec::new();

    // 1. A recording that is in sync, through the proxy and the exports.
    let reference = dir.join("reference.mp4");
    make_reference(&reference, secs, true, true, 0.0)?;
    let base = measure(&reference).context("measure the reference")?;
    rows.push(row("reference (no processing)", &base, Ok(base.clone())));
    let cache = dir.join("cache");
    rows.push(row("editor preview: proxy video and sound", &base, measure_proxy(&reference, &cache)));
    rows.push(row("export, styled", &base, export_measure(&reference, dir, "styled.mp4", 0.05, None)));
    rows.push(row("export, copy", &base, export_measure(&reference, dir, "copy.mp4", 0.0, None)));
    rows.push(row("export, after cutting the first 3.5 s", &base, export_measure(&reference, dir, "cut.mp4", 0.05, Some((3500.0, secs as f64 * 1000.0)))));

    // 2. A screen file and a microphone file recorded separately; the
    // microphone opened 350 ms after the screen. Joining them must put each
    // beep back with its flash.
    let rec = dir.join("sidecar").join("recording.mkv");
    std::fs::create_dir_all(rec.parent().unwrap())?;
    make_reference(&rec, secs, true, false, 0.0)?;
    make_reference(&crate::finalize::mic_sidecar(&rec), secs, false, true, 350.0)?;
    let ideal = SyncResult { pairs: 0, offset_ms: 0.0, jitter_ms: 0.0, drift_ms_per_min: 0.0, drift_total_ms: 0.0, worst_ms: 0.0, flash_period_s: 1.0 };
    let joined = crate::finalize::remux_to_mp4_info(&rec).map(|r| r.path).and_then(|p| measure(&p));
    rows.push(row("finish recording: screen + microphone opened 350 ms late", &ideal, joined));
    Ok(rows)
}

fn export_measure(src: &Path, dir: &Path, name: &str, padding: f64, trim: Option<(f64, f64)>) -> Result<SyncResult> {
    let mut p = crate::new_project(src, None)?;
    p.style.padding = padding;
    p.style.shadow = 0.0;
    p.style.background = crate::project::Background::Solid { color: [0, 0, 0] };
    p.export.quality = 360;
    if let Some((a, b)) = trim {
        p.trim(a, b);
    }
    let out = dir.join(name);
    crate::export::export(&p, &out, |_| {})?;
    measure(&out)
}

/// A table of the rows, for the console and the CI summary.
pub fn format_rows(rows: &[StageRow]) -> String {
    let mut s = String::from("| stage | sound vs picture (ms) | moved by stage (ms) | drift over the clip (ms) | jitter (ms) | result |\n|---|---|---|---|---|---|\n");
    for r in rows {
        match &r.result {
            Some(x) => s.push_str(&format!(
                "| {} | {:+.0} | {:+.0} | {:+.0} | {:.0} | {} |\n",
                r.stage, x.offset_ms, r.moved_ms.unwrap_or(0.0), x.drift_total_ms, x.jitter_ms, if r.ok { "ok" } else { "OUT OF BUDGET" }
            )),
            None => s.push_str(&format!("| {} | | | | | FAILED: {} |\n", r.stage, r.error.clone().unwrap_or_default())),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beeps_and_flashes_are_found_in_synthetic_data() {
        let mut x = vec![0.0f32; 16_000 * 4];
        for k in 1..4 {
            for i in 0..1600 {
                x[k * 16_000 + 80 + i] = 0.5 * (i as f32 * 0.4).sin();
            }
        }
        let b = find_beeps(&x, 16_000);
        assert_eq!(b.len(), 3);
        assert!((b[0] - (1.0 + 0.005)).abs() < 0.005, "{b:?}");
        let text = "frame:0 pts:0 pts_time:0\nlavfi.signalstats.YAVG=10\nframe:1 pts:1 pts_time:0.5\nlavfi.signalstats.YAVG=240\nframe:2 pts:2 pts_time:0.6\nlavfi.signalstats.YAVG=240\nframe:3 pts:3 pts_time:0.7\nlavfi.signalstats.YAVG=10\n";
        assert_eq!(parse_flashes(text), vec![0.5]);
        let r = summarize(&[1.0, 2.0, 3.0, 4.0], &[1.05, 2.05, 3.05, 4.05]).unwrap();
        assert!((r.offset_ms - 50.0).abs() < 1e-6 && r.drift_ms_per_min.abs() < 1e-6 && r.jitter_ms < 1e-6);
        let d = summarize(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.001, 3.002, 4.003]).unwrap();
        assert!((d.drift_ms_per_min - 60.0).abs() < 1e-6, "{d:?}");
    }

    #[test]
    fn every_file_stage_keeps_the_sound_with_the_picture() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = std::env::temp_dir().join(format!("dl-sync-stages-{}", std::process::id()));
        let rows = run_file_stages(&d).unwrap();
        let table = format_rows(&rows);
        eprintln!("{table}");
        assert!(rows.iter().all(|r| r.ok), "{table}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_reference_signal_measures_as_in_sync() {
        if !ffmpeg::ffmpeg_found() {
            return;
        }
        let d = std::env::temp_dir().join(format!("dl-sync-ref-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("ref.mp4");
        make_reference(&p, 8, true, true, 0.0).unwrap();
        let r = measure(&p).unwrap();
        assert!(r.pairs >= 6, "{r:?}");
        assert!(r.offset_ms.abs() < 25.0 && r.worst_ms < 40.0, "{r:?}");
        assert!(r.drift_ms_per_min.abs() < 30.0, "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Print the flash and beep times of a file (debugging aid).
pub fn dump(path: &Path) -> Result<String> {
    let f = flash_times(path)?;
    let b = beep_times(path)?;
    Ok(format!("flashes {:?}\nbeeps   {:?}", f.iter().map(|v| (v * 1000.0).round() / 1000.0).collect::<Vec<_>>(), b.iter().map(|v| (v * 1000.0).round() / 1000.0).collect::<Vec<_>>()))
}

/// Result of one real screen + sound capture.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureRow {
    pub method: String,
    pub result: Option<SyncResult>,
    pub error: Option<String>,
}

/// Records a flashing X11 screen with a beeping microphone through the real
/// capture and finish code, and again with the previous method (the
/// microphone mixed into the screen's ffmpeg), and measures both.
///
/// The flash is the X root window turning white (`xsetroot`), the beep plays
/// into a PulseAudio null sink whose monitor is the "microphone". The two
/// start from separate processes, so the absolute offset has a skew of a few
/// tens of ms that is the same for both methods; what matters is the drift
/// and the gap between the methods.
///
/// Needs `DISPLAY` (for example Xvfb), `xsetroot` and a PulseAudio sink named
/// `probe_sink` (`pactl load-module module-null-sink sink_name=probe_sink`).
#[cfg(target_os = "linux")]
pub fn run_capture_linux(dir: &Path, secs: u32) -> Result<Vec<CaptureRow>> {
    use crate::capture::{self, CaptureTarget, RecordOptions};
    use std::process::Stdio;
    use std::time::Duration;
    std::fs::create_dir_all(dir)?;
    let display = std::env::var("DISPLAY").context("DISPLAY is not set (start Xvfb)")?;
    let sink = std::env::var("BOOYA_PROBE_SINK").unwrap_or_else(|_| "probe_sink".into());
    let mic = format!("{sink}.monitor");

    // The generator: flashes (xsetroot) and beeps (ffmpeg into the sink), starting now.
    let generate = |secs: u32| -> Result<std::thread::JoinHandle<()>> {
        let mut beep = crate::proc::command(ffmpeg_bin());
        beep.args(["-v", "error", "-nostdin", "-re", "-f", "lavfi", "-i"])
            .arg(format!("aevalsrc='0.6*sin(2*PI*1000*t)*gte(t,1)*lt(mod(t,1),0.1)':s=48000:d={}", secs + 1))
            .args(["-f", "pulse", "-device", &sink, "probe_beep"])
            .stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = beep.spawn().context("start the beep source")?;
        let disp = display.clone();
        Ok(std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let set = |c: &str| {
                let _ = crate::proc::command("xsetroot").env("DISPLAY", &disp).args(["-solid", c]).status();
            };
            set("black");
            for k in 1..=secs {
                let at = Duration::from_secs(k as u64);
                if let Some(w) = at.checked_sub(t0.elapsed()) {
                    std::thread::sleep(w);
                }
                set("white");
                std::thread::sleep(Duration::from_millis(100));
                set("black");
            }
            let _ = child.wait();
        }))
    };

    let mut rows = Vec::new();
    // 1. The real thing: record() with the microphone on the side, then finish.
    let one = dir.join("sidecar");
    let _ = std::fs::remove_dir_all(&one);
    let r = (|| -> Result<SyncResult> {
        let opts = RecordOptions {
            target: CaptureTarget::Screen { display: 0 },
            fps: 30,
            mic: true,
            system_audio: false,
            out_dir: one.clone(),
            keys: crate::input::KeyLogging::default(),
            webcam: false,
            mic_device: Some(mic.clone()),
            max_height: None,
        };
        let session = capture::start(&opts)?;
        let g = generate(secs)?;
        let _ = g.join();
        std::thread::sleep(Duration::from_millis(300));
        let rec = session.stop()?;
        measure(&rec.video)
    })();
    rows.push(CaptureRow { method: "screen + microphone recorded separately (new)".into(), result: r.as_ref().ok().cloned(), error: r.err().map(|e| format!("{e:#}")) });

    // 2. The previous method: screen and microphone in one ffmpeg.
    let old = dir.join("mixed.mkv");
    let r = (|| -> Result<SyncResult> {
        let (w, h) = {
            let o = crate::proc::command("xdpyinfo").env("DISPLAY", &display).output()?;
            let t = String::from_utf8_lossy(&o.stdout).to_string();
            let d = t.lines().find_map(|l| l.trim().strip_prefix("dimensions:").map(|r| r.split_whitespace().next().unwrap_or("").to_string())).unwrap_or_default();
            let (w, h) = d.split_once('x').context("screen size")?;
            (w.parse::<u32>()? & !1, h.parse::<u32>()? & !1)
        };
        let mut c = crate::proc::command(ffmpeg_bin());
        c.args(["-v", "error", "-y", "-f", "x11grab", "-draw_mouse", "0", "-framerate", "30", "-video_size"])
            .arg(format!("{w}x{h}")).args(["-i", &format!("{display}+0,0"), "-f", "pulse", "-i", &mic, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&old).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = c.spawn()?;
        std::thread::sleep(Duration::from_millis(1500));
        let g = generate(secs)?;
        let _ = g.join();
        std::thread::sleep(Duration::from_millis(300));
        if let Some(mut i) = child.stdin.take() {
            use std::io::Write;
            let _ = i.write_all(b"q");
        }
        let _ = child.wait();
        measure(&old)
    })();
    rows.push(CaptureRow { method: "screen + microphone in one ffmpeg (previous)".into(), result: r.as_ref().ok().cloned(), error: r.err().map(|e| format!("{e:#}")) });
    Ok(rows)
}

/// Table of capture rows.
pub fn format_capture(rows: &[CaptureRow]) -> String {
    let mut s = String::from("| capture method | sound vs picture (ms, includes the test's own start skew) | drift over the clip (ms) | jitter (ms) | pairs |\n|---|---|---|---|---|\n");
    for r in rows {
        match &r.result {
            Some(x) => s.push_str(&format!("| {} | {:+.0} | {:+.0} | {:.0} | {} |\n", r.method, x.offset_ms, x.drift_total_ms, x.jitter_ms, x.pairs)),
            None => s.push_str(&format!("| {} | | | | FAILED: {} |\n", r.method, r.error.clone().unwrap_or_default())),
        }
    }
    s
}

/// How regular the flashes of a video-only capture are: the number found, the
/// mean period in seconds (a clock that runs right gives 1.000 for a source
/// that flashes once a second) and the worst deviation from a perfect grid, ms.
pub fn flash_regularity(path: &Path) -> Result<(usize, f64, f64)> {
    let f = flash_times(path)?;
    if f.len() < 3 {
        bail!("only {} flashes found in {}", f.len(), path.display());
    }
    let n = f.len() as f64;
    let xs: Vec<f64> = (0..f.len()).map(|i| i as f64).collect();
    let (mx, my) = (xs.iter().sum::<f64>() / n, f.iter().sum::<f64>() / n);
    let slope = xs.iter().zip(&f).map(|(x, y)| (x - mx) * (y - my)).sum::<f64>() / xs.iter().map(|x| (x - mx).powi(2)).sum::<f64>();
    let worst = xs.iter().zip(&f).map(|(x, y)| (y - (my + slope * (x - mx))).abs() * 1000.0).fold(0.0, f64::max);
    Ok((f.len(), slope, worst))
}
