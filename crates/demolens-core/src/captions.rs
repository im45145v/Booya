//! Captions: group timed words into readable lines, export SRT / WebVTT
//! in output time (after trims, cuts and speed changes), and plan auto cuts
//! for silences and filler words.

use crate::project::{Caption, CaptionWord, Project};

/// Group words into captions of at most `max_chars`, breaking on pauses
/// longer than `max_gap_ms` and after sentence-ending punctuation.
pub fn words_to_captions(words: &[CaptionWord], max_chars: usize, max_gap_ms: f64) -> Vec<Caption> {
    let mut out: Vec<Caption> = Vec::new();
    let mut cur: Vec<CaptionWord> = Vec::new();
    let flush = |cur: &mut Vec<CaptionWord>, out: &mut Vec<Caption>| {
        if cur.is_empty() {
            return;
        }
        let text = cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        out.push(Caption { start_ms: cur[0].start_ms, end_ms: cur[cur.len() - 1].end_ms, text, words: std::mem::take(cur) });
    };
    for w in words {
        let t = w.text.trim();
        if t.is_empty() {
            continue;
        }
        let w = CaptionWord { start_ms: w.start_ms, end_ms: w.end_ms, text: t.split_whitespace().collect::<Vec<_>>().join("") };
        if let Some(last) = cur.last() {
            let len: usize = cur.iter().map(|w| w.text.len() + 1).sum::<usize>() + w.text.len();
            let ends_sentence = last.text.ends_with(['.', '?', '!']);
            if len > max_chars || w.start_ms - last.end_ms > max_gap_ms || ends_sentence {
                flush(&mut cur, &mut out);
            }
        }
        cur.push(w);
    }
    flush(&mut cur, &mut out);
    // Keep captions on screen a little longer, without overlapping the next.
    for i in 0..out.len() {
        let next = out.get(i + 1).map(|c| c.start_ms).unwrap_or(f64::MAX);
        out[i].end_ms = (out[i].end_ms + 400.0).min(next);
    }
    out
}

fn ts(ms: f64, sep: char) -> String {
    let ms = ms.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:02}{}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, sep, ms % 1000)
}

/// True when a word survives the project's cuts.
pub fn word_visible(p: &Project, w: &CaptionWord) -> bool {
    [w.start_ms + 40.0, (w.start_ms + w.end_ms) / 2.0, w.end_ms - 40.0].iter().any(|&t| p.output_time(t).is_some())
}

/// Captions mapped to output time; captions fully cut out are dropped.
pub fn output_captions(p: &Project) -> Vec<(f64, f64, String)> {
    let mut v = Vec::new();
    for c in &p.captions {
        // Use the visible words so cut filler words disappear from the text.
        let kept: Vec<&CaptionWord> = c.words.iter().filter(|w| word_visible(p, w)).collect();
        let text = if c.words.is_empty() { c.text.clone() } else { kept.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ") };
        if text.trim().is_empty() {
            continue;
        }
        let start = (0..=20).map(|i| c.start_ms + (c.end_ms - c.start_ms) * i as f64 / 20.0).find_map(|t| p.output_time(t));
        let end = (0..=20).rev().map(|i| c.start_ms + (c.end_ms - c.start_ms) * i as f64 / 20.0 - 0.5).find_map(|t| p.output_time(t));
        if let (Some(a), Some(b)) = (start, end) {
            if b > a {
                v.push((a, b, text));
            }
        }
    }
    v
}

pub fn to_srt(p: &Project) -> String {
    output_captions(p)
        .iter()
        .enumerate()
        .map(|(i, (a, b, t))| format!("{}\n{} --> {}\n{}\n", i + 1, ts(*a, ','), ts(*b, ','), t))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn to_vtt(p: &Project) -> String {
    let mut s = String::from("WEBVTT\n\n");
    for (a, b, t) in output_captions(p) {
        s.push_str(&format!("{} --> {}\n{}\n\n", ts(a, '.'), ts(b, '.'), t));
    }
    s
}

const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "erm", "er", "ah", "hmm", "mm"];

pub fn is_filler(word: &str) -> bool {
    let w: String = word.chars().filter(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
    FILLERS.contains(&w.as_str())
}

#[derive(Debug, Clone)]
pub struct AutoCutOptions {
    pub cut_fillers: bool,
    pub cut_silences: bool,
    /// Silences longer than this are shortened.
    pub min_silence_ms: f64,
    /// Silence kept on each side of a cut so speech does not sound clipped.
    pub keep_ms: f64,
}

impl Default for AutoCutOptions {
    fn default() -> Self {
        Self { cut_fillers: true, cut_silences: true, min_silence_ms: 700.0, keep_ms: 180.0 }
    }
}

/// Whisper word timestamps are coarse and often stretch across pauses.
/// Pull word edges out of detected silences so captions line up with the
/// voice and cutting a pause never swallows a word.
pub fn snap_to_speech(words: &mut [CaptionWord], silences: &[(f64, f64)]) {
    for i in 0..words.len() {
        let next = words.get(i + 1).map(|n| n.end_ms).unwrap_or(f64::MAX);
        let w = &mut words[i];
        for &(a, b) in silences {
            if w.start_ms >= a && w.end_ms <= b {
                // The whole word sits in a pause: whisper placed it too
                // early, so move it to where the voice starts again.
                let dur = w.end_ms - w.start_ms;
                w.start_ms = b;
                w.end_ms = (b + dur).min(next).max(b + 1.0);
                continue;
            }
            if w.start_ms >= a && w.start_ms < b && b < w.end_ms {
                w.start_ms = b;
            }
            if w.end_ms > a && w.end_ms <= b && a > w.start_ms {
                w.end_ms = a;
            }
        }
    }
}

/// Source ranges to remove. `silences` comes from `audio::detect_silences`.
pub fn plan_cuts(words: &[CaptionWord], silences: &[(f64, f64)], o: &AutoCutOptions) -> Vec<(f64, f64)> {
    let mut r: Vec<(f64, f64)> = Vec::new();
    if o.cut_fillers {
        for w in words.iter().filter(|w| is_filler(&w.text)) {
            r.push((w.start_ms - 40.0, w.end_ms + 40.0));
        }
    }
    if o.cut_silences {
        for &(a, b) in silences {
            if b - a >= o.min_silence_ms {
                r.push((a + o.keep_ms, b - o.keep_ms));
            }
        }
    }
    r.retain(|(a, b)| b > a);
    r.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in r {
        match merged.last_mut() {
            Some(m) if a <= m.1 + 60.0 => m.1 = m.1.max(b),
            _ => merged.push((a.max(0.0), b)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_snap_out_of_silence() {
        let mut ws = vec![w(5000.0, 6000.0, "march."), w(6000.0, 6580.0, "Press"), w(10660.0, 12000.0, "saved.")];
        snap_to_speech(&mut ws, &[(5280.0, 6860.0), (11240.0, 11830.0)]);
        assert_eq!((ws[0].start_ms, ws[0].end_ms), (5000.0, 5280.0));
        // A word placed entirely inside a pause moves to the end of it.
        assert_eq!((ws[1].start_ms, ws[1].end_ms), (6860.0, 7440.0));
        assert_eq!(ws[2].end_ms, 12000.0);
    }

    fn w(a: f64, b: f64, t: &str) -> CaptionWord {
        CaptionWord { start_ms: a, end_ms: b, text: t.into() }
    }

    #[test]
    fn groups_words_into_lines() {
        let words = vec![w(0.0, 300.0, "Hello"), w(300.0, 600.0, "there."), w(700.0, 900.0, "Click"), w(900.0, 1200.0, "save"), w(3000.0, 3300.0, "Done")];
        let c = words_to_captions(&words, 42, 700.0);
        let texts: Vec<_> = c.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello there.", "Click save", "Done"]);
        assert!(c[0].end_ms <= c[1].start_ms);
    }

    #[test]
    fn srt_uses_output_time_and_drops_cut_words() {
        let mut p = Project::new("a.mp4".into(), None, 10, 10, 10_000.0);
        p.captions = words_to_captions(&[w(1000.0, 1400.0, "So"), w(1500.0, 1900.0, "um"), w(2000.0, 2400.0, "click")], 42, 700.0);
        p.cut_ranges(&[(1460.0, 1940.0)]);
        let srt = to_srt(&p);
        assert!(srt.starts_with("1\n00:00:01,000 --> "), "{srt}");
        assert!(srt.contains("So click"));
        assert!(!srt.contains("um"));
        assert!(to_vtt(&p).starts_with("WEBVTT"));
    }

    #[test]
    fn fillers_and_silences_become_cuts() {
        assert!(is_filler("Um,") && is_filler("uh") && !is_filler("umbrella"));
        let words = vec![w(0.0, 400.0, "Okay"), w(500.0, 800.0, "um"), w(900.0, 1200.0, "next")];
        let cuts = plan_cuts(&words, &[(2000.0, 4000.0), (5000.0, 5300.0)], &AutoCutOptions::default());
        assert_eq!(cuts, vec![(460.0, 840.0), (2180.0, 3820.0)]);
    }
}
