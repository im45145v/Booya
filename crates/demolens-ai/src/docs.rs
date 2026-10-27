//! Turn a recording into written material: a title, YouTube style chapters,
//! a step by step guide and a short social post. The structure comes from
//! the recording itself (zoom segments, clicks, typed text and the
//! transcript). A local LLM polishes the wording when one is available,
//! otherwise a rule based writer is used, so this always works offline.

use crate::llm::Llm;
use anyhow::Result;
use demolens_core::project::Project;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    /// Output time in ms.
    pub at_ms: f64,
    pub title: String,
    /// What was said during the step.
    pub said: String,
    /// What happened (clicks, typing).
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoDocs {
    pub title: String,
    pub summary: String,
    pub steps: Vec<Step>,
    pub social_post: String,
    pub generated_by: String,
}

pub fn fmt_ts(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

pub(crate) fn transcript_between(p: &Project, a: f64, b: f64) -> String {
    let mut out = Vec::new();
    for c in &p.captions {
        for w in &c.words {
            if w.start_ms >= a && w.start_ms < b && p.output_time(w.start_ms).is_some() && !demolens_core::captions::is_filler(&w.text) {
                out.push(w.text.clone());
            }
        }
        if c.words.is_empty() && c.start_ms >= a && c.start_ms < b {
            out.push(c.text.clone());
        }
    }
    out.join(" ")
}

/// Split the recording into steps from its activity, in source time.
pub fn extract_steps(p: &Project, log: Option<&demolens_core::events::EventLog>) -> Vec<Step> {
    let mut bounds: Vec<f64> = vec![0.0];
    let mut segs = p.zoom.clone();
    segs.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    for z in &segs {
        if z.start_ms - bounds.last().unwrap() > 1500.0 {
            bounds.push(z.start_ms);
        }
    }
    // Long gaps between captions also start a new step when there is no zoom data.
    if segs.is_empty() {
        for w in p.captions.windows(2) {
            if w[1].start_ms - w[0].end_ms > 1200.0 {
                bounds.push(w[1].start_ms);
            }
        }
    }
    bounds.push(p.duration_ms);
    let mut steps = Vec::new();
    for w in bounds.windows(2) {
        let (a, b) = (w[0], w[1]);
        let Some(at) = first_output_time(p, a, b) else { continue };
        let mut actions = Vec::new();
        if let Some(log) = log {
            let mut typed = String::new();
            for (t, key, mods) in log.keys() {
                if t < a || t >= b {
                    continue;
                }
                if mods.iter().any(|m| m != "Shift") {
                    actions.push(format!("Pressed {}+{}", mods.join("+"), key));
                } else if key.chars().count() == 1 {
                    typed.push_str(key);
                } else if key == "Space" {
                    typed.push(' ');
                } else if key == "Return" || key == "Enter" {
                    actions.push("Pressed Enter".into());
                }
            }
            let clicks = log.clicks().filter(|(t, _, _)| *t >= a && *t < b).count();
            if clicks > 0 {
                actions.insert(0, format!("{clicks} click{}", if clicks == 1 { "" } else { "s" }));
            }
            if !typed.trim().is_empty() {
                actions.push(format!("Typed \"{}\"", typed.trim()));
            }
        }
        let said = transcript_between(p, a, b);
        if said.is_empty() && actions.is_empty() && !steps.is_empty() {
            continue;
        }
        steps.push(Step { at_ms: at, title: String::new(), said, actions });
    }
    steps
}

fn first_output_time(p: &Project, a: f64, b: f64) -> Option<f64> {
    let mut t = a;
    while t < b {
        if let Some(o) = p.output_time(t) {
            return Some(o);
        }
        t += 100.0;
    }
    None
}

pub(crate) fn first_sentence(s: &str, max_words: usize) -> String {
    let end = s.find(['.', '?', '!', ',', ';']).unwrap_or(s.len());
    let words: Vec<&str> = s[..end].split_whitespace().take(max_words).collect();
    let mut out = words.join(" ");
    if let Some(c) = out.get(..1) {
        out = c.to_uppercase() + &out[1..];
    }
    out
}

/// Rule based writer used when no local LLM is available.
pub fn heuristic(p: &Project, mut steps: Vec<Step>) -> DemoDocs {
    for (i, s) in steps.iter_mut().enumerate() {
        s.title = if !s.said.is_empty() {
            first_sentence(&s.said, 7)
        } else if let Some(a) = s.actions.iter().find(|a| a.starts_with("Typed")) {
            a.clone()
        } else if i == 0 {
            "Intro".into()
        } else {
            format!("Step {}", i + 1)
        };
    }
    let all: String = steps.iter().map(|s| s.said.as_str()).collect::<Vec<_>>().join(" ");
    let title = if all.trim().is_empty() {
        let name = p.source.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default();
        format!("Demo: {name}")
    } else {
        first_sentence(&all, 9)
    };
    let mins = p.output_duration_ms() / 60000.0;
    let summary = format!("A {} walkthrough in {} steps.", if mins < 1.0 { format!("{:.0} second", p.output_duration_ms() / 1000.0) } else { format!("{mins:.1} minute") }, steps.len());
    let social_post = format!("{title}. {summary} Watch the full demo below.");
    DemoDocs { title, summary, steps, social_post, generated_by: "rule based writer (no local LLM found)".into() }
}

fn context(steps: &[Step]) -> String {
    steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("Step {} at {}: said: \"{}\"; actions: {}", i + 1, fmt_ts(s.at_ms), s.said, if s.actions.is_empty() { "none".into() } else { s.actions.join(", ") }))
        .collect::<Vec<_>>()
        .join("\n")
}

const SYSTEM: &str = "You write clear, plain product documentation from screen recording notes. Use simple English. Never use em dashes. Never invent features that are not in the notes.";

/// Generate docs with a local LLM, falling back to the rule based writer
/// for any part the model fails on.
pub fn generate(p: &Project, log: Option<&demolens_core::events::EventLog>, llm: Option<&Llm>) -> Result<DemoDocs> {
    let steps = extract_steps(p, log);
    let mut d = heuristic(p, steps);
    let Some(llm) = llm else { return Ok(d) };
    let ctx = context(&d.steps);
    let n = d.steps.len();
    if let Ok(t) = llm.chat(SYSTEM, &format!("Notes from a product demo recording:\n{ctx}\n\nWrite one short title (max 8 words) for this demo video. Reply with the title only."), 40) {
        let t = t.lines().next().unwrap_or("").trim_matches(['"', '*', '#', ' ']).to_string();
        if !t.is_empty() {
            d.title = t;
        }
    }
    if let Ok(t) = llm.chat(SYSTEM, &format!("Notes from a product demo recording:\n{ctx}\n\nWrite a chapter name of 2 to 6 words for each of the {n} steps. Reply with exactly {n} lines, one name per line, no numbers."), 200) {
        let names: Vec<String> = t
            .lines()
            .map(clean_name)
            .filter(|l| !l.is_empty())
            .collect();
        if names.len() >= n {
            for (s, name) in d.steps.iter_mut().zip(names) {
                s.title = name;
            }
        }
    }
    if let Ok(t) = llm.chat(SYSTEM, &format!("Notes from a product demo recording:\n{ctx}\n\nWrite a two sentence summary of what the demo shows."), 160) {
        if !t.is_empty() {
            d.summary = t;
        }
    }
    if let Ok(t) = llm.chat(SYSTEM, &format!("Demo title: {}\nSummary: {}\n\nWrite a short, friendly social media post (under 280 characters) announcing this demo video. No emojis, at most two hashtags. Reply with the post only.", d.title, d.summary), 160) {
        if let Some(t) = tidy_post(&t) {
            d.social_post = t;
        }
    }
    d.generated_by = format!("local LLM: {}", llm.label);
    Ok(d)
}

/// Small models sometimes loop on hashtags and emojis. Keep the first
/// paragraph, drop emojis, keep at most two hashtags and 280 characters.
/// Returns None when nothing usable is left.
pub fn tidy_post(raw: &str) -> Option<String> {
    // Skip a preamble such as "Here's a friendly post:".
    let para = raw
        .split("\n\n")
        .map(|p| p.trim().trim_matches('"').trim())
        .find(|p| !p.is_empty() && !p.ends_with(':'))
        .unwrap_or("")
        .replace('\n', " ");
    let mut tags = 0;
    let mut words = Vec::new();
    for w in para.split_whitespace() {
        let w: String = w.chars().filter(|c| (*c as u32) < 0x2000 || c.is_alphanumeric()).collect();
        if w.is_empty() {
            continue;
        }
        if w.starts_with('#') {
            tags += 1;
            if tags > 2 || words.iter().any(|x: &String| x.eq_ignore_ascii_case(&w)) {
                continue;
            }
        }
        words.push(w);
    }
    let mut out = String::new();
    for w in words {
        if out.len() + w.len() + 1 > 280 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w);
    }
    let letters = out.chars().filter(|c| c.is_alphabetic()).count();
    (letters >= 20).then_some(out)
}

fn clean_name(l: &str) -> String {
    // Models decorate list items in many ways ("1. Step 1: \"Name\"", "**Name**").
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)^\s*(?:[-*#>]+\s*|(?:step|chapter)\s*\d*\s*[.):-]?\s*|\d+\s*[.):-]\s*)").unwrap());
    static TAIL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let tail = TAIL.get_or_init(|| regex::Regex::new(r"(?i)\s*[-(:]?\s*(?:step|chapter)\s*\d+\)?\s*$").unwrap());
    let mut cur = l.trim().replace(['"', '\u{201c}', '\u{201d}'], "");
    loop {
        let next = re.replace(&cur, "");
        let next = tail.replace(&next, "").trim().trim_matches(['"', '*', '\'']).trim().to_string();
        if next == cur {
            return next;
        }
        cur = next;
    }
}

pub fn chapters_text(d: &DemoDocs) -> String {
    let mut out = String::new();
    for (i, s) in d.steps.iter().enumerate() {
        // YouTube needs the first chapter at 00:00.
        let at = if i == 0 { 0.0 } else { s.at_ms };
        out.push_str(&format!("{} {}\n", fmt_ts(at), s.title));
    }
    out
}

pub fn markdown(d: &DemoDocs) -> String {
    let mut out = format!("# {}\n\n{}\n\n## Chapters\n\n", d.title, d.summary);
    for l in chapters_text(d).lines() {
        out.push_str(&format!("- {l}\n"));
    }
    out.push_str("\n## Step by step\n\n");
    for (i, s) in d.steps.iter().enumerate() {
        out.push_str(&format!("### {}. {} ({})\n\n", i + 1, s.title, fmt_ts(s.at_ms)));
        if !s.said.is_empty() {
            out.push_str(&format!("{}\n\n", s.said));
        }
        for a in &s.actions {
            out.push_str(&format!("- {a}\n"));
        }
        if !s.actions.is_empty() {
            out.push('\n');
        }
    }
    out.push_str(&format!("## Social post\n\n{}\n\n_Generated offline by DemoLens with the {}._\n", d.social_post, d.generated_by));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use demolens_core::project::{Caption, CaptionWord};
    use demolens_core::zoom::ZoomSegment;

    fn project() -> Project {
        let mut p = Project::new("demo_invoice.mp4".into(), None, 100, 100, 20_000.0);
        let z = |a: f64, b: f64| ZoomSegment { start_ms: a, end_ms: b, scale: 2.0, follow_cursor: false, focus: (0.0, 0.0), reason: "click".into() };
        p.zoom = vec![z(1000.0, 4000.0), z(9000.0, 12000.0)];
        let w = |t: f64, s: &str| CaptionWord { start_ms: t, end_ms: t + 300.0, text: s.into() };
        let words = vec![w(1100.0, "Open"), w(1400.0, "the"), w(1700.0, "invoices"), w(2000.0, "page."), w(9100.0, "um"), w(9500.0, "Export"), w(9800.0, "to"), w(10100.0, "PDF.")];
        p.captions = vec![Caption { start_ms: 1100.0, end_ms: 10400.0, text: String::new(), words }];
        p
    }

    #[test]
    fn steps_follow_activity_and_skip_fillers() {
        let p = project();
        let d = heuristic(&p, extract_steps(&p, None));
        assert_eq!(d.steps.len(), 2, "{:?}", d.steps);
        assert_eq!(d.steps[0].title, "Open the invoices page");
        assert_eq!(d.steps[1].title, "Export to PDF");
        assert_eq!(d.steps[1].at_ms, 9000.0);
        assert_eq!(chapters_text(&d), "00:00 Open the invoices page\n00:09 Export to PDF\n");
        assert!(markdown(&d).contains("### 2. Export to PDF (00:09)"));
    }

    #[test]
    fn runaway_posts_are_tidied() {
        let raw = "\u{1F680}\u{2728} Invoices Saved Successfully #Invoices #InvoiceManagement \u{1F310} #Invoices #InvoiceManagement #More";
        assert_eq!(tidy_post(raw).unwrap(), "Invoices Saved Successfully #Invoices #InvoiceManagement");
        assert!(tidy_post("#a #b").is_none());
        assert_eq!(tidy_post("Here's a post:\n\n\"Saving invoices is now one click.\"").unwrap(), "Saving invoices is now one click.");
        assert!(tidy_post(&"word ".repeat(200)).unwrap().len() <= 280);
    }

    #[test]
    fn llm_chapter_names_are_cleaned() {
        assert_eq!(clean_name("Step 1: \"Invoice Search\""), "Invoice Search");
        assert_eq!(clean_name("2. **Save**"), "Save");
        assert_eq!(clean_name("- Export to PDF"), "Export to PDF");
        assert_eq!(clean_name("1. Step 1: \"Invoices Page\""), "Invoices Page");
        assert_eq!(clean_name("3D printing"), "3D printing");
        assert_eq!(clean_name("Invoice Search\" Chapter 1"), "Invoice Search");
        assert_eq!(clean_name("Save (Step 2)"), "Save");
    }

    #[test]
    fn chapters_shift_after_cuts() {
        let mut p = project();
        p.cut_ranges(&[(4000.0, 8000.0)]);
        let d = heuristic(&p, extract_steps(&p, None));
        assert_eq!(d.steps[1].at_ms, 5000.0);
    }
}
