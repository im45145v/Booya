//! Local speech to text with whisper.cpp (via whisper-rs). Audio never
//! leaves the machine. Produces word level timestamps for captions,
//! karaoke highlighting and filler word cuts.

use anyhow::Result;
use booya_core::project::CaptionWord;
use std::path::Path;

/// Prompt that nudges Whisper to keep disfluencies ("um", "uh") instead of
/// silently cleaning them up, so they can be cut.
pub const FILLER_PROMPT: &str = "Umm, so, uh, let me show you. Hmm, okay, um, here it is.";

/// Installed speech models, best caption quality first.
pub const PREFERENCE: &[&str] = &["whisper-small.en", "whisper-large-v3-turbo-q5", "whisper-small", "whisper-base.en", "whisper-tiny.en",
];

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
    let voiced: Vec<bool> = (0..rms.len()).map(|i| {
            rms[i.saturating_sub(pad)..(i + pad + 1).min(rms.len())].iter().any(|&r| r > thresh)
        }).collect();
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
pub fn compact(audio: &[f32], spans: &[(usize, usize)], rate: usize,
) -> (Vec<f32>, Vec<(usize, usize, usize)>) {
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
fn cache_path(audio: &[f32], model: &Path, language: Option<&str>, prompt: &str,
) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for x in audio {
        h.update(x.to_le_bytes());
    }
    h.update(model.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
    );
    h.update(std::fs::metadata(model).map(|m| m.len()).unwrap_or(0).to_le_bytes(),
    );
    h.update(language.unwrap_or("en"));
    h.update(prompt);
    h.update(b"v4");
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
    let vocab = std::env::var("BOOYA_WHISPER_PROMPT").ok();
    transcribe_with_cancel(video, model, language, vocab.as_deref(), || false)
}

/// Transcribe with an optional extra prompt (product names, jargon). The
/// result is cached by a hash of the audio, so re-running is instant.
pub fn transcribe_with(video: &Path, model: &Path, language: Option<&str>, vocabulary: Option<&str>,
) -> Result<Vec<CaptionWord>> {
    transcribe_with_cancel(video, model, language, vocabulary, || false)
}

/// Transcribe while checking cancellation between Whisper windows.
pub fn transcribe_with_cancel(
    video: &Path,
    model: &Path,
    language: Option<&str>,
    vocabulary: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<CaptionWord>> {
    let audio = booya_core::audio::decode_mono(video, 16_000)?;
    anyhow::ensure!(!cancelled(), "transcription cancelled");
    if audio.is_empty() {
        return Ok(Vec::new());
    }
    let prompt = match vocabulary.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => format!("{FILLER_PROMPT} {v}."),
        None => FILLER_PROMPT.to_string(),
    };
    let cache = cache_path(&audio, model, language, &prompt);
    if std::env::var_os("BOOYA_NO_TRANSCRIPT_CACHE").is_none() {
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
    // Long recordings go through whisper in overlapping chunks. Whisper's own
    // 30 s windows are the usual place where fast, continuous speech loses
    // whole sentences; here every second is heard by at least one full chunk.
    let chunks = plan_chunks(compacted.len(), 16_000);
    let parts = run_ranges(&compacted, model, language, &prompt, &chunks, false, &cancelled,
    )?;
    let mut words = merge_chunks(&chunks, parts, 16_000);
    for w in &mut words {
        w.start_ms = uncompact_ms(w.start_ms, &map, 16_000);
        w.end_ms = uncompact_ms(w.end_ms, &map, 16_000).max(w.start_ms);
    }
    words.retain(|w| !booya_core::captions::is_nonspeech(&w.text));
    let voiced: Vec<(f64, f64)> = spans.iter().map(|&(a, b)| (a as f64 * 1000.0 / 16_000.0, b as f64 * 1000.0 / 16_000.0)).collect();
    // Where the voice is clearly on but nothing was transcribed, listen again
    // to just that stretch.
    let tight = voice_onsets(&audio, 16_000);
    // Words whisper piled up at the end of a chunk are first spread over the
    // voice they belong to, so they are not taken for missing and heard again
    // (a second, different hearing would only duplicate them).
    retime_implausible(&mut words, &voiced);
    let holes: Vec<(f64, f64)> = uncovered(&words, &tight, 1_500.0).into_iter().flat_map(|h| split_hole(h, 7_000.0)).collect();
    if !holes.is_empty() {
        let ranges: Vec<(usize, usize)> = holes.iter().map(|&(a, b)| {
                (((a - 300.0).max(0.0) * 16.0) as usize, (((b + 300.0) * 16.0) as usize).min(audio.len()),
                )
            }).collect();
        let mut found =
            match run_ranges(&audio, model, language, &prompt, &ranges, false, &cancelled) {
                Ok(found) => found,
                Err(_) if cancelled() => anyhow::bail!("transcription cancelled"),
                Err(_) => Vec::new(),
            };
        anyhow::ensure!(!cancelled(), "transcription cancelled");
        // A stretch that still gave fewer than two words is heard once more,
        // this time without whisper's "this is not speech" shortcut.
        let again: Vec<usize> = (0..ranges.len()).filter(|&i| {
                found.get(i).map(|w| {
                        w.iter().filter(|w| !booya_core::captions::is_nonspeech(&w.text)).count() < 2
                    }).unwrap_or(true)
            }).collect();
        if !again.is_empty() {
            let rs: Vec<(usize, usize)> = again.iter().map(|&i| ranges[i]).collect();
            if let Ok(second) = run_ranges(&audio, model, language, &prompt, &rs, true, &cancelled) {
                for (i, ws) in again.into_iter().zip(second) {
                    if found.len() <= i {
                        found.resize(i + 1, Vec::new());
                    }
                    found[i] = ws;
                }
            }
            anyhow::ensure!(!cancelled(), "transcription cancelled");
        }
        {
            for (((a, b), (from, to)), ws) in holes.iter().zip(&ranges).zip(found) {
                // Whisper's times are from the start of the stretch it was given.
                let off = *from as f64 * 1000.0 / 16_000.0;
                let end = *to as f64 * 1000.0 / 16_000.0;
                let ws: Vec<CaptionWord> = ws
                    .into_iter()
                    .map(|mut w| {
                        w.start_ms += off;
                        w.end_ms += off;
                        w
                    })
                    .filter(|w| !booya_core::captions::is_nonspeech(&w.text))
                    .collect();
                let mut ws = place_heard_again(ws, &tight, (*a, *b), (off, end));
                if std::env::var_os("BOOYA_DEBUG_TRANSCRIBE").is_some() {
                    eprintln!("transcribe: hole {a:.0}..{b:.0} heard again {:?}", ws.iter().map(|w| (w.text.as_str(), w.start_ms.round())).collect::<Vec<_>>());
                }
                if ws.len() >= 2 && !only_hallucination(&ws) {
                    words.append(&mut ws);
                }
            }
            words.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        }
    }
    if std::env::var_os("BOOYA_DEBUG_TRANSCRIBE").is_some() {
        eprintln!("transcribe: voiced {voiced:?}\ntranscribe: onsets {tight:?}\ntranscribe: raw {:?}", words.iter().map(|w| (w.text.as_str(), w.start_ms.round(), w.end_ms.round())).collect::<Vec<_>>());
    }
    fix_times(&mut words, &voiced, &tight);
    if std::env::var_os("BOOYA_DEBUG_TRANSCRIBE").is_some() {
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


/// Brings whisper's word times onto the voice. Whisper's word times are
/// crude: a small model can hold one word over several seconds and then pile
/// everything after it into one instant, or past the end of the audio. The
/// words are right but their times are not, and a caption with no time on
/// screen never shows, so a whole sentence went missing.
///
/// The steps: spread local pile-ups over the voice they belong to; when the
/// times as a whole cannot be believed (see `times_are_unbelievable`) lay all
/// the words over the voiced stretches in proportion to their length instead;
/// finally move any word still sitting in silence onto the voice.
pub fn fix_times(words: &mut Vec<CaptionWord>, voiced: &[(f64, f64)], tight: &[(f64, f64)]) {
    retime_implausible(words, voiced);
    if times_are_unbelievable(words, tight) {
        respread_all(words, tight);
    }
    anchor_to_voice(words, tight);
    // A caption needs a moment on screen: no word may end up with no time.
    for i in 0..words.len() {
        let next = words.get(i + 1).map(|n| n.start_ms).unwrap_or(f64::MAX);
        let w = &mut words[i];
        if w.end_ms - w.start_ms < 60.0 {
            w.end_ms = (w.start_ms + 60.0).min(next.max(w.start_ms + 20.0));
        }
    }
}

/// Proportional positions of the words over the voice: each takes a share of
/// the voiced time in proportion to its length.
fn proportional(words: &[CaptionWord], voiced: &[(f64, f64)]) -> Option<Vec<CaptionWord>> {
    let total: f64 = voiced.iter().map(|v| v.1 - v.0).sum();
    if words.len() < 4 || total < 80.0 * words.len() as f64 {
        return None;
    }
    let mut v = words.to_vec();
    spread(&mut v, voiced, total);
    Some(v)
}

/// True when the times could not be what was spoken: words after the voice
/// has ended, a run of words with no duration, or words more than a few
/// seconds from where their share of the voice puts them.
pub fn times_are_unbelievable(words: &[CaptionWord], voiced: &[(f64, f64)]) -> bool {
    let Some(end) = voiced.last().map(|v| v.1) else { return false;
    };
    let Some(reference) = proportional(words, voiced) else { return false;
    };
    let late = words.iter().filter(|w| w.start_ms > end + 500.0).count();
    let mut squashed = 0;
    let mut run = 0;
    for w in words {
        if w.end_ms - w.start_ms < 40.0 {
            run += 1;
            squashed = squashed.max(run);
        } else {
            run = 0;
        }
    }
    // Only meaningful when the words run to the end of the voice. A transcript
    // that stops early (words missing) has correct times for what it has.
    let reaches = words.last().map(|w| w.end_ms >= end - 1_500.0).unwrap_or(false);
    let far = if reaches { words.iter().zip(&reference).filter(|(w, r)| (w.start_ms - r.start_ms).abs() > 3_500.0).count() } else { 0 };
    late >= 1 || squashed >= 3 || far >= 2
}

fn respread_all(words: &mut Vec<CaptionWord>, voiced: &[(f64, f64)]) {
    if let Some(v) = proportional(words, voiced) {
        *words = v;
    }
}

/// Where the voice really is, in ms: runs of at least 60 ms that are clearly
/// above the room noise, with pauses under 150 ms bridged. Stricter than
/// `speech_spans` (which pads by 300 ms to be safe for whisper), so it can say
/// when speech starts. The level is relative: a hum or a hissy microphone does
/// not count as voice.
pub fn voice_onsets(audio: &[f32], rate: usize) -> Vec<(f64, f64)> {
    let win = rate * 20 / 1000;
    if audio.len() < win * 3 {
        return vec![];
    }
    let rms: Vec<f32> = audio.chunks(win).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[sorted.len() / 10];
    let loud = sorted[sorted.len() * 9 / 10];
    let thresh = (floor * 3.0).max(floor + 0.12 * (loud - floor)).max(0.004);
    let ms = |w: usize| w as f64 * 20.0;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < rms.len() {
        if rms[i] > thresh {
            let s = i;
            while i < rms.len() && rms[i] > thresh {
                i += 1;
            }
            if i - s >= 3 {
                runs.push((s, i));
            }
        } else {
            i += 1;
        }
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in runs {
        match out.last_mut() {
            Some(l) if ms(a) - l.1 < 150.0 => l.1 = ms(b),
            _ => out.push((ms(a), ms(b))),
        }
    }
    out
}

/// Put every word where there is voice. Whisper gives the first word of a
/// recording a start near 0 whatever came before it, and stretches words over
/// pauses; a caption then shows seconds before anyone speaks. A word that sits
/// in silence (more than 120 ms before a voiced stretch or 150 ms after it) is
/// moved to the next voiced stretch, and a run of such words is spread along
/// it. Words already on voice are not touched, so this is a no-op on good
/// timings. `voiced` is from `voice_onsets`, in ms.
pub fn anchor_to_voice(words: &mut [CaptionWord], voiced: &[(f64, f64)]) {
    if voiced.is_empty() || words.is_empty() {
        return;
    }
    let near = |t: f64| {
        voiced.iter().any(|&(a, b)| t >= a - 120.0 && t <= b + 150.0)
    };
    let n = words.len();
    let mut i = 0;
    while i < n {
        if near(words[i].start_ms) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && !near(words[j].start_ms) {
            j += 1;
        }
        let k = (j - i) as f64;
        let t0 = words[i].start_ms;
        let (lo, hi) = match voiced.iter().find(|s| s.0 > t0) {
            Some(&(a, b)) => {
                let mut hi = b.min(a + k * 350.0).max(a + k * 60.0);
                // Not past the next word that is already on the voice.
                if j < n && words[j].start_ms > a + k * 60.0 {
                    hi = hi.min(words[j].start_ms);
                }
                (a, hi)
            }
            None => {
                let &(a, b) = voiced.last().unwrap();
                ((b - k * 350.0).max(a), b)
            }
        };
        let slot = (hi - lo) / k;
        for (m, w) in words[i..j].iter_mut().enumerate() {
            w.start_ms = lo + slot * m as f64;
            w.end_ms = (w.start_ms + slot * 0.95).max(w.start_ms + 20.0);
        }
        i = j;
    }
    // Order is kept: a word never starts before the one before it.
    for m in 1..n {
        let prev = words[m - 1].start_ms;
        if words[m].start_ms < prev {
            words[m].start_ms = prev;
            words[m].end_ms = words[m].end_ms.max(prev + 20.0);
        }
    }
}

/// Seconds of audio per whisper chunk and how much neighbours share.
pub const CHUNK_S: usize = 24;
pub const OVERLAP_S: usize = 4;

/// Chunks (start, end in samples) that cover all of the audio, each sharing
/// `OVERLAP_S` seconds with the next one.
pub fn plan_chunks(len: usize, rate: usize) -> Vec<(usize, usize)> {
    let chunk = CHUNK_S * rate;
    let overlap = OVERLAP_S * rate;
    if len <= chunk + overlap {
        return vec![(0, len)];
    }
    let step = chunk - overlap;
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        out.push((start, (start + chunk).min(len)));
        if start + chunk >= len {
            break;
        }
        start += step;
    }
    out
}

fn norm_word(t: &str) -> String {
    t.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Join the words of overlapping chunks (each in ms from the start of its own
/// chunk) into one transcript on a shared clock. A word is taken from any
/// chunk that heard it, except those cut short by the edge of a chunk, so a
/// word one chunk skipped is still there when its neighbour caught it. A word
/// two chunks both heard is kept once. A word the speaker repeats inside one
/// chunk is never dropped.
pub fn merge_chunks(chunks: &[(usize, usize)], parts: Vec<Vec<CaptionWord>>, rate: usize,
) -> Vec<CaptionWord> {
    const EDGE: f64 = 300.0;
    let to_ms = |s: usize| s as f64 * 1000.0 / rate as f64;
    let mut all: Vec<(usize, CaptionWord)> = Vec::new();
    for (k, ws) in parts.into_iter().enumerate() {
        let (off, len) = chunks.get(k).map(|c| (to_ms(c.0), to_ms(c.1 - c.0))).unwrap_or((0.0, f64::MAX));
        for mut w in ws {
            let cut_short = (k > 0 && w.start_ms < EDGE) || (k + 1 < chunks.len() && w.end_ms > len - EDGE);
            if cut_short {
                continue;
            }
            w.start_ms += off;
            w.end_ms += off;
            all.push((k, w));
        }
    }
    all.sort_by(|a, b| a.1.start_ms.total_cmp(&b.1.start_ms));
    let mut out: Vec<(usize, CaptionWord)> = Vec::new();
    for (k, w) in all {
        let dup = out.iter().rev().take(8).any(|(pk, p)| {
            *pk != k && norm_word(&p.text) == norm_word(&w.text) && (p.start_ms - w.start_ms).abs() < 600.0
        });
        if !dup {
            out.push((k, w));
        }
    }
    out.into_iter().map(|(_, w)| w).collect()
}

/// Stretches between words where the voice is on for at least `min_ms` in
/// total (`voiced`, from `voice_onsets`, which comes in many short runs for
/// fast speech) and not a single word was transcribed. A word's end is taken
/// as at most a second after its start, because whisper sometimes stretches
/// one word over everything it dropped.
pub fn uncovered(words: &[CaptionWord], voiced: &[(f64, f64)], min_ms: f64) -> Vec<(f64, f64)> {
    let Some(end) = voiced.last().map(|v| v.1) else { return Vec::new();
    };
    let mut gaps: Vec<(f64, f64)> = Vec::new();
    let mut at = 0.0f64;
    for w in words {
        if w.start_ms > at {
            gaps.push((at, w.start_ms));
        }
        at = at.max(w.end_ms.min(w.start_ms + 1_000.0));
    }
    if end > at {
        gaps.push((at, end));
    }
    let mut out = Vec::new();
    for (a, b) in gaps {
        let inside: Vec<(f64, f64)> = voiced.iter().filter(|v| v.1 > a && v.0 < b).map(|v| (v.0.max(a), v.1.min(b))).collect();
        let total: f64 = inside.iter().map(|v| v.1 - v.0).sum();
        if total >= min_ms {
            out.push((inside[0].0, inside[inside.len() - 1].1));
        }
    }
    out
}

/// Word times from whisper without alignment heads are crude. The small models
/// sometimes stretch one word over several seconds and squash the words after
/// it into one instant at the end. A caption that starts at the very end of the
/// video then never shows. Words with such times are spread again over the time
/// the voice is really on (`voiced`, in ms), in proportion to their length.
pub fn retime_implausible(words: &mut [CaptionWord], voiced: &[(f64, f64)]) {
    let n = words.len();
    let chars = |w: &CaptionWord| {
        w.text.chars().filter(|c| c.is_alphanumeric()).count().max(1)
    };
    let collapsed = |w: &CaptionWord| w.end_ms - w.start_ms < 25.0;
    let bad = |w: &CaptionWord| collapsed(w) || w.end_ms - w.start_ms > 800.0 + 120.0 * chars(w) as f64;
    let mut i = 0;
    while i < n {
        if !bad(&words[i]) {
            i += 1;
            continue;
        }
        // The run is the bad word and what piled up after it, including a word
        // or two between them that happens to have a believable length.
        let mut j = i + 1;
        let stretched = |w: &CaptionWord| w.end_ms - w.start_ms > 800.0 + 120.0 * chars(w) as f64;
        while j < n && (collapsed(&words[j]) || (!stretched(&words[j]) && (1..=2).any(|d| {
                        j + d < n && collapsed(&words[j + d]) && (1..d).all(|e| !stretched(&words[j + e]))
                    }))) {
            j += 1;
        }
        if j - i < 2 {
            // A lone word held over a pause: it ends when it has been said.
            if stretched(&words[i]) {
                words[i].end_ms = words[i].start_ms + (150.0 + 110.0 * chars(&words[i]) as f64).min(words[i].end_ms - words[i].start_ms);
            }
            i = j;
            continue;
        }
        let lo = words[i].start_ms;
        let hi = if j < n { words[j].start_ms.max(words[j - 1].end_ms) } else { words[j - 1].end_ms };
        let pieces: Vec<(f64, f64)> = voiced.iter().filter_map(|&(a, b)| (a.max(lo) < b.min(hi)).then(|| (a.max(lo), b.min(hi)))).collect();
        let total: f64 = pieces.iter().map(|(a, b)| b - a).sum();
        if total < 60.0 * (j - i) as f64 {
            i = j;
            continue;
        }
        spread(&mut words[i..j], &pieces, total);
        i = j;
    }
}

/// Lays `words` one after the other over the stretches in `pieces` (ms, in
/// order, `total` ms long in all), each taking a share in proportion to its
/// length.
fn spread(words: &mut [CaptionWord], pieces: &[(f64, f64)], total: f64) {
    let chars = |w: &CaptionWord| {
        w.text.chars().filter(|c| c.is_alphanumeric()).count().max(1)
    };
    let lo = pieces.first().map(|p| p.0).unwrap_or(0.0);
    let at = |mut v: f64| {
        for (a, b) in pieces {
            if v <= b - a {
                return a + v;
            }
            v -= b - a;
        }
        pieces.last().map(|p| p.1).unwrap_or(lo)
    };
    let weights: Vec<f64> = words.iter().map(|w| chars(w) as f64 + 1.0).collect();
    let sum: f64 = weights.iter().sum();
    let mut acc = 0.0;
    for (w, wt) in words.iter_mut().zip(weights) {
        w.start_ms = at(acc / sum * total);
        acc += wt;
        w.end_ms = at(acc / sum * total - 20.0).max(w.start_ms + 20.0);
    }
}

/// Words heard again in a stretch whose times are mostly unbelievable (held
/// for seconds, or squashed into an instant) are laid over the voice in that
/// stretch instead: the words are right, the times are not.
pub fn respread_if_implausible(words: &mut [CaptionWord], voiced: &[(f64, f64)], from: f64, to: f64,
) -> bool {
    let chars = |w: &CaptionWord| {
        w.text.chars().filter(|c| c.is_alphanumeric()).count().max(1)
    };
    let bad = |w: &CaptionWord| {
        w.end_ms - w.start_ms < 25.0 || w.end_ms - w.start_ms > 800.0 + 120.0 * chars(w) as f64
    };
    if words.len() < 2 || (words.iter().filter(|w| bad(w)).count() as f64) < 0.3 * words.len() as f64 {
        return false;
    }
    let pieces: Vec<(f64, f64)> = voiced.iter().filter_map(|&(a, b)| (a.max(from) < b.min(to)).then(|| (a.max(from), b.min(to)))).collect();
    let total: f64 = pieces.iter().map(|(a, b)| b - a).sum();
    if total < 60.0 * words.len() as f64 {
        return false;
    }
    spread(words, &pieces, total);
    true
}

/// Words heard again in a hole `(a, b)` of the voice, placed inside it. Whisper
/// often gets the words right but stamps them late (or past the end of the
/// short clip it was given), and dropping words by their time lost whole
/// sentences. Words that fall outside the hole, or whose times are not
/// believable, are laid over the voice in the hole instead. `window` is the
/// stretch of audio that was heard (from, to), in ms.
pub fn place_heard_again(mut ws: Vec<CaptionWord>, voiced: &[(f64, f64)], (a, b): (f64, f64), window: (f64, f64),
) -> Vec<CaptionWord> {
    if ws.len() < 2 {
        return ws;
    }
    let outside = ws.iter().filter(|w| w.start_ms < a - 100.0 || w.start_ms >= b).count();
    if outside > 0 {
        let pieces: Vec<(f64, f64)> = voiced.iter().filter_map(|&(x, y)| (x.max(a) < y.min(b)).then(|| (x.max(a), y.min(b)))).collect();
        let total: f64 = pieces.iter().map(|(x, y)| y - x).sum();
        if total >= 60.0 * ws.len() as f64 {
            spread(&mut ws, &pieces, total);
        } else {
            // Not enough voice to lay them over: keep them in the hole in order.
            let slot = (b - a) / ws.len() as f64;
            for (i, w) in ws.iter_mut().enumerate() {
                w.start_ms = a + slot * i as f64;
                w.end_ms = w.start_ms + slot * 0.95;
            }
        }
    } else {
        respread_if_implausible(&mut ws, voiced, window.0, window.1);
    }
    ws
}

/// A voiced stretch with nothing transcribed, cut into pieces of at most
/// `max_ms` (whisper tiny does much better on a few seconds than on ten).
pub fn split_hole((a, b): (f64, f64), max_ms: f64) -> Vec<(f64, f64)> {
    let n = ((b - a) / max_ms).ceil().max(1.0) as usize;
    let step = (b - a) / n as f64;
    (0..n).map(|i| (a + step * i as f64, a + step * (i + 1) as f64)).collect()
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
    ["you", "thank you", "thanks", "thanks for watching", "thank you for watching", "bye", "bye bye", "so", "the", "uh", "um", "hmm", "mm",
    ].contains(&text.as_str())
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

/// Run whisper over each range (start, end in samples) of `audio`. The words of
/// range k come back in ms from the start of that range.
#[cfg(feature = "whisper")]
fn run_ranges(audio: &[f32], model: &Path, language: Option<&str>, prompt: &str, ranges: &[(usize, usize)], lenient: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Vec<CaptionWord>>> {
    use anyhow::Context;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    anyhow::ensure!(!cancelled(), "transcription cancelled");

    // Keep whisper.cpp's model loading chatter off the terminal unless asked.
    if std::env::var_os("BOOYA_DEBUG_WHISPER").is_none() {
        whisper_rs::install_logging_hooks();
    }
    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .context("could not load the whisper model")?;
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) as i32;
    let mut all = Vec::new();
    for &(from, to) in ranges {
        anyhow::ensure!(!cancelled(), "transcription cancelled");
        let mut state = ctx.create_state().context("whisper state")?;
        // Greedy, with a fallback to warmer sampling when a window looks bad
        // (repeats, low confidence): that is what recovers skipped words.
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 3 });
        p.set_n_threads(threads);
        p.set_language(Some(language.unwrap_or("en")));
        p.set_token_timestamps(true);
        p.set_split_on_word(true);
        p.set_max_len(1);
        p.set_initial_prompt(prompt);
        // Each chunk stands alone: text carried over from the one before is
        // what makes whisper skip ahead or repeat itself.
        p.set_no_context(true);
        p.set_suppress_blank(true);
        // Less eager to call a window "no speech": fast talkers read as quiet.
        // A lenient pass (used on stretches where the voice is plainly on
        // but nothing was written) never gives up on a window as "no speech".
        p.set_no_speech_thold(if lenient { 1.0 } else { 0.35 });
        p.set_temperature(0.0);
        p.set_temperature_inc(if lenient { 0.4 } else { 0.2 });
        p.set_entropy_thold(2.4);
        p.set_logprob_thold(if lenient { -2.0 } else { -1.0 });
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_special(false);
        p.set_print_timestamps(false);
        state.full(p, &audio[from.min(audio.len())..to.min(audio.len())]).context("whisper failed")?;

        let mut words = Vec::new();
        for seg in state.as_iter() {
            let text = seg.to_str_lossy().map(|s| s.trim().to_string()).unwrap_or_default();
            if text.is_empty() || text.starts_with('[') || text.starts_with('(') {
                continue;
            }
            // Timestamps are in centiseconds.
            words.push(CaptionWord { start_ms: seg.start_timestamp() as f64 * 10.0, end_ms: seg.end_timestamp() as f64 * 10.0, text,
            });
        }
        all.push(merge_fragments(words));
    }
    Ok(all)
}

#[cfg(not(feature = "whisper"))]
fn run_ranges(_audio: &[f32], _model: &Path, _language: Option<&str>, _prompt: &str, _ranges: &[(usize, usize)], _lenient: bool,
    _cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Vec<CaptionWord>>> {
    anyhow::bail!("this build of Booya has no speech recognition (built without the 'whisper' feature)")
}

fn run_whisper(audio: &[f32], model: &Path, language: Option<&str>, prompt: &str,
) -> Result<Vec<CaptionWord>> {
    Ok(run_ranges(audio, model, language, prompt, &[(0, audio.len())], false,
        &|| false,
    )?.into_iter().next().unwrap_or_default())
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
        let w = |t: &str| CaptionWord { start_ms: 0.0, end_ms: 100.0, text: t.into(),
        };
        assert!(only_hallucination(&[w("Thank"), w("you.")]));
        assert!(only_hallucination(&[w("Thanks"), w("for"), w("watching!")]));
        assert!(!only_hallucination(&[w("Thank"), w("you"), w("for"), w("coming"), w("today")]));
        assert!(!only_hallucination(&[w("Click"), w("here")]));
        assert!(!only_hallucination(&[]));
    }

    #[test]
    fn words_squashed_at_the_end_are_spread_over_the_voice() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
        // What whisper tiny did on the CI recording: "Press" stretched over four
        // seconds, the rest of the sentence piled up at the end of the video.
        let mut words = vec![w(300.0, 900.0, "Let's"), w(7050.0, 11000.0, "Press"), w(12010.0, 12010.0, "Ctrl+S"), w(12010.0, 12010.0, "to"), w(12010.0, 12010.0, "save"), w(12010.0, 12010.0, "it,"),
        ];
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
    fn a_pile_up_at_the_end_is_not_mistaken_for_a_hole_once_it_is_spread() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
        let voiced = [(0.0, 5400.0), (6750.0, 11370.0)];
        let mut words = vec![w(300.0, 900.0, "Let's"), w(900.0, 1700.0, "open"), w(1700.0, 2500.0, "the"), w(2500.0, 3300.0, "invoices"), w(3300.0, 4300.0, "page"), w(4320.0, 7050.0, "March."), w(7050.0, 10970.0, "Press"), w(10970.0, 11360.0, "Ctrl+S"),
        ];
        for t in ["to", "save", "it,", "and", "now", "it", "is"] {
            words.push(w(11360.0, 11360.0, t));
        }
        assert!(!uncovered(&words, &voiced, 1_500.0).is_empty(), "before spreading it looks like a hole");
        retime_implausible(&mut words, &voiced);
        assert!(uncovered(&words, &voiced, 1_500.0).is_empty(), "{words:?}");
    }

    #[test]
    fn words_with_unbelievable_times_are_laid_over_the_voice() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
        // What whisper tiny gave for eight seconds of fast speech: a few held
        // words, then everything else squashed at the end.
        let mut ws = vec![w(70.0, 2030.0, "So,"), w(2030.0, 4480.0, "let's"), w(4480.0, 5460.0, "go"),
        ];
        for t in ["back", "to", "the", "dashboard", "and"] {
            ws.push(w(8850.0, 8850.0, t));
        }
        assert!(respread_if_implausible(&mut ws, &[(0.0, 8860.0)], 0.0, 8860.0));
        assert!(ws.windows(2).all(|p| p[0].end_ms <= p[1].start_ms + 1.0), "{ws:?}");
        assert!(ws[3].start_ms < 6000.0 && ws[7].end_ms <= 8860.0 && ws.iter().all(|w| w.end_ms - w.start_ms < 2500.0), "{ws:?}");
        let mut fine = vec![w(0.0, 300.0, "one"), w(300.0, 700.0, "two"), w(700.0, 900.0, "three"),
        ];
        assert!(!respread_if_implausible(&mut fine, &[(0.0, 900.0)], 0.0, 900.0), "good times stay");
        assert_eq!(fine[1].start_ms, 300.0);
    }

    #[test]
    fn long_holes_are_heard_in_short_pieces() {
        assert_eq!(split_hole((0.0, 5000.0), 7000.0), vec![(0.0, 5000.0)]);
        let p = split_hole((1000.0, 11000.0), 7000.0);
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|(a, b)| b - a <= 7000.0) && p[0].1 == p[1].0 && p[1].1 == 11000.0);
    }

    #[test]
    fn ci_recording_with_the_whole_tail_collapsed() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
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
    fn a_lone_held_word_and_a_second_stretch_before_the_collapse() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
        // From the second CI run: "March." held to the next sentence, "Press"
        // held for four seconds, then one normal word, then the pile-up.
        let mut words = vec![w(4320.0, 7050.0, "March."), w(7050.0, 10970.0, "Press"), w(10970.0, 11360.0, "Ctrl+S"),
        ];
        for t in ["to", "save", "it,", "and", "now", "it", "is"] {
            words.push(w(11360.0, 11360.0, t));
        }
        words.push(w(11360.0, 11370.0, "saved."));
        retime_implausible(&mut words, &[(0.0, 5400.0), (6750.0, 11370.0)]);
        assert!(words[0].end_ms < 5300.0, "{words:?}");
        assert!(words[1].end_ms < 8300.0, "{words:?}");
        let save = &words[4];
        assert!(save.start_ms > 8000.0 && save.start_ms < 10000.0, "{words:?}");
        assert!(words.iter().all(|w| w.end_ms <= 11370.0 && w.start_ms < w.end_ms), "{words:?}");
        for p in words[1..].windows(2) {
            assert!(p[0].end_ms <= p[1].start_ms + 1.0, "{words:?}");
        }
    }

    #[test]
    fn two_stretched_words_in_a_row_are_not_one_run() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
        // Third CI run: "March." held, then "Press" held, then the pile-up.
        let mut words = vec![w(4320.0, 7050.0, "March."), w(7050.0, 11360.0, "Press")];
        for t in ["Ctrl+Lester", "save", "it,", "and", "now", "it", "is"] {
            words.push(w(11360.0, 11360.0, t));
        }
        words.push(w(11360.0, 11370.0, "saved."));
        retime_implausible(&mut words, &[(0.0, 5400.0), (6750.0, 11370.0)]);
        assert!(words[0].end_ms < 5300.0, "{words:?}");
        assert!(words[1].start_ms >= 7050.0 && words[1].end_ms < 8300.0, "Press stays where it started: {words:?}");
        assert!(words[3].start_ms > 8000.0, "{words:?}");
        assert!(words.iter().all(|w| w.end_ms <= 11370.0), "{words:?}");
    }

    #[test]
    fn normal_timings_are_not_touched() {
        let w = |a: f64, b: f64, t: &str| booya_core::project::CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        };
        let orig = vec![w(0.0, 400.0, "Open"), w(400.0, 700.0, "the"), w(700.0, 1500.0, "settings"),
        ];
        let mut words = orig.clone();
        retime_implausible(&mut words, &[(0.0, 1500.0)]);
        assert_eq!(words, orig);
    }

    #[test]
    fn vad_compacts_long_silence_and_maps_back() {
        let rate = 16_000;
        let tone = |n: usize| {
            (0..n).map(|i| (i as f32 * 0.05).sin() * 0.3).collect::<Vec<f32>>()
        };
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
        let w = |t: &str| CaptionWord { start_ms: 0.0, end_ms: 1.0, text: t.into(),
        };
        let out = merge_fragments(vec![w("Hello"), w(","), w("world"), w("."), w("It"), w("'s"),
        ]);
        let t: Vec<_> = out.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(t, vec!["Hello,", "world.", "It's"]);
    }

    // A stand-in for speech: a 180 Hz tone with a syllable rhythm.
    fn voice(ms: usize, amp: f32) -> Vec<f32> {
        (0..ms * 16).map(|i| {
                amp * (i as f32 * 2.0 * std::f32::consts::PI * 180.0 / 16_000.0).sin() * (0.6 + 0.4 * (i as f32 * 2.0 * std::f32::consts::PI * 4.0 / 16_000.0).sin().abs())
            }).collect()
    }
    fn hum(ms: usize, amp: f32) -> Vec<f32> {
        (0..ms * 16).map(|i| amp * (i as f32 * 0.37).sin()).collect()
    }
    fn word(t: &str, a: f64, b: f64) -> CaptionWord {
        CaptionWord { start_ms: a, end_ms: b, text: t.into(),
        }
    }

    #[test]
    fn captions_do_not_start_before_the_voice_after_two_seconds_of_silence() {
        for lead in [vec![0.0f32; 32_000], hum(2_000, 0.004)] {
            let mut audio = lead;
            audio.extend(voice(3_000, 0.3));
            let onsets = voice_onsets(&audio, 16_000);
            assert_eq!(onsets.len(), 1, "{onsets:?}");
            assert!((onsets[0].0 - 2_000.0).abs() < 100.0, "{onsets:?}");
            // Whisper puts the first words at 0, as it does for real recordings.
            let mut words = vec![word("Hello", 0.0, 400.0), word("and", 400.0, 600.0), word("welcome", 600.0, 1_300.0), word("back", 2_400.0, 2_800.0),
            ];
            anchor_to_voice(&mut words, &onsets);
            assert!(words[0].start_ms >= 1_900.0, "first word at {}", words[0].start_ms);
            assert!(words.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
            assert!(words.iter().all(|w| w.end_ms > w.start_ms && w.end_ms <= 5_200.0));
            // A word that was on the voice stays where it was.
            assert_eq!(words[3].start_ms, 2_400.0);
        }
    }

    #[test]
    fn good_timings_are_left_alone() {
        let mut audio = voice(1_000, 0.3);
        audio.extend(vec![0.0; 16_000]);
        audio.extend(voice(1_000, 0.3));
        let onsets = voice_onsets(&audio, 16_000);
        assert_eq!(onsets.len(), 2);
        let mut words = vec![word("one", 50.0, 500.0), word("two", 2_050.0, 2_500.0)];
        let before = words.clone();
        anchor_to_voice(&mut words, &onsets);
        assert_eq!(words, before);
    }

    #[test]
    fn fast_continuous_speech_is_never_cut_out_of_what_whisper_hears() {
        // Ten seconds of syllables with 40 ms dips and no real pause.
        let mut audio = Vec::new();
        for _ in 0..50 {
            audio.extend(voice(160, 0.3));
            audio.extend(hum(40, 0.002));
        }
        let spans = speech_spans(&audio, 16_000);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert!(spans[0].0 == 0 && spans[0].1 >= audio.len() - 16, "{spans:?}");
        // Everything above the room level lies inside a kept span.
        let (compacted, _) = compact(&audio, &spans, 16_000);
        assert_eq!(compacted.len(), audio.len());
        let onsets = voice_onsets(&audio, 16_000);
        assert_eq!(onsets.len(), 1);
        assert!(onsets[0].1 - onsets[0].0 > 9_800.0);
    }

    #[test]
    fn chunks_cover_all_audio_with_overlap() {
        let rate = 16_000;
        assert_eq!(plan_chunks(10 * rate, rate), vec![(0, 10 * rate)]);
        for len in [29 * rate, 60 * rate, 61 * rate, 250 * rate + 7] {
            let c = plan_chunks(len, rate);
            assert_eq!(c[0].0, 0);
            assert_eq!(c.last().unwrap().1, len);
            for w in c.windows(2) {
                assert_eq!(w[0].1 - w[1].0, OVERLAP_S * rate, "overlap");
            }
            assert!(c.iter().all(|&(a, b)| b - a <= CHUNK_S * rate && b - a >= OVERLAP_S * rate));
        }
    }

    #[test]
    fn overlapping_chunks_give_every_word_exactly_once_even_when_each_hears_a_different_half() {
        let rate = 16_000;
        // 50 s of speech, a word every 400 ms, three chunks.
        let all: Vec<(String, f64)> = (0..125).map(|i| (format!("w{i}"), i as f64 * 400.0)).collect();
        let chunks = plan_chunks(50 * rate, rate);
        assert_eq!(chunks.len(), 3);
        let to_ms = |s: usize| s as f64 * 1000.0 / rate as f64;
        let mut parts = Vec::new();
        for (k, &(a, b)) in chunks.iter().enumerate() {
            let mut ws = Vec::new();
            for (n, (t, at)) in all.iter().enumerate() {
                if *at >= to_ms(a) && *at + 300.0 <= to_ms(b) {
                    // Each chunk times the shared words a little differently and
                    // drops a few (the ones whisper skipped).
                    let in_overlap = chunks.windows(2).any(|w| *at >= to_ms(w[1].0) && *at + 300.0 <= to_ms(w[0].1));
                    if in_overlap && (n + k) % 3 == 0 && chunks.iter().all(|c| {
                            (*at - to_ms(c.0)).abs() > 1_000.0 && (*at - to_ms(c.1)).abs() > 1_000.0
                        }) {
                        continue;
                    }
                    let jitter = if k % 2 == 0 { 40.0 } else { -60.0 };
                    ws.push(word(t, at - to_ms(a) + jitter, at - to_ms(a) + 300.0));
                }
            }
            parts.push(ws);
        }
        let merged = merge_chunks(&chunks, parts, rate);
        let texts: Vec<&str> = merged.iter().map(|w| w.text.as_str()).collect();
        let mut seen = std::collections::HashSet::new();
        assert!(texts.iter().all(|t| seen.insert(*t)), "a word came twice: {texts:?}");
        assert!(merged.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
        // A word is lost only when every chunk that covered it dropped it, and
        // the drop pattern above never does that.
        let missing: Vec<usize> = (0..125).filter(|i| !texts.contains(&format!("w{i}").as_str())).collect();
        assert!(missing.is_empty(), "missing {missing:?} chunks {chunks:?}");
    }

    #[test]
    fn a_word_the_speaker_says_twice_is_kept_twice() {
        let chunks = vec![(0, 16_000 * 10)];
        let parts = vec![vec![word("very", 0.0, 300.0), word("very", 350.0, 650.0), word("fast", 700.0, 1_000.0),
        ]];
        assert_eq!(merge_chunks(&chunks, parts, 16_000).len(), 3);
    }

    #[test]
    fn voiced_stretches_with_no_words_are_found_so_they_can_be_heard_again() {
        let voiced = vec![(0.0, 10_000.0)];
        let words = vec![word("a", 100.0, 400.0), word("b", 500.0, 900.0), word("c", 5_200.0, 5_500.0),
        ];
        let holes = uncovered(&words, &voiced, 2_000.0);
        assert_eq!(holes, vec![(900.0, 5_200.0), (5_500.0, 10_000.0)]);
        assert!(uncovered(&words, &voiced, 5_000.0).is_empty());
    }

    #[test]
    fn fast_speech_in_many_short_runs_still_counts_as_one_hole() {
        // Voice on for 5.6 s in runs with short pauses, words only at the start.
        let voiced = vec![(0.0, 1_160.0), (1_320.0, 3_820.0), (4_100.0, 4_440.0), (4_720.0, 4_900.0), (5_080.0, 6_980.0), (7_160.0, 7_880.0),
        ];
        let words = vec![word("you", 1_320.0, 1_410.0), word("downloads", 2_460.0, 3_000.0),
        ];
        let holes = uncovered(&words, &voiced, 2_000.0);
        assert_eq!(holes, vec![(3_000.0, 7_880.0)]);
        // A word stretched over the dropped part does not hide it.
        let stretched = vec![word("you", 1_320.0, 7_800.0)];
        assert_eq!(uncovered(&stretched, &voiced, 2_000.0), vec![(2_320.0, 7_880.0)]);
        assert!(uncovered(&[], &[], 2_000.0).is_empty());
    }

    /// The voice of the 20 s clip a user sent: a pause at the start, three
    /// stretches of talking (about 6 s, 6 s and 3 s) and silence to the end.
    fn clip_voice() -> Vec<(f64, f64)> {
        vec![(1420.0, 2080.0), (2680.0, 7360.0), (8360.0, 10740.0), (10960.0, 14520.0), (15380.0, 15900.0), (16480.0, 17000.0),
        ]
    }

    #[test]
    fn a_sentence_whisper_stamped_after_the_voice_is_still_shown() {
        // What the small model gave for that clip: the middle sentence drifts
        // later and later, and the last dozen words are all stamped at the very
        // end of the file (20.27 s), where no one is talking.
        let spec: &[(&str, f64, f64)] = &[
            ("So", 170.0, 510.0), ("I'm", 510.0, 1420.0), ("just", 1420.0, 1700.0), ("trying", 1700.0, 2770.0), ("to", 2770.0, 3070.0), ("check", 3070.0, 3930.0),
            ("my", 3930.0, 4270.0), ("mic", 4270.0, 4780.0), ("and", 4780.0, 5290.0), ("this", 5290.0, 5970.0), ("is", 5970.0, 6310.0), ("how", 6310.0, 6820.0),
            ("we", 6820.0, 7160.0), ("can", 7160.0, 8350.0), ("join", 8350.0, 8360.0), ("the", 8360.0, 8970.0), ("permissions.", 8970.0, 11260.0),
            ("And", 11260.0, 11770.0), ("this", 11770.0, 12450.0), ("is", 12450.0, 12940.0), ("where", 12940.0, 13670.0), ("you", 13670.0, 14150.0),
            ("can", 14150.0, 15350.0), ("find", 15350.0, 15390.0), ("my", 15390.0, 15690.0), ("phone", 15690.0, 16550.0), ("and", 16550.0, 17740.0), ("then", 17740.0, 17740.0),
            ("if", 17740.0, 18080.0), ("you", 18080.0, 18630.0), ("click", 18630.0, 19450.0), ("on", 19450.0, 19790.0), ("others,", 19790.0, 20260.0),
            ("you", 20260.0, 20260.0), ("will", 20260.0, 20260.0), ("get", 20260.0, 20260.0), ("a", 20260.0, 20260.0), ("prompt", 20260.0, 20260.0), ("like", 20260.0, 20260.0),
            ("this.", 20260.0, 20260.0), ("Let's", 20260.0, 20260.0), ("see.", 20260.0, 20260.0), ("Thank", 20260.0, 20260.0), ("you.", 20260.0, 30000.0),
        ];
        let mut words: Vec<CaptionWord> = spec.iter().map(|&(t, a, b)| word(t, a, b)).collect();
        let voiced = clip_voice();
        fix_times(&mut words, &voiced, &voiced);
        let end = voiced.last().unwrap().1;
        for (i, w) in words.iter().enumerate() {
            assert!(w.start_ms <= end + 200.0, "{} starts at {} after the voice ended ({end})", w.text, w.start_ms);
            assert!(w.end_ms - w.start_ms >= 60.0, "{} has no time: {w:?}", w.text);
            assert!(i == 0 || w.start_ms >= words[i - 1].start_ms, "words stay in order at {}", w.text);
        }
        // Every sentence is on screen while it is said.
        let at = |t: &str| words.iter().find(|w| w.text == t).unwrap().start_ms;
        assert!(at("find") < 14_600.0 && at("find") > 8_300.0, "the middle sentence is where the voice is: {}", at("find"));
        assert!(at("Thank") > 15_300.0, "{}", at("Thank"));
        let caps = booya_core::captions::words_to_captions(&words, 42, 900.0);
        assert!(caps.iter().all(|c| c.end_ms - c.start_ms >= 500.0), "{:?}", caps.iter().map(|c| (c.start_ms, c.end_ms, &c.text)).collect::<Vec<_>>());
        // Nothing is on screen with no one speaking for long, and the 8 to 13 s stretch has captions.
        let covered = |t: f64| caps.iter().any(|c| t >= c.start_ms && t < c.end_ms);
        assert!((8_500..14_000).step_by(250).all(|t| covered(t as f64)), "a stretch of speech has no caption");
    }

    #[test]
    fn words_heard_again_for_a_hole_are_placed_inside_it_not_thrown_away() {
        // Whisper heard the hole again but stamped the words past its end.
        let heard: Vec<CaptionWord> = ["and", "this", "is", "where", "you", "can", "find"].iter().enumerate().map(|(i, t)| word(t, 14_000.0 + i as f64 * 20.0, 14_015.0 + i as f64 * 20.0)).collect();
        let voiced = clip_voice();
        let placed = place_heard_again(heard, &voiced, (8360.0, 10740.0), (8060.0, 11040.0));
        assert_eq!(placed.len(), 7, "no word is dropped");
        assert!(placed.iter().all(|w| w.start_ms >= 8_260.0 && w.start_ms < 10_740.0), "{:?}", placed.iter().map(|w| w.start_ms).collect::<Vec<_>>());
        assert!(placed.windows(2).all(|p| p[1].start_ms > p[0].start_ms));
    }

    #[test]
    fn believable_times_are_left_alone() {
        let voiced = clip_voice();
        let spec: &[(&str, f64, f64)] = &[("So", 1450.0, 1700.0), ("I'm", 1700.0, 2000.0), ("just", 2700.0, 3000.0), ("trying", 3000.0, 3400.0), ("to", 3400.0, 3500.0), ("check", 3500.0, 4000.0),
        ];
        let mut words: Vec<CaptionWord> = spec.iter().map(|&(t, a, b)| word(t, a, b)).collect();
        let before = words.clone();
        fix_times(&mut words, &voiced, &voiced);
        assert_eq!(words, before);
    }
}
