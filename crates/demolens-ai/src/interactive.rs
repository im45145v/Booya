//! Interactive click-through demo: a single self-contained HTML file built
//! from a recording. Each click in the event log becomes a step with a
//! screenshot, a pulsing hotspot where the click happened and a short
//! caption taken from the narration. Nothing is loaded from the network and
//! nothing is uploaded; the file works from disk, an email attachment or any
//! static host. Blur boxes and the secret redaction already in the project
//! are applied to every screenshot.

use crate::docs;
use crate::llm::Llm;
use anyhow::{bail, Context, Result};
use base64::Engine;
use demolens_core::events::EventLog;
use demolens_core::project::Project;
use demolens_core::{export, ffmpeg};
use serde::Serialize;
use std::io::Write;
use std::process::Stdio;

#[derive(Debug, Clone)]
pub struct Options {
    /// Screenshot width in pixels.
    pub width: u32,
    pub title: Option<String>,
    pub cta_text: Option<String>,
    pub cta_url: Option<String>,
    /// Show the small "Made with Booya" line.
    pub badge: bool,
    pub max_steps: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { width: 1280, title: None, cta_text: None, cta_url: None, badge: true, max_steps: 40 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoStep {
    pub title: String,
    pub text: String,
    /// Click position as a fraction of the screenshot, when there was a click.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// JPEG data URL.
    pub image: String,
    /// Output time of the step, ms.
    pub at_ms: f64,
}

#[derive(Debug, Serialize)]
struct Page<'a> {
    title: &'a str,
    summary: &'a str,
    steps: &'a [DemoStep],
    cta_text: &'a str,
    cta_url: &'a str,
    badge: bool,
    aspect: f64,
}

/// Source times (ms) and positions (source px) of the clicks worth a step.
/// Rapid clicks on the same spot (double clicks, a click storm) count once.
pub fn click_points(p: &Project, log: &EventLog, max_steps: usize) -> Vec<(f64, f64, f64)> {
    let mut out: Vec<(f64, f64, f64)> = Vec::new();
    let near = p.source_width as f64 * 0.06;
    for (t, x, y) in log.clicks() {
        if p.output_time(t).is_none() {
            continue;
        }
        if let Some(&(lt, lx, ly)) = out.last() {
            if t - lt < 700.0 && (x - lx).abs() < near && (y - ly).abs() < near {
                continue;
            }
        }
        out.push((t, x, y));
    }
    // Too many steps make a bad demo. Drop the clicks closest in time to the previous one.
    while out.len() > max_steps.max(1) {
        let i = (1..out.len()).min_by(|&a, &b| (out[a].0 - out[a - 1].0).total_cmp(&(out[b].0 - out[b - 1].0))).unwrap_or(out.len() - 1);
        out.remove(i);
    }
    out
}

fn jpeg(png: &[u8]) -> Result<Vec<u8>> {
    let mut c = demolens_core::proc::command(ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-f", "png_pipe", "-i", "pipe:0", "-frames:v", "1", "-q:v", "4", "-pix_fmt", "yuvj420p", "-f", "mjpeg", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start ffmpeg")?;
    let mut stdin = c.stdin.take().unwrap();
    let data = png.to_vec();
    let w = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let out = c.wait_with_output()?;
    let _ = w.join();
    if !out.status.success() || out.stdout.is_empty() {
        bail!("ffmpeg could not encode the screenshot");
    }
    Ok(out.stdout)
}

fn even(v: f64) -> u32 {
    ((v.round() as u32) & !1).max(2)
}

/// Build the steps with their screenshots.
pub fn build_steps(p: &Project, log: Option<&EventLog>, opts: &Options, mut progress: impl FnMut(usize, usize)) -> Result<Vec<DemoStep>> {
    let mut flat = p.clone();
    flat.zoom.clear();
    flat.style.padding = 0.0;
    flat.style.corner_radius = 0.0;
    flat.style.shadow = 0.0;
    flat.style.glass.enabled = false;
    flat.cursor.visible = false;
    flat.cursor.click_ripples = false;
    flat.keys.enabled = false;
    flat.caption_style.burn_in = false;
    flat.webcam = None;
    let w = opts.width.clamp(320, 3840) & !1;
    let h = even(w as f64 * p.source_height as f64 / p.source_width.max(1) as f64);
    let r = export::make_renderer(&flat, w, h)?;
    let (dw, dh) = r.decode_size();

    // Points: clicks, or one per documented step when there are no clicks.
    let mut pts: Vec<(f64, Option<(f64, f64)>)> = Vec::new();
    match log {
        Some(l) if l.clicks().next().is_some() => {
            for (t, x, y) in click_points(p, l, opts.max_steps) {
                pts.push((t, Some((x / p.source_width as f64, y / p.source_height as f64))));
            }
        }
        _ => {
            for s in docs::extract_steps(p, log).into_iter().take(opts.max_steps) {
                if let Some(t) = p.source_time(s.at_ms) {
                    pts.push((t, None));
                }
            }
        }
    }
    if pts.is_empty() {
        bail!("nothing to show: no clicks and no activity in this recording");
    }
    let total = pts.len();
    let mut steps = Vec::new();
    for (i, (t, pos)) in pts.iter().enumerate() {
        progress(i, total);
        // A moment before the click, so the screenshot shows what was clicked.
        let shot_t = (t - 120.0).clamp(0.0, (p.duration_ms - 40.0).max(0.0));
        let src = ffmpeg::frame_at(&p.source, shot_t, dw, dh)?;
        let png = r.render(&src, shot_t, None).encode_png()?;
        let said = docs::transcript_between(p, t - 1500.0, t + 2500.0);
        let title = if !said.is_empty() {
            docs::first_sentence(&said, 8)
        } else if i == 0 {
            "Get started".to_string()
        } else {
            format!("Step {}", i + 1)
        };
        let text = if said.chars().count() > 160 { format!("{}...", said.chars().take(157).collect::<String>().trim_end()) } else { said };
        steps.push(DemoStep {
            title,
            text,
            x: pos.map(|q| q.0.clamp(0.0, 1.0)),
            y: pos.map(|q| q.1.clamp(0.0, 1.0)),
            image: format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg(&png)?)),
            at_ms: p.output_time(*t).unwrap_or(0.0),
        });
    }
    progress(total, total);
    Ok(steps)
}

const TEMPLATE: &str = include_str!("interactive.html");

/// Put the page data into the template. `</` is escaped so no text can end
/// the script block early.
pub fn render_html(title: &str, summary: &str, steps: &[DemoStep], opts: &Options, aspect: f64) -> Result<String> {
    let page = Page {
        title,
        summary,
        steps,
        cta_text: opts.cta_text.as_deref().unwrap_or(""),
        cta_url: opts.cta_url.as_deref().filter(|u| u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")).unwrap_or(""),
        badge: opts.badge,
        aspect,
    };
    let json = serde_json::to_string(&page)?.replace("</", "<\\/").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    Ok(TEMPLATE.replace("{{TITLE}}", &esc(title)).replace("{{DATA}}", &json))
}

/// The whole pipeline: steps, title and the HTML.
pub fn generate(p: &Project, log: Option<&EventLog>, llm: Option<&Llm>, opts: &Options, progress: impl FnMut(usize, usize)) -> Result<(String, usize)> {
    let steps = build_steps(p, log, opts, progress)?;
    let d = docs::generate(p, log, llm)?;
    let title = opts.title.clone().unwrap_or(d.title);
    let aspect = p.source_width as f64 / p.source_height.max(1) as f64;
    let html = render_html(&title, &d.summary, &steps, opts, aspect)?;
    Ok((html, steps.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use demolens_core::events::{Event, EventKind, MouseButton};

    fn click(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Click { x, y, button: MouseButton::Left } }
    }

    #[test]
    fn rapid_clicks_on_one_spot_make_one_step() {
        let p = Project::new("a.mp4".into(), None, 1000, 500, 20_000.0);
        let mut log = EventLog::new(1000, 500);
        log.events = vec![click(1000.0, 100.0, 100.0), click(1300.0, 102.0, 99.0), click(5000.0, 600.0, 300.0)];
        let pts = click_points(&p, &log, 40);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[1].0, 5000.0);
    }

    #[test]
    fn step_count_is_capped() {
        let p = Project::new("a.mp4".into(), None, 1000, 500, 100_000.0);
        let mut log = EventLog::new(1000, 500);
        log.events = (0..30).map(|i| click(i as f64 * 2000.0, (i * 30) as f64, 10.0)).collect();
        assert_eq!(click_points(&p, &log, 10).len(), 10);
    }

    #[test]
    fn script_cannot_be_closed_by_text() {
        let steps = vec![DemoStep { title: "</script><b>x".into(), text: "t".into(), x: None, y: None, image: "data:,".into(), at_ms: 0.0 }];
        let h = render_html("<i>T</i>", "s", &steps, &Options::default(), 1.7).unwrap();
        assert_eq!(h.matches("</script>").count(), TEMPLATE.matches("</script>").count());
        assert!(h.contains("<\\/script>"));
        assert!(h.contains("&lt;i&gt;T&lt;/i&gt;"));
    }
}
