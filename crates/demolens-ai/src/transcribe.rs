//! Local speech to text with whisper.cpp (via whisper-rs). Audio never
//! leaves the machine. Produces word level timestamps for captions,
//! karaoke highlighting and filler word cuts.

use anyhow::Result;
use demolens_core::project::CaptionWord;
use std::path::Path;

/// Prompt that nudges Whisper to keep disfluencies ("um", "uh") instead of
/// silently cleaning them up, so they can be cut.
pub const FILLER_PROMPT: &str = "Umm, so, uh, let me show you. Hmm, okay, um, here it is.";

/// Installed speech models, best caption quality first.
pub const PREFERENCE: &[&str] = &["whisper-small.en", "whisper-large-v3-turbo-q5", "whisper-small", "whisper-base.en", "whisper-tiny.en"];

pub fn default_model() -> Option<std::path::PathBuf> {
    use crate::models;
    PREFERENCE
        .iter()
        .filter_map(|id| models::find(id).ok())
        .find(models::is_installed)
        .or_else(|| models::first_installed(models::Kind::Speech))
        .map(|m| models::path_of(&m))
}

/// Simple energy based voice activity detection. Returns the kept spans as
/// (start_sample, end_sample). Silences longer than `max_gap` samples are
/// shortened, which stops Whisper inventing words in long quiet stretches
/// and makes it faster.
pub fn speech_spans(audio: &[f32], rate: usize) -> Vec<(usize, usize)> {
    let win = rate * 30 / 1000;
    if audio.len() < win * 2 {
        return vec![(0, audio.len())];
    }
    let rms: Vec<f32> = audio.chunks(win).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[sorted.len() / 10];
    let thresh = (floor * 3.0).max(0.006);
    let pad = 10; // 300 ms either side
    let voiced: Vec<bool> = (0..rms.len()).map(|i| rms[i.saturating_sub(pad)..(i + pad + 1).min(rms.len())].iter().any(|&r| r > thresh)).collect();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < voiced.len() {
        if voiced[i] {
            let s = i;
            while i < voiced.len() && voiced[i] {
                i += 1;
            }
            spans.push((s * win, (i * win).min(audio.len())));
        } else {
            i += 1;
        }
    }
    if spans.is_empty() {
        return vec![];
    }
    spans
}

/// Join speech spans with 200 ms of silence between them. Returns the
/// compacted audio and a map of (compact_start, original_start, len).
pub fn compact(audio: &[f32], spans: &[(usize, usize)], rate: usize) -> (Vec<f32>, Vec<(usize, usize, usize)>) {
    let gap = rate / 5;
    let mut out = Vec::with_capacity(audio.len());
    let mut map = Vec::new();
    for &(s, e) in spans {
        if !out.is_empty() {
            out.extend(std::iter::repeat(0.0).take(gap));
        }
        map.push((out.len(), s, e - s));
        out.extend_from_slice(&audio[s..e]);
    }
    (out, map)
}

/// Compact time (ms) back to original time (ms).
pub fn uncompact_ms(t_ms: f64, map: &[(usize, usize, usize)], rate: usize) -> f64 {
    let t = (t_ms / 1000.0 * rate as f64) as usize;
    let seg = map.iter().rev().find(|(c, _, _)| *c <= t).copied().unwrap_or((0, 0, 0));
    let off = (t - seg.0).min(seg.2);
    (seg.1 + off) as f64 * 1000.0 / rate as f64
}

/// Cache key: audio content, model file and options.
fn cache_path(audio: &[f32], model: &Path, language: Option<&str>, prompt: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for x in audio {
        h.update(x.to_le_bytes());
    }
    h.update(model.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
    h.update(std::fs::metadata(model).map(|m| m.len()).unwrap_or(0).to_le_bytes());
    h.update(language.unwrap_or("en"));
    h.update(prompt);
    h.update(b"v2");
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    crate::models::models_dir().parent().map(|p| p.join("transcripts")).unwrap_or_else(|| "transcripts".into()).join(format!("{hex}.json"))
}

/// Write a cache entry readable by this user only.
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    use std::io::Write;
    o.open(path)?.write_all(data.as_bytes())
}

/// Delete all cached transcripts (part of "delete my data").
pub fn clear_cache() -> Result<()> {
    let d = crate::models::models_dir().parent().map(|p| p.join("transcripts")).unwrap_or_else(|| "transcripts".into());
    if d.exists() {
        std::fs::remove_dir_all(d)?;
    }
    Ok(())
}

pub fn transcribe(video: &Path, model: &Path, language: Option<&str>) -> Result<Vec<CaptionWord>> {
    let vocab = std::env::var("DEMOLENS_WHISPER_PROMPT").ok();
    transcribe_with(video, model, language, vocab.as_deref())
}

/// Transcribe with an optional extra prompt (product names, jargon). The
/// result is cached by a hash of the audio, so re-running is instant.
pub fn transcribe_with(video: &Path, model: &Path, language: Option<&str>, vocabulary: Option<&str>) -> Result<Vec<CaptionWord>> {
    let audio = demolens_core::audio::decode_mono(video, 16_000)?;
    if audio.is_empty() {
        return Ok(Vec::new());
    }
    let prompt = match vocabulary.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => format!("{FILLER_PROMPT} {v}."),
        None => FILLER_PROMPT.to_string(),
    };
    let cache = cache_path(&audio, model, language, &prompt);
    if std::env::var_os("DEMOLENS_NO_TRANSCRIPT_CACHE").is_none() {
        if let Some(w) = std::fs::read_to_string(&cache).ok().and_then(|s| serde_json::from_str::<Vec<CaptionWord>>(&s).ok()) {
            return Ok(w);
        }
    }
    let spans = speech_spans(&audio, 16_000);
    if spans.is_empty() {
        return Ok(Vec::new());
    }
    let (compacted, map) = compact(&audio, &spans, 16_000);
    let mut words = run_whisper(&compacted, model, language, &prompt)?;
    for w in &mut words {
        w.start_ms = uncompact_ms(w.start_ms, &map, 16_000);
        w.end_ms = uncompact_ms(w.end_ms, &map, 16_000).max(w.start_ms);
    }
    words.retain(|w| !demolens_core::captions::is_nonspeech(&w.text));
    if let Ok(s) = serde_json::to_string(&words) {
        let _ = write_private(&cache, &s);
    }
    Ok(words)
}

#[cfg(feature = "whisper")]
fn run_whisper(audio: &[f32], model: &Path, language: Option<&str>, prompt: &str) -> Result<Vec<CaptionWord>> {
    use anyhow::Context;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    // Keep whisper.cpp's model loading chatter off the terminal unless asked.
    if std::env::var_os("DEMOLENS_DEBUG_WHISPER").is_none() {
        whisper_rs::install_logging_hooks();
    }
    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .context("could not load the whisper model")?;
    let mut state = ctx.create_state().context("whisper state")?;
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) as i32;
    p.set_n_threads(threads);
    p.set_language(Some(language.unwrap_or("en")));
    p.set_token_timestamps(true);
    p.set_split_on_word(true);
    p.set_max_len(1);
    p.set_initial_prompt(prompt);
    p.set_suppress_blank(true);
    p.set_no_speech_thold(0.6);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    state.full(p, audio).context("whisper failed")?;

    let mut words = Vec::new();
    for seg in state.as_iter() {
        let text = seg.to_str_lossy().map(|s| s.trim().to_string()).unwrap_or_default();
        if text.is_empty() || text.starts_with('[') || text.starts_with('(') {
            continue;
        }
        // Timestamps are in centiseconds.
        words.push(CaptionWord { start_ms: seg.start_timestamp() as f64 * 10.0, end_ms: seg.end_timestamp() as f64 * 10.0, text });
    }
    Ok(merge_fragments(words))
}

#[cfg(not(feature = "whisper"))]
fn run_whisper(_audio: &[f32], _model: &Path, _language: Option<&str>, _prompt: &str) -> Result<Vec<CaptionWord>> {
    anyhow::bail!("this build of DemoLens has no speech recognition (built without the 'whisper' feature)")
}

/// whisper.cpp sometimes emits punctuation or word pieces as separate
/// segments. Glue anything that does not start with a space-separated word
/// (for example "," or "'s") onto the previous word.
pub fn merge_fragments(words: Vec<CaptionWord>) -> Vec<CaptionWord> {
    let mut out: Vec<CaptionWord> = Vec::new();
    for w in words {
        let is_fragment = w.text.chars().next().is_some_and(|c| !c.is_alphanumeric() && c != '$' && c != '#');
        match out.last_mut() {
            Some(prev) if is_fragment => {
                prev.text.push_str(&w.text);
                prev.end_ms = prev.end_ms.max(w.end_ms);
            }
            _ => out.push(w),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_compacts_long_silence_and_maps_back() {
        let rate = 16_000;
        let tone = |n: usize| (0..n).map(|i| (i as f32 * 0.05).sin() * 0.3).collect::<Vec<f32>>();
        let mut a = tone(rate);
        a.extend(vec![0.0; rate * 5]);
        a.extend(tone(rate));
        let spans = speech_spans(&a, rate);
        assert_eq!(spans.len(), 2);
        let (c, map) = compact(&a, &spans, rate);
        assert!(c.len() < a.len() / 2);
        // A word at the start of the second span maps back to about 6 s.
        let t = map[1].0 as f64 * 1000.0 / rate as f64 + 400.0;
        let orig = uncompact_ms(t, &map, rate);
        assert!((orig - 6100.0).abs() < 80.0, "{orig}");
        assert!(speech_spans(&vec![0.0; rate * 3], rate).is_empty());
    }
    #[test]
    fn punctuation_is_glued() {
        let w = |t: &str| CaptionWord { start_ms: 0.0, end_ms: 1.0, text: t.into() };
        let out = merge_fragments(vec![w("Hello"), w(","), w("world"), w("."), w("It"), w("'s")]);
        let t: Vec<_> = out.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(t, vec!["Hello,", "world.", "It's"]);
    }
}
