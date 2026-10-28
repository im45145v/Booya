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
    h.update(b"v3");
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
    if !audible(&audio) {
        // Nothing but a noise floor: whisper would invent words from it.
        return Ok(Vec::new());
    }
    // Quiet recordings are brought up to a normal level first; whisper drops
    // words it cannot hear. The cache key above uses the original samples.
    let audio = normalise_for_speech(audio);
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
    let voiced: Vec<(f64, f64)> = spans.iter().map(|&(a, b)| (a as f64 * 1000.0 / 16_000.0, b as f64 * 1000.0 / 16_000.0)).collect();
    if std::env::var_os("DEMOLENS_DEBUG_TRANSCRIBE").is_some() {
        eprintln!("transcribe: voiced {voiced:?}\ntranscribe: raw {:?}", words.iter().map(|w| (w.text.as_str(), w.start_ms.round(), w.end_ms.round())).collect::<Vec<_>>());
    }
    retime_implausible(&mut words, &voiced);
    if std::env::var_os("DEMOLENS_DEBUG_TRANSCRIBE").is_some() {
        eprintln!("transcribe: retimed {:?}", words.iter().map(|w| (w.text.as_str(), w.start_ms.round(), w.end_ms.round())).collect::<Vec<_>>());
    }
    if only_hallucination(&words) {
        words.clear();
    }
    if let Ok(s) = serde_json::to_string(&words) {
        let _ = write_private(&cache, &s);
    }
    Ok(words)
}

/// Word times from whisper without alignment heads are crude. The small models
/// sometimes stretch one word over several seconds and squash the words after
/// it into one instant at the end. A caption that starts at the very end of the
/// video then never shows. Words with such times are spread again over the time
/// the voice is really on (`voiced`, in ms), in proportion to their length.
pub fn retime_implausible(words: &mut [CaptionWord], voiced: &[(f64, f64)]) {
    let n = words.len();
    let chars = |w: &CaptionWord| w.text.chars().filter(|c| c.is_alphanumeric()).count().max(1);
    let collapsed = |w: &CaptionWord| w.end_ms - w.start_ms < 25.0;
    let bad = |w: &CaptionWord| collapsed(w) || w.end_ms - w.start_ms > 800.0 + 120.0 * chars(w) as f64;
    let mut i = 0;
    while i < n {
        if !bad(&words[i]) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < n && collapsed(&words[j]) {
            j += 1;
        }
        let lo = words[i].start_ms;
        let hi = if j < n { words[j].start_ms.max(words[j - 1].end_ms) } else { words[j - 1].end_ms };
        let pieces: Vec<(f64, f64)> = voiced.iter().filter_map(|&(a, b)| (a.max(lo) < b.min(hi)).then(|| (a.max(lo), b.min(hi)))).collect();
        let total: f64 = pieces.iter().map(|(a, b)| b - a).sum();
        if j - i < 2 && !(words[i].end_ms - words[i].start_ms > 800.0 + 120.0 * chars(&words[i]) as f64) || total < 60.0 * (j - i) as f64 {
            i = j;
            continue;
        }
        let at = |mut v: f64| {
            for (a, b) in &pieces {
                if v <= b - a {
                    return a + v;
                }
                v -= b - a;
            }
            pieces.last().map(|p| p.1).unwrap_or(lo)
        };
        let weights: Vec<f64> = (i..j).map(|k| chars(&words[k]) as f64 + 1.0).collect();
        let sum: f64 = weights.iter().sum();
        let mut acc = 0.0;
        for (k, wt) in (i..j).zip(weights) {
            words[k].start_ms = at(acc / sum * total);
            acc += wt;
            words[k].end_ms = at(acc / sum * total - 20.0).max(words[k].start_ms + 20.0);
        }
        i = j;
    }
}

/// Quietest 30 ms window (RMS) that counts as sound at all: about -52 dBFS.
/// A microphone that is on but picks up nothing sits below this.
const AUDIBLE_RMS: f32 = 0.0025;

/// Whether the track has anything above the noise floor to transcribe.
pub fn audible(audio: &[f32]) -> bool {
    let win = 16_000 * 30 / 1000;
    audio.chunks(win).any(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt() >= AUDIBLE_RMS)
}

/// Whisper's stock output for audio with no speech in it. A transcript that
/// is nothing but these is thrown away.
pub fn only_hallucination(words: &[CaptionWord]) -> bool {
    if words.is_empty() || words.len() > 4 {
        return false;
    }
    let text: String = words.iter().map(|w| w.text.trim().to_lowercase()).collect::<Vec<_>>().join(" ");
    let text = text.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    ["you", "thank you", "thanks", "thanks for watching", "thank you for watching", "bye", "bye bye", "so", "the", "uh", "um", "hmm", "mm"].contains(&text.as_str())
}

/// Scale quiet audio so its peak sits near 0.9, with at most 24 dB of gain.
/// Loud audio is left alone, so this never clips or changes a good recording.
pub fn normalise_for_speech(mut audio: Vec<f32>) -> Vec<f32> {
    let peak = audio.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 1e-4 && peak < 0.5 {
        let gain = (0.9 / peak).min(15.8);
        for x in &mut audio {
            *x = (*x * gain).clamp(-1.0, 1.0);
        }
    }
    audio
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

/// Load the model and run it over one second of silence. Proves the file is a
/// working model on this machine without needing any speech.
pub fn self_test(model: &Path) -> Result<String> {
    let t0 = std::time::Instant::now();
    let silence = vec![0.0f32; 16_000];
    let words = run_whisper(&silence, model, Some("en"), FILLER_PROMPT)?;
    Ok(format!("model loaded and ran in {:.1} s ({} words from silence)", t0.elapsed().as_secs_f64(), words.len()))
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
    #[test]
    fn quiet_audio_is_lifted_and_loud_audio_is_left_alone() {
        let quiet: Vec<f32> = (0..100).map(|i| 0.1 * ((i as f32) / 7.0).sin()).collect();
        let up = super::normalise_for_speech(quiet.clone());
        let peak = up.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.9).abs() < 0.02, "{peak}");
        let loud: Vec<f32> = quiet.iter().map(|x| x * 8.0).collect();
        assert_eq!(super::normalise_for_speech(loud.clone()), loud);
        let silent = vec![0.0f32; 10];
        assert_eq!(super::normalise_for_speech(silent.clone()), silent);
    }

    use super::*;

    #[test]
    fn a_noise_floor_is_not_audible_and_speech_level_sound_is() {
        let hiss: Vec<f32> = (0..32_000).map(|i| 0.001 * ((i * 7919 % 100) as f32 / 50.0 - 1.0)).collect();
        assert!(!audible(&hiss));
        assert!(!audible(&[]));
        let tone: Vec<f32> = (0..32_000).map(|i| 0.05 * (i as f32 * 0.05).sin()).collect();
        assert!(audible(&tone));
    }

    #[test]
    fn stock_hallucinations_on_their_own_are_dropped() {
        let w = |t: &str| CaptionWord { start_ms: 0.0, end_ms: 100.0, text: t.into() };
        assert!(only_hallucination(&[w("Thank"), w("you.")]));
        assert!(only_hallucination(&[w("Thanks"), w("for"), w("watching!")]));
        assert!(!only_hallucination(&[w("Thank"), w("you"), w("for"), w("coming"), w("today")]));
        assert!(!only_hallucination(&[w("Click"), w("here")]));
        assert!(!only_hallucination(&[]));
    }

    #[test]
    fn words_squashed_at_the_end_are_spread_over_the_voice() {
        let w = |a: f64, b: f64, t: &str| demolens_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into() };
        // What whisper tiny did on the CI recording: "Press" stretched over four
        // seconds, the rest of the sentence piled up at the end of the video.
        let mut words = vec![w(300.0, 900.0, "Let's"), w(7050.0, 11000.0, "Press"), w(12010.0, 12010.0, "Ctrl+S"), w(12010.0, 12010.0, "to"), w(12010.0, 12010.0, "save"), w(12010.0, 12010.0, "it,")];
        retime_implausible(&mut words, &[(0.0, 1000.0), (6900.0, 11100.0)]);
        assert_eq!((words[0].start_ms, words[0].end_ms), (300.0, 900.0), "a sane word is left alone");
        for pair in words[1..].windows(2) {
            assert!(pair[0].end_ms <= pair[1].start_ms + 1.0 && pair[0].start_ms < pair[0].end_ms, "{words:?}");
        }
        assert!(words.iter().all(|w| w.end_ms <= 11100.0), "{words:?}");
        assert!(words[1].end_ms < 8500.0, "'Press' is short again: {words:?}");
        assert!(words[2].start_ms > 7000.0 && words[2].start_ms < 9500.0, "{words:?}");
        assert!(words[5].start_ms > 9000.0, "{words:?}");
    }

    #[test]
    fn ci_recording_with_the_whole_tail_collapsed() {
        let w = |a: f64, b: f64, t: &str| demolens_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into() };
        let mut words = vec![w(4650.0, 5100.0, "march."), w(7050.0, 11040.0, "Press")];
        for t in ["Ctrl-Lester", "Save", "it,", "and", "now", "it", "is"] {
            words.push(w(12011.0, 12012.0, t));
        }
        words.push(w(12011.0, 12021.0, "saved."));
        retime_implausible(&mut words, &[(300.0, 5200.0), (7050.0, 11040.0)]);
        assert!(words.iter().all(|w| w.end_ms <= 11040.0), "{words:?}");
        assert!(words[1].end_ms < 8500.0, "{words:?}");
    }

    #[test]
    fn normal_timings_are_not_touched() {
        let w = |a: f64, b: f64, t: &str| demolens_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into() };
        let orig = vec![w(0.0, 400.0, "Open"), w(400.0, 700.0, "the"), w(700.0, 1500.0, "settings")];
        let mut words = orig.clone();
        retime_implausible(&mut words, &[(0.0, 1500.0)]);
        assert_eq!(words, orig);
    }

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
