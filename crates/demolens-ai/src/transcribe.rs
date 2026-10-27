//! Local speech to text with whisper.cpp (via whisper-rs). Audio never
//! leaves the machine. Produces word level timestamps for captions,
//! karaoke highlighting and filler word cuts.

use anyhow::Result;
use demolens_core::project::CaptionWord;
use std::path::Path;

/// Prompt that nudges Whisper to keep disfluencies ("um", "uh") instead of
/// silently cleaning them up, so they can be cut.
pub const FILLER_PROMPT: &str = "Umm, so, uh, let me show you. Hmm, okay, um, here it is.";

pub fn default_model() -> Option<std::path::PathBuf> {
    crate::models::first_installed(crate::models::Kind::Speech).map(|m| crate::models::path_of(&m))
}

#[cfg(feature = "whisper")]
pub fn transcribe(video: &Path, model: &Path, language: Option<&str>) -> Result<Vec<CaptionWord>> {
    use anyhow::Context;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let audio = demolens_core::audio::decode_mono(video, 16_000)?;
    if audio.is_empty() {
        return Ok(Vec::new());
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
    p.set_initial_prompt(FILLER_PROMPT);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    state.full(p, &audio).context("whisper failed")?;

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
pub fn transcribe(_video: &Path, _model: &Path, _language: Option<&str>) -> Result<Vec<CaptionWord>> {
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
    fn punctuation_is_glued() {
        let w = |t: &str| CaptionWord { start_ms: 0.0, end_ms: 1.0, text: t.into() };
        let out = merge_fragments(vec![w("Hello"), w(","), w("world"), w("."), w("It"), w("'s")]);
        let t: Vec<_> = out.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(t, vec!["Hello,", "world.", "It's"]);
    }
}
