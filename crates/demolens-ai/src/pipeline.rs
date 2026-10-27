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
    let mut words = transcribe::transcribe(&p.source, &model, lang)?;
    let samples = audio::decode_mono(&p.source, 16_000)?;
    captions::snap_to_speech(&mut words, &audio::detect_silences(&samples, 16_000, -38.0, 250.0));
    p.captions = captions::words_to_captions(&words, 42, 900.0);
    Ok(TranscribeReport { words: words.len(), fillers: words.iter().filter(|w| captions::is_filler(&w.text)).count(), lines: p.captions.len() })
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
    let planned = captions::plan_cuts(&words, &silences, o);
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
