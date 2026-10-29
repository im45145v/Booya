//! Auto blur for sensitive info. Frames are sampled, text is found with a
//! local OCR model (ocrs), matched against detectors for emails, API keys,
//! cards, phone numbers, Aadhaar, PAN, UPI IDs, IBANs, IP addresses and
//! bearer tokens, plus the field next to a "Password" label, and turned into
//! blur boxes that are merged over time so a value stays hidden while it is
//! on screen. This is an aid, not a guarantee: OCR can miss small or low
//! contrast text, so the app asks the user to review before exporting.

use anyhow::Result;
use booya_core::project::{BlurBox, Project};
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
    /// Indian Aadhaar number (12 digits, Verhoeff check digit).
    Aadhaar,
    /// Indian PAN (ABCDE1234F).
    Pan,
    /// UPI ID (name@bank).
    Upi,
    Iban,
    Ip,
    /// "Bearer ..." or "password: ..." style secrets.
    Token,
    /// The field next to a "Password" label.
    Password,
}

/// Verhoeff checksum, used by Aadhaar.
fn verhoeff(digits: &str) -> bool {
    const D: [[u8; 10]; 10] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
        [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
        [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
        [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
        [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
        [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
        [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
        [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
        [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
    ];
    const P: [[u8; 10]; 8] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
        [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
        [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
        [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
        [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
        [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
        [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
    ];
    let d: Vec<u8> = digits.chars().filter_map(|c| c.to_digit(10).map(|v| v as u8)).collect();
    let mut c = 0u8;
    for (i, &x) in d.iter().rev().enumerate() {
        c = D[c as usize][P[i % 8][x as usize] as usize];
    }
    c == 0
}

fn iban_ok(s: &str) -> bool {
    let t: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    if !(15..=34).contains(&t.len()) {
        return false;
    }
    let r = format!("{}{}", &t[4..], &t[..4]);
    let mut m = 0u32;
    for c in r.chars() {
        let v = if c.is_ascii_digit() { c.to_digit(10).unwrap() } else { c as u32 - 'A' as u32 + 10 };
        for ch in v.to_string().chars() {
            m = (m * 10 + ch.to_digit(10).unwrap()) % 97;
        }
    }
    m == 1
}

fn res() -> &'static [(Sensitive, Regex)] {
    static R: OnceLock<Vec<(Sensitive, Regex)>> = OnceLock::new();
    R.get_or_init(|| {
        vec![
            (Sensitive::Token, Regex::new(r"(?i)\b(bearer\s+[A-Za-z0-9\-._~+/]{16,}=*|(?:password|passwd|pwd|secret|api[_ ]?key|token)\s*[:=]\s*\S{4,})").unwrap()),
            (Sensitive::Aadhaar, Regex::new(r"\b[2-9]\d{3}[ -]?\d{4}[ -]?\d{4}\b").unwrap()),
            (Sensitive::Pan, Regex::new(r"\b[A-Z]{3}[PCHFATBLJG][A-Z]\d{4}[A-Z]\b").unwrap()),
            (Sensitive::Iban, Regex::new(r"\b[A-Z]{2}\d{2}(?:[ ]?[A-Z0-9]{4}){2,7}(?:[ ]?[A-Z0-9]{1,4})?\b").unwrap()),
            (Sensitive::Email, Regex::new(r"(?i)[a-z0-9._%+\-]+\s?@\s?[a-z0-9\-]+(\.[a-z0-9\-]+)*\.[a-z]{2,}").unwrap()),
            (
                Sensitive::ApiKey,
                Regex::new(r"\b((?i:sk|pk|rk)[-_ ](?i:live|test|proj)[-_ ]?[A-Za-z0-9][^\s]{11,}|(sk|pk|rk)[-_][A-Za-z0-9]{16,}|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z\-_]{30,}|[A-Fa-f0-9]{32,}|eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,})").unwrap(),
            ),
            (Sensitive::Upi, Regex::new(r"(?i)\b[a-z0-9][a-z0-9.\-_]{1,255}@(?:ok)?[a-z]{2,64}\b").unwrap()),
            (Sensitive::Ip, Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap()),
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
    let card_re = &res().iter().find(|(k, _)| *k == Sensitive::Card).unwrap().1;
    let card_like: Vec<std::ops::Range<usize>> = card_re.find_iter(text).map(|m| m.range()).collect();
    for (kind, re) in res() {
        for m in re.find_iter(text) {
            if out.iter().any(|(_, r)| r.start < m.end() && m.start() < r.end) {
                continue;
            }
            let s = m.as_str();
            let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
            let ok = match kind {
                Sensitive::Card => luhn(s),
                Sensitive::Aadhaar => verhoeff(s),
                Sensitive::Iban => iban_ok(s),
                // An email (with a dotted domain) is reported as an email.
                Sensitive::Upi => !s.split('@').nth(1).unwrap_or("").contains('.'),
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
    let mut reader = booya_core::ffmpeg::FrameReader::open(&project.source, 0.0, fps, w, h)?;
    let total = (project.duration_ms / sample_ms).ceil().max(1.0);
    let mut hits = Vec::new();
    let mut i = 0u64;
    // OCR only runs when the picture changed; otherwise the last frame's
    // findings are carried forward. Static stretches cost almost nothing.
    let mut last_sig: Option<Vec<u8>> = None;
    let mut last_hits: Vec<Hit> = Vec::new();
    while let Some(frame) = reader.next()? {
        let t = i as f64 * sample_ms;
        i += 1;
        // Skip frames that are entirely cut from the output.
        if project.output_time(t).is_none() {
            continue;
        }
        let sig = signature(&frame.data, w as usize, h as usize);
        if last_sig.as_ref().is_some_and(|l| !changed(l, &sig)) {
            hits.extend(last_hits.iter().map(|x| Hit { t_ms: t, ..x.clone() }));
            progress((i as f64 / total).min(1.0));
            continue;
        }
        last_sig = Some(sig);
        let first_new = hits.len();
        let rgb: Vec<u8> = frame.data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let img = ImageSource::from_bytes(&rgb, (w, h))?;
        let input = engine.prepare_input(img)?;
        let words = engine.detect_words(&input)?;
        let lines = engine.find_text_lines(&input, &words);
        let texts = engine.recognize_text(&input, &lines)?;
        let mut line_boxes: Vec<(String, [f64; 4])> = Vec::new();
        for line in texts.into_iter().flatten() {
            // Build the line string while remembering each char's box.
            let mut s = String::new();
            let mut rects: Vec<(usize, [f64; 4])> = Vec::new();
            for c in line.chars().iter() {
                let r = c.rect;
                rects.push((s.len(), [r.left() as f64, r.top() as f64, r.right() as f64, r.bottom() as f64]));
                s.push(c.char);
            }
            if let (Some(a), Some(b)) = (rects.first(), rects.last()) {
                line_boxes.push((s.clone(), [a.1[0], a.1[1].min(b.1[1]), b.1[2], a.1[3].max(b.1[3])]));
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
        for b in password_fields(&line_boxes) {
            hits.push(Hit { t_ms: t, kind: Sensitive::Password, text: String::new(), x: b[0], y: b[1], w: b[2] - b[0], h: b[3] - b[1] });
        }
        last_hits = hits[first_new..].to_vec();
        progress((i as f64 / total).min(1.0));
    }
    Ok(hits)
}

/// 48x27 grey thumbnail of an RGBA frame, for change detection.
pub fn signature(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (sw, sh) = (48usize, 27usize);
    let mut out = Vec::with_capacity(sw * sh);
    for j in 0..sh {
        for i in 0..sw {
            let (x, y) = (i * w / sw + w / sw / 2, j * h / sh + h / sh / 2);
            let k = (y.min(h - 1) * w + x.min(w - 1)) * 4;
            let p = rgba.get(k..k + 3).unwrap_or(&[0, 0, 0]);
            out.push(((p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10) as u8);
        }
    }
    out
}

/// True when two thumbnails differ enough to be worth another OCR pass.
pub fn changed(a: &[u8], b: &[u8]) -> bool {
    a.len() != b.len() || a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 6).count() > 0
}

/// Boxes for the text field next to (right of, or under) a "Password"
/// label. Input boxes show dots that OCR reads as anything, so the field
/// is blurred whatever it contains.
pub fn password_fields(lines: &[(String, [f64; 4])]) -> Vec<[f64; 4]> {
    let mut out = Vec::new();
    for (text, b) in lines {
        let lower = text.to_lowercase();
        let Some(pos) = lower.find("password").or_else(|| lower.find("passcode")).or_else(|| lower.find("pin")) else { continue };
        if lower.contains("forgot") || lower.contains("reset") || (lower.starts_with("pin") && lower.len() > 12) {
            continue;
        }
        let h = (b[3] - b[1]).max(8.0);
        let label_end = b[0] + (b[2] - b[0]) * ((pos + 8).min(text.len()) as f64 / text.len().max(1) as f64);
        // Text after the label on the same line ("Password: ****").
        if text.len() > pos + 10 {
            out.push([label_end, b[1] - h * 0.3, b[2] + h, b[3] + h * 0.3]);
        }
        // The input box usually sits under the label.
        out.push([b[0] - h * 0.5, b[3], b[0] + (h * 18.0).max(b[2] - b[0]), b[3] + h * 2.6]);
    }
    out
}

/// Read text from one frame. Used by the self test.
#[cfg(feature = "ocr")]
pub fn read_text(frame: &booya_core::render::Frame) -> Result<String> {
    use crate::models::require;
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
    use rten::Model;
    let det = Model::load_file(require("ocr-detection")?)?;
    let rec = Model::load_file(require("ocr-recognition")?)?;
    let engine = OcrEngine::new(OcrEngineParams { detection_model: Some(det), recognition_model: Some(rec), ..Default::default() })?;
    let rgb: Vec<u8> = frame.data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let img = ImageSource::from_bytes(&rgb, (frame.w, frame.h))?;
    let input = engine.prepare_input(img)?;
    Ok(engine.get_text(&input)?)
}

#[cfg(not(feature = "ocr"))]
pub fn read_text(_frame: &booya_core::render::Frame) -> Result<String> {
    anyhow::bail!("this build of Booya has no OCR (built without the 'ocr' feature)")
}

/// Draw a known line of text, read it back and check the secret finder sees
/// an email address in it.
pub fn self_test() -> Result<String> {
    let t0 = std::time::Instant::now();
    let mut f = booya_core::render::Frame::new(900, 160);
    for px in f.data.chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 255, 255, 255]);
    }
    booya_core::text::draw(&mut f, 30.0, 50.0, 44.0, "contact test@example.com", [0.0, 0.0, 0.0], 1.0);
    let text = read_text(&f)?;
    let found = find_sensitive(&text).into_iter().any(|(k, _)| k == Sensitive::Email);
    if !found {
        anyhow::bail!("OCR ran but did not read the test text (it read: {:?})", text.trim());
    }
    Ok(format!("read a test screen and found the email address in {:.1} s", t0.elapsed().as_secs_f64()))
}

#[cfg(not(feature = "ocr"))]
pub fn scan(_project: &Project, _sample_ms: f64, _progress: impl FnMut(f64)) -> Result<Vec<Hit>> {
    anyhow::bail!("this build of Booya has no OCR (built without the 'ocr' feature)")
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
        // OCR often reads underscores as spaces and changes case.
        assert_eq!(kinds("SK Live 51Hg8ZtLm3?9VbOe7RKP2"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("AKIAIOSFODNN7EXAMPLE"), vec![Sensitive::ApiKey]);
        // OCR often reads "_" as a space.
        assert_eq!(kinds("APl key sk live 51HxQ2aBcDeFgHijkLmN"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("APl key sk live 51H?Q2aBcDeFgHijkLmN"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("token ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ012345"), vec![Sensitive::ApiKey]);
        assert_eq!(kinds("Card 4242 4242 4242 4242 exp"), vec![Sensitive::Card]);
        assert_eq!(kinds("Call +1 (415) 555-0132 now"), vec![Sensitive::Phone]);
    }

    #[test]
    fn detects_indian_ids_and_more() {
        // 2341 2341 2346 has a valid Verhoeff check digit.
        assert_eq!(kinds("Aadhaar 2341 2341 2346"), vec![Sensitive::Aadhaar]);
        assert_ne!(kinds("Aadhaar 2341 2341 2345"), vec![Sensitive::Aadhaar]);
        assert_eq!(kinds("PAN ABCPE1234F"), vec![Sensitive::Pan]);
        assert_eq!(kinds("Pay ashish.malla@okhdfcbank"), vec![Sensitive::Upi]);
        assert_eq!(kinds("Pay ravi@ybl"), vec![Sensitive::Upi]);
        assert_eq!(kinds("mail ravi@example.com"), vec![Sensitive::Email]);
        assert_eq!(kinds("IBAN GB82 WEST 1234 5698 7654 32"), vec![Sensitive::Iban]);
        assert_eq!(kinds("server 192.168.10.24 is up"), vec![Sensitive::Ip]);
        assert_eq!(kinds("Authorization: Bearer abcdefghijklmnop1234"), vec![Sensitive::Token]);
        assert_eq!(kinds("password: hunter22"), vec![Sensitive::Token]);
        assert!(kinds("Version 1.2.3 released").is_empty());
    }

    #[test]
    fn ignores_harmless_text() {
        assert!(kinds("Invoice March total 120.50").is_empty());
        assert!(kinds("Card 1234 5678 9012 3456").is_empty()); // fails Luhn
        assert!(kinds("2026-10-27 12:30").is_empty());
    }

    #[test]
    fn password_labels_blur_the_field() {
        let f = password_fields(&[("Password".into(), [100.0, 200.0, 180.0, 216.0]), ("Name".into(), [100.0, 100.0, 140.0, 116.0])]);
        assert_eq!(f.len(), 1);
        assert!(f[0][1] >= 216.0 && f[0][3] > 240.0);
        assert!(password_fields(&[("Forgot password?".into(), [0.0, 0.0, 10.0, 10.0])]).is_empty());
        assert!(!changed(&[10, 10], &[12, 14]) && changed(&[10, 10], &[30, 10]));
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
