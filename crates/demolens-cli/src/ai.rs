//! Local AI commands: model manager, transcription, captions, auto cuts,
//! auto redaction and written docs. Everything runs on this machine.

use crate::Args;
use anyhow::{bail, Context, Result};
use demolens_ai::{docs, llm::Llm, models, redact, transcribe};
use demolens_core::captions::{self, AutoCutOptions};
use demolens_core::events::EventLog;
use demolens_core::{audio, project::Project};
use std::path::PathBuf;

fn bar(label: &str) -> impl FnMut(u64, u64) {
    let label = label.to_string();
    let mut last = u64::MAX;
    move |done, total| {
        let pct = if total > 0 { done * 100 / total } else { 0 };
        if pct != last {
            eprint!("\r{label} {pct:>3}%  {} / {}", models::human(done), models::human(total));
            last = pct;
        }
    }
}

fn on_off(v: &str) -> Result<bool> {
    match v {
        "on" | "true" | "1" | "yes" => Ok(true),
        "off" | "false" | "0" | "no" => Ok(false),
        _ => bail!("expected on or off"),
    }
}

pub fn models_cmd(mut a: Args) -> Result<()> {
    match a.0.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            println!("models folder: {}", models::models_dir().display());
            for m in models::list() {
                println!(
                    "{} {:<18} {:>9}  {:<8} {}  ({})",
                    if m.installed { "[x]" } else { "[ ]" },
                    m.info.id,
                    m.size_human,
                    format!("{:?}", m.info.kind).to_lowercase(),
                    m.info.title,
                    m.info.license
                );
            }
            Ok(())
        }
        "download" => {
            a.0.remove(0);
            let id = a.pos("model id")?;
            let p = models::download(&id, bar(&id))?;
            eprintln!();
            println!("installed {}", p.display());
            Ok(())
        }
        "delete" => {
            a.0.remove(0);
            let id = a.pos("model id")?;
            models::delete(&id)?;
            println!("deleted {id}");
            Ok(())
        }
        o => bail!("unknown models command {o} (list, download ID, delete ID)"),
    }
}

fn speech_model(a: &mut Args) -> Result<PathBuf> {
    match a.opt(&["--model"]) {
        Some(id) => models::require(&id),
        None => transcribe::default_model().context("no speech model installed. Run: demolens models download whisper-base.en"),
    }
}

pub fn transcribe_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let model = speech_model(&mut a)?;
    let lang = a.opt(&["--lang"]);
    let mut p = Project::load(&pp)?;
    let t0 = std::time::Instant::now();
    let mut words = transcribe::transcribe(&p.source, &model, lang.as_deref())?;
    let samples = audio::decode_mono(&p.source, 16_000)?;
    captions::snap_to_speech(&mut words, &audio::detect_silences(&samples, 16_000, -38.0, 250.0));
    p.captions = captions::words_to_captions(&words, 42, 900.0);
    p.save(&pp)?;
    let fillers = words.iter().filter(|w| captions::is_filler(&w.text)).count();
    println!(
        "{} words ({} fillers), {} caption lines in {:.1}s",
        words.len(),
        fillers,
        p.captions.len(),
        t0.elapsed().as_secs_f64()
    );
    for c in &p.captions {
        println!("  [{:>6.2}s] {}", c.start_ms / 1000.0, c.text);
    }
    Ok(())
}

pub fn captions_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let mut p = Project::load(&pp)?;
    if let Some(v) = a.opt(&["--burn"]) {
        p.caption_style.burn_in = on_off(&v)?;
    }
    if let Some(v) = a.opt(&["--size"]) {
        p.caption_style.size = v.parse()?;
    }
    if let Some(out) = a.opt(&["-o"]) {
        let text = if out.ends_with(".vtt") { captions::to_vtt(&p) } else { captions::to_srt(&p) };
        std::fs::write(&out, text)?;
        println!("wrote {out}");
    }
    p.save(&pp)?;
    println!("{} caption lines, burn in {}", p.captions.len(), if p.caption_style.burn_in { "on" } else { "off" });
    Ok(())
}

pub fn autocut_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let dry = a.flag("--dry-run");
    let mut o = AutoCutOptions::default();
    let only_f = a.flag("--fillers");
    let only_s = a.flag("--silences");
    if only_f || only_s {
        o.cut_fillers = only_f;
        o.cut_silences = only_s;
    }
    if let Some(v) = a.opt(&["--min-silence"]) {
        o.min_silence_ms = v.parse()?;
    }
    let mut p = Project::load(&pp)?;
    let words: Vec<_> = p.captions.iter().flat_map(|c| c.words.clone()).collect();
    if o.cut_fillers && words.is_empty() {
        eprintln!("note: no transcript yet, so only silences are cut (run demolens transcribe first)");
    }
    let silences = if o.cut_silences {
        let samples = audio::decode_mono(&p.source, 16_000)?;
        audio::detect_silences(&samples, 16_000, -38.0, o.min_silence_ms)
    } else {
        Vec::new()
    };
    let cuts = captions::plan_cuts(&words, &silences, &o);
    let before = p.output_duration_ms();
    for (s, e) in &cuts {
        let what = if words.iter().any(|w| captions::is_filler(&w.text) && w.start_ms >= s - 50.0 && w.end_ms <= e + 50.0) { "filler" } else { "silence" };
        println!("  cut {:>6.2}s to {:>6.2}s  ({what})", s / 1000.0, e / 1000.0);
    }
    if dry {
        println!("{} cuts planned (dry run, nothing changed)", cuts.len());
        return Ok(());
    }
    p.cut_ranges(&cuts);
    p.validate()?;
    p.save(&pp)?;
    println!("{} cuts, output {:.1}s -> {:.1}s", cuts.len(), before / 1000.0, p.output_duration_ms() / 1000.0);
    Ok(())
}

pub fn redact_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let every: f64 = a.opt(&["--every"]).unwrap_or("500".into()).parse()?;
    let dry = a.flag("--dry-run");
    let mut p = Project::load(&pp)?;
    let t0 = std::time::Instant::now();
    let mut last = -1i64;
    let hits = redact::scan(&p, every, |f| {
        let pct = (f * 100.0) as i64;
        if pct != last {
            eprint!("\rscanning {pct}%");
            last = pct;
        }
    })?;
    eprintln!();
    let mut boxes = redact::hits_to_blur(&hits, every, 6.0);
    for b in &mut boxes {
        if b.end_ms + 2.0 * every >= p.duration_ms {
            b.end_ms = p.duration_ms;
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for h in &hits {
        if seen.insert((format!("{:?}", h.kind), h.text.clone())) {
            println!("  found {:?}: {}", h.kind, h.text);
        }
    }
    println!("{} detections -> {} blur boxes in {:.1}s", hits.len(), boxes.len(), t0.elapsed().as_secs_f64());
    if !dry {
        // Running redact twice should not stack duplicate boxes.
        let dup = |a: &demolens_core::project::BlurBox, b: &demolens_core::project::BlurBox| {
            (a.x - b.x).abs() < 8.0 && (a.y - b.y).abs() < 8.0 && (a.w - b.w).abs() < 16.0 && (a.start_ms - b.start_ms).abs() < every
        };
        for b in boxes {
            if !p.blur.iter().any(|o| dup(o, &b)) {
                p.blur.push(b);
            }
        }
        p.save(&pp)?;
    }
    Ok(())
}

pub fn docs_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let out = PathBuf::from(a.opt(&["-o"]).unwrap_or("docs".into()));
    let no_llm = a.flag("--no-llm");
    let p = Project::load(&pp)?;
    let log = p.events.as_ref().and_then(|e| EventLog::load(e).ok());
    let llm = if no_llm { None } else { Llm::detect()? };
    match &llm {
        Some(l) => eprintln!("using {}", l.label),
        None => eprintln!("no local LLM found, using the rule based writer"),
    }
    let t0 = std::time::Instant::now();
    let d = docs::generate(&p, log.as_ref(), llm.as_ref())?;
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("guide.md"), docs::markdown(&d))?;
    std::fs::write(out.join("chapters.txt"), docs::chapters_text(&d))?;
    std::fs::write(out.join("post.txt"), &d.social_post)?;
    std::fs::write(out.join("docs.json"), serde_json::to_string_pretty(&d)?)?;
    println!("{}\n\n{}\n{}\n{}", d.title, d.summary, docs::chapters_text(&d), d.social_post);
    println!("\nwrote {} in {:.1}s", out.display(), t0.elapsed().as_secs_f64());
    Ok(())
}

pub fn audio_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let mut p = Project::load(&pp)?;
    if let Some(v) = a.opt(&["--denoise"]) {
        p.audio.denoise = on_off(&v)?;
    }
    if let Some(v) = a.opt(&["--mute"]) {
        p.audio.mute = on_off(&v)?;
    }
    p.save(&pp)?;
    println!("denoise {}, mute {}", p.audio.denoise, p.audio.mute);
    Ok(())
}
