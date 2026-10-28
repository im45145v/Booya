//! Project level AI operations shared by the CLI and the desktop app.

use crate::{models, redact, transcribe};
use anyhow::{Context, Result};
use demolens_core::captions::{self, AutoCutOptions};
use demolens_core::project::{BlurBox, Project};
use demolens_core::audio;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TranscribeReport {
    pub words: usize,
    pub fillers: usize,
    pub lines: usize,
    /// The speech model that produced the words ("cloud" for a provider).
    pub model: String,
    /// True when a small model was used and a better one would help.
    pub weak_model: bool,
    /// Why there are no captions, when there are none: "no_audio" (the
    /// recording has no sound track), "too_quiet" (only a noise floor) or
    /// "no_speech" (sound, but no words). Empty otherwise.
    #[serde(default)]
    pub reason: String,
}

pub fn speech_model(id: Option<&str>) -> Result<std::path::PathBuf> {
    match id {
        Some(id) => models::require(id),
        None => transcribe::default_model().context("no speech model installed. Download one in the model manager (whisper-base.en is a good start)"),
    }
}

/// Transcribe the project's audio into captions (word timed, in source time).
pub fn transcribe_project(p: &mut Project, model: Option<&str>, lang: Option<&str>) -> Result<TranscribeReport> {
    let mut samples = Vec::new();
    let has_audio = demolens_core::ffmpeg::probe(&p.source).map(|i| i.has_audio).unwrap_or(true);
    if has_audio {
        samples = audio::decode_mono(&p.source, 16_000)?;
    }
    let reason = if !has_audio || samples.is_empty() {
        Some("no_audio")
    } else if !transcribe::audible(&samples) {
        Some("too_quiet")
    } else {
        None
    };
    if let Some(r) = reason {
        // Nothing to hear: no captions, and no error either.
        return Ok(TranscribeReport { words: 0, fillers: 0, lines: p.captions.len(), model: String::new(), weak_model: false, reason: r.into() });
    }
    let model = speech_model(model)?;
    let words = transcribe::transcribe(&p.source, &model, lang)?;
    let name = model.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let mut r = if words.is_empty() {
        TranscribeReport { words: 0, fillers: 0, lines: p.captions.len(), model: String::new(), weak_model: false, reason: "no_speech".into() }
    } else {
        finish_transcript(p, words)?
    };
    r.weak_model = name.contains("tiny");
    r.model = name;
    Ok(r)
}

/// Transcribe with a cloud provider the user opted into. The audio track
/// (16 kHz mono WAV) is the only thing sent.
pub fn transcribe_project_cloud(p: &mut Project, cfg: &crate::providers::CloudConfig, provider: crate::providers::Provider, key: &str, lang: Option<&str>) -> Result<TranscribeReport> {
    let samples = audio::decode_mono(&p.source, 16_000)?;
    let wav = crate::providers::wav_bytes(&samples, 16_000);
    let words = crate::providers::transcribe(cfg, provider, key, &wav, lang)?;
    finish_transcript(p, words)
}

fn finish_transcript(p: &mut Project, mut words: Vec<demolens_core::project::CaptionWord>) -> Result<TranscribeReport> {
    words.retain(|w| !captions::is_nonspeech(&w.text));
    if words.is_empty() || transcribe::only_hallucination(&words) {
        return Ok(TranscribeReport { words: 0, fillers: 0, lines: p.captions.len(), model: "cloud".into(), weak_model: false, reason: "no_speech".into() });
    }
    let samples = audio::decode_mono(&p.source, 16_000)?;
    captions::snap_to_speech(&mut words, &audio::detect_silences(&samples, 16_000, -38.0, 250.0));
    // Captions never show before anyone speaks, whoever did the transcribing.
    transcribe::anchor_to_voice(&mut words, &transcribe::voice_onsets(&samples, 16_000));
    p.captions = captions::words_to_captions(&words, 42, 900.0);
    Ok(TranscribeReport { words: words.len(), fillers: words.iter().filter(|w| captions::is_filler(&w.text)).count(), lines: p.captions.len(), model: "cloud".into(), weak_model: false, reason: String::new() })
}

#[derive(Debug, Serialize)]
pub struct CutReport {
    /// (start_ms, end_ms, "filler" | "silence")
    pub cuts: Vec<(f64, f64, String)>,
    pub before_ms: f64,
    pub after_ms: f64,
    pub had_transcript: bool,
}

/// Plan (and unless `dry_run`, apply) cuts for filler words and long pauses.
pub fn autocut_project(p: &mut Project, o: &AutoCutOptions, dry_run: bool) -> Result<CutReport> {
    let words: Vec<_> = p.captions.iter().flat_map(|c| c.words.clone()).collect();
    let log = p.events.as_ref().and_then(|e| demolens_core::events::EventLog::load(e).ok());
    // Quiet is only a candidate. A stretch is cut only when the microphone is
    // quiet AND the pointer, keyboard and picture are all still: many demos are
    // given in silence, and the sound alone says nothing about the screen.
    let silences = if o.cut_silences {
        let info = demolens_core::ffmpeg::probe(&p.source)?;
        let quiet = if info.has_audio {
            let samples = audio::decode_mono(&p.source, 16_000)?;
            audio::detect_silences(&samples, 16_000, -38.0, o.min_silence_ms)
        } else {
            vec![(0.0, p.duration_ms)]
        };
        let idle_opts = demolens_core::activity::IdleOptions { min_idle_ms: o.min_silence_ms.max(2500.0), ..Default::default() };
        let idle = demolens_core::activity::idle_stretches(&p.source, log.as_ref(), p.duration_ms, &idle_opts)
            .context("could not check the screen for activity, so no pauses were cut")?;
        demolens_core::activity::intersect(&quiet, &idle)
    } else {
        Vec::new()
    };
    let mut planned = captions::plan_cuts(&words, &silences, o);
    // A quiet stretch is not dead air when the presenter is clicking or
    // typing in it: keep the on-screen action and only cut around it.
    if let Some(log) = log.as_ref() {
        let activity: Vec<f64> = log.clicks().map(|c| c.0).chain(log.keys().map(|k| k.0)).collect();
        planned = protect_activity(&planned, &activity, 400.0, 900.0, 300.0);
        // Also keep the cursor travelling to each click, so a cut never
        // makes the pointer jump straight onto the button.
        planned = protect_ranges(&planned, &click_travel(log), 300.0);
    }
    let cuts = planned
        .iter()
        .map(|&(s, e)| {
            let filler = words.iter().any(|w| captions::is_filler(&w.text) && w.start_ms >= s - 50.0 && w.end_ms <= e + 50.0);
            (s, e, if filler { "filler" } else { "silence" }.to_string())
        })
        .collect();
    let before = p.output_duration_ms();
    if !dry_run {
        p.cut_ranges(&planned);
        p.validate()?;
    }
    Ok(CutReport { cuts, before_ms: before, after_ms: p.output_duration_ms(), had_transcript: !words.is_empty() })
}

/// Remove `[t - before, t + after]` around every activity time from the cut
/// ranges, dropping leftover pieces shorter than `min_len` ms.
pub fn protect_activity(cuts: &[(f64, f64)], activity: &[f64], before: f64, after: f64, min_len: f64) -> Vec<(f64, f64)> {
    let keep: Vec<(f64, f64)> = activity.iter().map(|&t| (t - before, t + after)).collect();
    protect_ranges(cuts, &keep, min_len)
}

/// For every click, the stretch where the pointer was moving towards it:
/// back from the click while move events keep coming (gaps under 250 ms),
/// at least 1 s and at most 2.5 s.
pub fn click_travel(log: &demolens_core::events::EventLog) -> Vec<(f64, f64)> {
    use demolens_core::events::EventKind;
    let moves: Vec<f64> = log.events.iter().filter(|e| matches!(e.kind, EventKind::Move { .. })).map(|e| e.t).collect();
    log.clicks()
        .map(|c| {
            let t = c.0;
            let mut start = t;
            for &m in moves.iter().rev().filter(|&&m| m <= t) {
                if start - m > 250.0 || t - m > 2_500.0 {
                    break;
                }
                start = m;
            }
            (start.min(t - 1_000.0) - 150.0, t)
        })
        .collect()
}

/// Remove the `keep` ranges from the cut ranges, dropping leftover pieces
/// shorter than `min_len` ms.
pub fn protect_ranges(cuts: &[(f64, f64)], keep: &[(f64, f64)], min_len: f64) -> Vec<(f64, f64)> {
    let mut keep = keep.to_vec();
    keep.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    for &(s, e) in cuts {
        let mut start = s;
        for &(ks, ke) in &keep {
            if ke <= start || ks >= e {
                continue;
            }
            if ks - start >= min_len {
                out.push((start, ks));
            }
            start = start.max(ke);
        }
        if e - start >= min_len {
            out.push((start, e));
        }
    }
    out
}

#[cfg(test)]
mod activity_tests {
    use super::protect_activity;
    #[test]
    fn cuts_skip_clicks_and_typing() {
        // A 10 s silence with typing at 4 s keeps 3.6..4.9 s.
        let r = protect_activity(&[(0.0, 10_000.0)], &[4_000.0], 400.0, 900.0, 300.0);
        assert_eq!(r, vec![(0.0, 3_600.0), (4_900.0, 10_000.0)]);
        // Leftovers shorter than the minimum are not cut.
        let r = protect_activity(&[(1_000.0, 1_500.0)], &[1_300.0], 400.0, 900.0, 300.0);
        assert!(r.is_empty());
        // No activity, no change.
        assert_eq!(protect_activity(&[(1.0, 2.0)], &[], 400.0, 900.0, 0.5), vec![(1.0, 2.0)]);
    }

    #[test]
    fn cursor_travel_before_a_click_is_kept() {
        use demolens_core::events::{Event, EventKind, EventLog, MouseButton};
        let mut log = EventLog::new(1920, 1080);
        // Pointer moves from 5.0 s to 6.8 s, clicks at 7.0 s.
        let mut t = 5_000.0;
        while t <= 6_800.0 {
            log.events.push(Event { t, kind: EventKind::Move { x: t / 10.0, y: 100.0 } });
            t += 50.0;
        }
        log.events.push(Event { t: 7_000.0, kind: EventKind::Click { x: 680.0, y: 100.0, button: MouseButton::Left } });
        let r = super::click_travel(&log);
        assert_eq!(r, vec![(4_850.0, 7_000.0)]);
        // A click with no movement still keeps 1 s before it.
        let mut still = EventLog::new(100, 100);
        still.events.push(Event { t: 3_000.0, kind: EventKind::Click { x: 1.0, y: 1.0, button: MouseButton::Left } });
        assert_eq!(super::click_travel(&still), vec![(1_850.0, 3_000.0)]);
    }
}

#[derive(Debug, Serialize)]
pub struct RedactReport {
    /// Distinct (kind, text) values found.
    pub found: Vec<(redact::Sensitive, String)>,
    pub detections: usize,
    pub added: usize,
}

/// Scan frames for sensitive text and add blur boxes for it.
pub fn redact_project(p: &mut Project, every_ms: f64, dry_run: bool, progress: impl FnMut(f64)) -> Result<RedactReport> {
    let hits = redact::scan(p, every_ms, progress)?;
    let mut boxes = redact::hits_to_blur(&hits, every_ms, 6.0);
    for b in &mut boxes {
        // The last sampled frame is up to two samples before the end.
        if b.end_ms + 2.0 * every_ms >= p.duration_ms {
            b.end_ms = p.duration_ms;
        }
    }
    let mut found: Vec<(redact::Sensitive, String)> = Vec::new();
    for h in &hits {
        if !found.iter().any(|(k, t)| *k == h.kind && *t == h.text) {
            found.push((h.kind, h.text.clone()));
        }
    }
    // Running it twice should not stack duplicate boxes.
    let dup = |a: &BlurBox, b: &BlurBox| (a.x - b.x).abs() < 8.0 && (a.y - b.y).abs() < 8.0 && (a.w - b.w).abs() < 16.0 && (a.start_ms - b.start_ms).abs() < every_ms;
    let mut added = 0;
    if !dry_run {
        for b in boxes {
            if !p.blur.iter().any(|o| dup(o, &b)) {
                p.blur.push(b);
                added += 1;
            }
        }
    } else {
        added = boxes.len();
    }
    Ok(RedactReport { found, detections: hits.len(), added })
}

/// One step of [`demo_ready`].
#[derive(Debug, Clone, Serialize)]
pub struct ReadyStep {
    pub name: &'static str,
    /// "done", "skipped" or "failed".
    pub state: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadyReport {
    pub steps: Vec<ReadyStep>,
    pub before_ms: f64,
    pub after_ms: f64,
}

#[derive(Debug, Clone)]
pub struct ReadyOptions {
    pub zoom: bool,
    pub captions: bool,
    /// Cut filler words (needs a transcript).
    pub cut: bool,
    /// Also cut pauses where the screen is still too. Off unless asked for.
    pub cut_silences: bool,
    pub blur: bool,
    pub denoise: bool,
    pub min_silence_ms: f64,
}

impl Default for ReadyOptions {
    fn default() -> Self {
        Self { zoom: true, captions: true, cut: true, cut_silences: false, blur: true, denoise: true, min_silence_ms: 700.0 }
    }
}

/// Auto edit in one call: auto zoom, transcribe and caption, cut
/// filler words and long pauses, blur secrets and clean up the audio. Steps
/// whose model is missing are skipped with a note instead of failing.
pub fn demo_ready(p: &mut Project, o: &ReadyOptions, mut progress: impl FnMut(f64, &str)) -> ReadyReport {
    let before_ms = p.output_duration_ms();
    let mut steps = Vec::new();
    let mut step = |name, state, detail: String| steps.push(ReadyStep { name, state, detail });
    let has_audio = demolens_core::ffmpeg::probe(&p.source).map(|i| i.has_audio).unwrap_or(false);

    if o.zoom {
        progress(0.05, "Adding auto zoom...");
        match p.events.as_ref().map(|e| demolens_core::events::EventLog::load(e)) {
            Some(Ok(log)) => {
                p.zoom = demolens_core::zoom::generate_zoom_segments(&log, p.duration_ms, &demolens_core::zoom::ZoomConfig::default());
                step("Auto zoom", "done", format!("{} zoom segments from clicks and typing", p.zoom.len()));
            }
            Some(Err(err)) => step("Auto zoom", "failed", format!("could not read the input log: {err}")),
            None => step("Auto zoom", "skipped", "no input log next to this video".into()),
        }
    }

    let mut have_words = !p.captions.is_empty();
    if o.captions {
        progress(0.15, "Transcribing on this computer...");
        if !has_audio {
            step("Captions", "skipped", "the video has no audio".into());
        } else if models::first_installed(models::Kind::Speech).is_none() {
            step("Captions", "skipped", "download a speech model (Whisper) in Models to get captions".into());
        } else {
            match transcribe_project(p, None, None) {
                Ok(r) => {
                    p.caption_style.burn_in = true;
                    have_words = true;
                    step("Captions", "done", format!("{} words, {} caption lines, burned in", r.words, r.lines));
                }
                Err(err) => step("Captions", "failed", err.to_string()),
            }
        }
    }

    if o.cut {
        progress(0.45, "Cutting filler words...");
        if !has_audio && !o.cut_silences {
            step("Auto cut", "skipped", "the video has no audio, so there are no filler words to cut".into());
        } else if !have_words && !o.cut_silences {
            step("Auto cut", "skipped", "no transcript, so no filler words to cut. Pauses are left alone unless you ask for them".into());
        } else {
            let co = AutoCutOptions { cut_fillers: have_words, cut_silences: o.cut_silences, min_silence_ms: o.min_silence_ms, ..Default::default() };
            match autocut_project(p, &co, false) {
                Ok(r) => {
                    let f = r.cuts.iter().filter(|c| c.2 == "filler").count();
                    step("Auto cut", "done", format!("{} filler words and {} still pauses removed. Ctrl+Z undoes it", f, r.cuts.len() - f));
                }
                Err(err) => step("Auto cut", "failed", err.to_string()),
            }
        }
    }

    if o.blur {
        progress(0.55, "Scanning frames for secrets...");
        if models::first_installed(models::Kind::Ocr).is_none() || models::require("ocr-detection").is_err() || models::require("ocr-recognition").is_err() {
            step("Blur secrets", "skipped", "download the two text recognition models (13 MB) in Models".into());
        } else {
            match redact_project(p, 500.0, false, |f| progress(0.55 + f * 0.4, "Scanning frames for secrets...")) {
                Ok(r) if r.found.is_empty() => step("Blur secrets", "done", "nothing sensitive found".into()),
                Ok(r) => {
                    let mut kinds: Vec<&str> = Vec::new();
                    for (k, _) in &r.found {
                        let n = match k {
                            redact::Sensitive::Email => "email",
                            redact::Sensitive::ApiKey => "API key",
                            redact::Sensitive::Card => "card number",
                            redact::Sensitive::Phone => "phone number",
                            redact::Sensitive::Aadhaar => "Aadhaar number",
                            redact::Sensitive::Pan => "PAN",
                            redact::Sensitive::Upi => "UPI ID",
                            redact::Sensitive::Iban => "IBAN",
                            redact::Sensitive::Ip => "IP address",
                            redact::Sensitive::Token => "token or password",
                            redact::Sensitive::Password => "password field",
                        };
                        if !kinds.contains(&n) {
                            kinds.push(n);
                        }
                    }
                    step("Blur secrets", "done", format!("blurred {} ({} boxes). Auto blur may miss text. Watch the export before sharing.", kinds.join(", "), r.added))
                }
                Err(err) => step("Blur secrets", "failed", err.to_string()),
            }
        }
    }

    if o.denoise && has_audio {
        p.audio.denoise = true;
        step("Clean audio", "done", "background noise removal on".into());
    }
    progress(1.0, "Done");
    ReadyReport { steps, before_ms, after_ms: p.output_duration_ms() }
}

/// The regression behind "my video was trimmed for no reason": a demo given
/// in silence must keep every second where something happens.
#[cfg(test)]
mod silent_demo_tests {
    use super::*;
    use demolens_core::events::{Event, EventKind, EventLog, MouseButton};

    /// A 14 s 640x360 video with a silent audio track. A box changes at the
    /// given times (so the picture is "active" there); the pointer moves in
    /// the given ranges and clicks at the given times.
    fn demo(name: &str, flashes: &[f64], moving: &[(f64, f64)], clicks: &[f64]) -> Option<Project> {
        let dir = std::env::temp_dir().join(format!("dl-silent-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let video = dir.join("demo.mp4");
        let enable = flashes.iter().map(|t| format!("between(t,{t},{})", t + 0.4)).collect::<Vec<_>>().join("+");
        let vf = if flashes.is_empty() { "null".to_string() } else { format!("drawbox=x=100:y=100:w=200:h=100:color=white:t=fill:enable='{enable}'") };
        let ok = std::process::Command::new(demolens_core::ffmpeg::ffmpeg_bin()) // proc:allow
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=black:s=640x360:r=15:d=14", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-vf", &vf, "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&video)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
        let mut log = EventLog::new(640, 360);
        for &(a, b) in moving {
            let mut t = a;
            while t < b {
                log.events.push(Event { t, kind: EventKind::Move { x: 50.0 + (t / 20.0) % 400.0, y: 80.0 } });
                t += 100.0;
            }
        }
        for &t in clicks {
            log.events.push(Event { t, kind: EventKind::Click { x: 100.0, y: 100.0, button: MouseButton::Left } });
        }
        log.events.sort_by(|a, b| a.t.total_cmp(&b.t));
        let ev = dir.join("demo.events.jsonl");
        log.save(&ev).ok()?;
        demolens_core::new_project(&video, Some(&ev)).ok()
    }

    fn silences_only() -> AutoCutOptions {
        AutoCutOptions { cut_fillers: false, cut_silences: true, min_silence_ms: 700.0, keep_ms: 180.0 }
    }

    #[test]
    fn a_silent_demo_with_activity_all_the_way_loses_nothing() {
        // Clicks about every 1.2 s with the pointer travelling in between, as in the report.
        let clicks: Vec<f64> = (1..12).map(|i| i as f64 * 1200.0).collect();
        let Some(mut p) = demo("busy", &[2.0, 5.0, 8.0, 11.0], &[(0.0, 13_000.0)], &clicks) else { return };
        let r = autocut_project(&mut p, &silences_only(), false).unwrap();
        assert!(r.cuts.is_empty(), "silent but busy: {:?}", r.cuts);
        assert_eq!(p.clips.len(), 1);
    }

    #[test]
    fn only_a_stretch_that_is_quiet_and_still_is_cut_and_with_padding() {
        // Busy until 5 s, then nothing at all until the end.
        let Some(mut p) = demo("tail", &[1.0, 3.0], &[(0.0, 5_000.0)], &[2_000.0, 4_800.0]) else { return };
        let r = autocut_project(&mut p, &silences_only(), true).unwrap();
        assert!(!r.cuts.is_empty(), "a long still and silent stretch is cut");
        for &(s, e, _) in &r.cuts {
            assert!(s >= 5_000.0 + 600.0, "padding after the last activity: {s}");
            assert!(e <= 14_500.0, "cut ends within the video (container length can run a little over): {e}");
        }
        // dry run changes nothing
        assert_eq!(p.clips.len(), 1);
    }

    #[test]
    fn something_changing_on_screen_without_any_input_is_not_cut() {
        // No pointer, no keys: only a loading screen that keeps changing.
        let Some(mut p) = demo("loading", &[1.0, 3.0, 5.0, 7.0, 9.0, 11.0, 13.0], &[], &[]) else { return };
        let r = autocut_project(&mut p, &silences_only(), true).unwrap();
        for &(s, e, _) in &r.cuts {
            for t in [1000.0, 3000.0, 5000.0, 7000.0, 9000.0, 11000.0, 13000.0] {
                assert!(!(s < t + 400.0 && e > t), "cut {s}..{e} swallows a change at {t}");
            }
        }
    }
}
