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
}

pub fn speech_model(id: Option<&str>) -> Result<std::path::PathBuf> {
    match id {
        Some(id) => models::require(id),
        None => transcribe::default_model().context("no speech model installed. Download one in the model manager (whisper-base.en is a good start)"),
    }
}

/// Transcribe the project's audio into captions (word timed, in source time).
pub fn transcribe_project(p: &mut Project, model: Option<&str>, lang: Option<&str>) -> Result<TranscribeReport> {
    let model = speech_model(model)?;
    let words = transcribe::transcribe(&p.source, &model, lang)?;
    let name = model.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let mut r = finish_transcript(p, words)?;
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
    let samples = audio::decode_mono(&p.source, 16_000)?;
    captions::snap_to_speech(&mut words, &audio::detect_silences(&samples, 16_000, -38.0, 250.0));
    p.captions = captions::words_to_captions(&words, 42, 900.0);
    Ok(TranscribeReport { words: words.len(), fillers: words.iter().filter(|w| captions::is_filler(&w.text)).count(), lines: p.captions.len(), model: "cloud".into(), weak_model: false })
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
    let silences = if o.cut_silences {
        let samples = audio::decode_mono(&p.source, 16_000)?;
        audio::detect_silences(&samples, 16_000, -38.0, o.min_silence_ms)
    } else {
        Vec::new()
    };
    let mut planned = captions::plan_cuts(&words, &silences, o);
    // A quiet stretch is not dead air when the presenter is clicking or
    // typing in it: keep the on-screen action and only cut around it.
    if let Some(log) = p.events.as_ref().and_then(|e| demolens_core::events::EventLog::load(e).ok()) {
        let activity: Vec<f64> = log.clicks().map(|c| c.0).chain(log.keys().map(|k| k.0)).collect();
        planned = protect_activity(&planned, &activity, 400.0, 900.0, 300.0);
        // Also keep the cursor travelling to each click, so a cut never
        // makes the pointer jump straight onto the button.
        planned = protect_ranges(&planned, &click_travel(&log), 300.0);
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
    pub cut: bool,
    pub blur: bool,
    pub denoise: bool,
    pub min_silence_ms: f64,
}

impl Default for ReadyOptions {
    fn default() -> Self {
        Self { zoom: true, captions: true, cut: true, blur: true, denoise: true, min_silence_ms: 700.0 }
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
        progress(0.45, "Cutting filler words and long pauses...");
        if !has_audio {
            step("Auto cut", "skipped", "the video has no audio".into());
        } else {
            let co = AutoCutOptions { cut_fillers: have_words, cut_silences: true, min_silence_ms: o.min_silence_ms, ..Default::default() };
            match autocut_project(p, &co, false) {
                Ok(r) => {
                    let f = r.cuts.iter().filter(|c| c.2 == "filler").count();
                    step("Auto cut", "done", format!("{} filler words and {} pauses removed", f, r.cuts.len() - f));
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
