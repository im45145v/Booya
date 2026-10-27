//! Auto blur for sensitive info. Frames are sampled, text is found with a
//! local OCR model (ocrs), matched against detectors for emails, API keys,
//! card numbers and phone numbers, and turned into blur boxes that are
//! merged over time so a value stays hidden while it is on screen.

use anyhow::Result;
use demolens_core::project::{BlurBox, Project};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sensitive {
    Email,
    ApiKey,
    Card,
    Phone,
}

fn res() -> &'static [(Sensitive, Regex)] {
    static R: OnceLock<Vec<(Sensitive, Regex)>> = OnceLock::new();
    R.get_or_init(|| {
        vec![
            (Sensitive::Email, Regex::new(r"(?i)[a-z0-9._%+\-]+\s?@\s?[a-z0-9\-]+(\.[a-z0-9\-]+)*\.[a-z]{2,}").unwrap()),
            (
                Sensitive::ApiKey,
                Regex::new(r"\b((sk|pk|rk)[-_ ](live|test|proj)[-_ ]?[A-Za-z0-9][^\s]{11,}|(sk|pk|rk)[-_][A-Za-z0-9]{16,}|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z\-_]{30,}|[A-Fa-f0-9]{32,}|eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,})").unwrap(),
            ),
            (Sensitive::Card, Regex::new(r"\b(?:\d[ -]?){12,18}\d\b").unwrap()),
            (Sensitive::Phone, Regex::new(r"(?:\+\d{1,3}[\s.-]?)?(?:\(\d{2,4}\)[\s.-]?)?\d{3,4}[\s.-]?\d{3,4}[\s.-]?\d{0,4}").unwrap()),
        ]
    })
}

fn luhn(digits: &str) -> bool {
    let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() < 13 || d.len() > 19 {
        return false;
    }
    let sum: u32 = d.iter().rev().enumerate().map(|(i, &x)| if i % 2 == 1 { let y = x * 2; if y > 9 { y - 9 } else { y } } else { x }).sum();
    sum % 10 == 0
}

/// Find sensitive spans in a line of text. Returns (kind, byte range).
pub fn find_sensitive(text: &str) -> Vec<(Sensitive, std::ops::Range<usize>)> {
    let mut out: Vec<(Sensitive, std::ops::Range<usize>)> = Vec::new();
    // Long digit groups that look like card numbers but fail the Luhn check
    // (order ids, invoice numbers) should not be reported as phone numbers.
    let card_like: Vec<std::ops::Range<usize>> = res()[2].1.find_iter(text).map(|m| m.range()).collect();
    for (kind, re) in res() {
        for m in re.find_iter(text) {
            if out.iter().any(|(_, r)| r.start < m.end() && m.start() < r.end) {
                continue;
            }
            let s = m.as_str();
            let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
            let ok = match kind {
                Sensitive::Card => luhn(s),
                Sensitive::Phone => (10..=15).contains(&digits) && !luhn_like_date(s) && !card_like.iter().any(|r| r.start < m.end() && m.start() < r.end),
                _ => true,
            };
            if ok {
                out.push((*kind, m.range()));
            }
        }
    }
    out.sort_by_key(|(_, r)| r.start);
    out
}

fn luhn_like_date(s: &str) -> bool {
    // Avoid blurring things like "2026-10-27 12:30".
    Regex::new(r"^\d{4}[-/.]\d{2}[-/.]\d{2}").unwrap().is_match(s.trim())
}

/// A sensitive region found in one sampled frame (source pixels).
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub t_ms: f64,
    pub kind: Sensitive,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// OCR misses text on some frames; detections of the same region this close
/// together are treated as one continuous appearance.
pub const MAX_GAP_MS: f64 = 2500.0;

/// Merge per-frame hits into blur boxes that span time. Hits whose boxes
/// overlap in consecutive samples become one box covering both.
pub fn hits_to_blur(hits: &[Hit], sample_ms: f64, pad: f64) -> Vec<BlurBox> {
    let mut boxes: Vec<BlurBox> = Vec::new();
    let mut sorted = hits.to_vec();
    sorted.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));
    for h in sorted {
        let (x, y, w, hh) = (h.x - pad, h.y - pad, h.w + 2.0 * pad, h.h + 2.0 * pad);
        let found = boxes.iter_mut().find(|b| {
            b.end_ms + (sample_ms * 1.5).max(MAX_GAP_MS) >= h.t_ms && x < b.x + b.w && b.x < x + w && y < b.y + b.h && b.y < y + hh
        });
        match found {
            Some(b) => {
                let (x1, y1) = ((b.x + b.w).max(x + w), (b.y + b.h).max(y + hh));
                b.x = b.x.min(x);
                b.y = b.y.min(y);
                b.w = x1 - b.x;
                b.h = y1 - b.y;
                b.end_ms = b.end_ms.max(h.t_ms + sample_ms / 2.0);
            }
            None => boxes.push(BlurBox {
                x,
                y,
                w,
                h: hh,
                start_ms: (h.t_ms - sample_ms / 2.0).max(0.0),
                end_ms: h.t_ms + sample_ms / 2.0,
                radius: ((hh / 2.0) as u32).clamp(8, 40),
            }),
        }
    }
    boxes
}

#[cfg(feature = "ocr")]
pub fn scan(project: &Project, sample_ms: f64, mut progress: impl FnMut(f64)) -> Result<Vec<Hit>> {
    use crate::models::require;
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
    use rten::Model;

    let det = Model::load_file(require("ocr-detection")?)?;
    let rec = Model::load_file(require("ocr-recognition")?)?;
    let engine = OcrEngine::new(OcrEngineParams { detection_model: Some(det), recognition_model: Some(rec), ..Default::default() })?;
    let fps = 1000.0 / sample_ms;
    let (w, h) = (project.source_width, project.source_height);
    let mut reader = demolens_core::ffmpeg::FrameReader::open(&project.source, 0.0, fps, w, h)?;
    let total = (project.duration_ms / sample_ms).ceil().max(1.0);
    let mut hits = Vec::new();
    let mut i = 0u64;
    while let Some(frame) = reader.next()? {
        let t = i as f64 * sample_ms;
        i += 1;
        // Skip frames that are entirely cut from the output.
        if project.output_time(t).is_none() {
            continue;
        }
        let rgb: Vec<u8> = frame.data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let img = ImageSource::from_bytes(&rgb, (w, h))?;
        let input = engine.prepare_input(img)?;
        let words = engine.detect_words(&input)?;
        let lines = engine.find_text_lines(&input, &words);
        let texts = engine.recognize_text(&input, &lines)?;
        for line in texts.into_iter().flatten() {
            // Build the line string while remembering each char's box.
            let mut s = String::new();
            let mut rects: Vec<(usize, [f64; 4])> = Vec::new();
            for c in line.chars().iter() {
                let r = c.rect;
                rects.push((s.len(), [r.left() as f64, r.top() as f64, r.right() as f64, r.bottom() as f64]));
                s.push(c.char);
            }
            if std::env::var_os("DEMOLENS_DEBUG_OCR").is_some() {
                eprintln!("ocr {t:.0}ms: {s}");
            }
            for (kind, range) in find_sensitive(&s) {
                let sel: Vec<&[f64; 4]> = rects.iter().filter(|(o, _)| range.contains(o)).map(|(_, r)| r).collect();
                if sel.is_empty() {
                    continue;
                }
                let x0 = sel.iter().map(|r| r[0]).fold(f64::MAX, f64::min);
                let y0 = sel.iter().map(|r| r[1]).fold(f64::MAX, f64::min);
                let x1 = sel.iter().map(|r| r[2]).fold(f64::MIN, f64::max);
                let y1 = sel.iter().map(|r| r[3]).fold(f64::MIN, f64::max);
                hits.push(Hit { t_ms: t, kind, text: s[range].to_string(), x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
            }
        }
        progress((i as f64 / total).min(1.0));
    }
    Ok(hits)
}

#[cfg(not(feature = "ocr"))]
pub fn scan(_project: &Project, _sample_ms: f64, _progress: impl FnMut(f64)) -> Result<Vec<Hit>> {
    anyhow::bail!("this build of DemoLens has no OCR (built without the 'ocr' feature)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<Sensitive> {
        find_sensitive(s).into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn detects_common_secrets() {
        assert_eq!(kinds("Contact jane.doe@acme.com today"), vec![Sensitive::Email]);
        assert_eq!(kinds("key sk_live_51HxQ2aBcDeFgHiJkLmN"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("AKIAIOSFODNN7EXAMPLE"), vec![Sensitive::ApiKey]);
        // OCR often reads "_" as a space.
        assert_eq!(kinds("APl key sk live 51HxQ2aBcDeFgHijkLmN"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("APl key sk live 51H?Q2aBcDeFgHijkLmN"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("token ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ012345"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("Card 4242 4242 4242 4242 exp"), vec![Sensitive::Card]);
        assert_eq!(kinds("Call +1 (415) 555-0132 now"), vec![Sensitive::Phone]);
    }

    #[test]
    fn ignores_harmless_text() {
        assert!(kinds("Invoice March total 120.50").is_empty());
        assert!(kinds("Card 1234 5678 9012 3456").is_empty()); // fails Luhn
        assert!(kinds("2026-10-27 12:30").is_empty());
    }

    #[test]
    fn hits_merge_over_time() {
        let h = |t: f64, x: f64| Hit { t_ms: t, kind: Sensitive::Email, text: "a@b.co".into(), x, y: 100.0, w: 200.0, h: 20.0 };
        let boxes = hits_to_blur(&[h(0.0, 100.0), h(500.0, 104.0), h(1000.0, 108.0), h(5000.0, 900.0), h(9000.0, 100.0)], 500.0, 4.0);
        assert_eq!(boxes.len(), 3);
        assert_eq!(boxes[0].start_ms, 0.0);
        assert_eq!(boxes[0].end_ms, 1250.0);
        assert!(boxes[0].w >= 216.0);
    }
}
