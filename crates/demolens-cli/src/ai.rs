//! Local AI commands: model manager, transcription, captions, auto cuts,
//! auto redaction and written docs. Everything runs on this machine.

use crate::Args;
use anyhow::{bail, Result};
use demolens_ai::{docs, llm::Llm, models, pipeline};
use demolens_core::captions::{self, AutoCutOptions};
use demolens_core::events::EventLog;
use demolens_core::project::Project;
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
        "verify" => {
            let mut bad = 0;
            for m in models::list().into_iter().filter(|m| m.installed || models::path_of(&m.info).exists()) {
                let ok = models::verify(m.info.id).unwrap_or(false);
                println!("{} {}", if ok { "ok      " } else { "MISMATCH" }, m.info.id);
                bad += usize::from(!ok);
            }
            if bad > 0 {
                bail!("{bad} model(s) failed verification; delete and download them again");
            }
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

pub fn transcribe_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let model = a.opt(&["--model"]);
    let lang = a.opt(&["--lang"]);
    let mut p = Project::load(&pp)?;
    let t0 = std::time::Instant::now();
    let r = pipeline::transcribe_project(&mut p, model.as_deref(), lang.as_deref())?;
    p.save(&pp)?;
    println!("{} words ({} fillers), {} caption lines in {:.1}s", r.words, r.fillers, r.lines, t0.elapsed().as_secs_f64());
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
    let r = pipeline::autocut_project(&mut p, &o, dry)?;
    if o.cut_fillers && !r.had_transcript {
        eprintln!("note: no transcript yet, so only silences are cut (run demolens transcribe first)");
    }
    for (s, e, what) in &r.cuts {
        println!("  cut {:>6.2}s to {:>6.2}s  ({what})", s / 1000.0, e / 1000.0);
    }
    if dry {
        println!("{} cuts planned (dry run, nothing changed)", r.cuts.len());
        return Ok(());
    }
    p.save(&pp)?;
    println!("{} cuts, output {:.1}s -> {:.1}s", r.cuts.len(), r.before_ms / 1000.0, r.after_ms / 1000.0);
    Ok(())
}

pub fn redact_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let every: f64 = a.opt(&["--every"]).unwrap_or("500".into()).parse()?;
    let dry = a.flag("--dry-run");
    let mut p = Project::load(&pp)?;
    let t0 = std::time::Instant::now();
    let mut last = -1i64;
    let r = pipeline::redact_project(&mut p, every, dry, |f| {
        let pct = (f * 100.0) as i64;
        if pct != last {
            eprint!("\rscanning {pct}%");
            last = pct;
        }
    })?;
    eprintln!();
    for (k, t) in &r.found {
        println!("  found {k:?}: {t}");
    }
    println!("{} detections -> {} blur boxes added in {:.1}s", r.detections, r.added, t0.elapsed().as_secs_f64());
    if !dry {
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

pub fn ready_cmd(mut a: Args) -> Result<()> {
    let pp = PathBuf::from(a.pos("project")?);
    let o = pipeline::ReadyOptions {
        zoom: !a.flag("--no-zoom"),
        captions: !a.flag("--no-captions"),
        cut: !a.flag("--no-cut"),
        blur: !a.flag("--no-blur"),
        denoise: !a.flag("--no-denoise"),
        ..Default::default()
    };
    let mut p = Project::load(&pp)?;
    let r = pipeline::demo_ready(&mut p, &o, |f, msg| eprint!("\r{:>3}% {msg:<45}", (f * 100.0) as u32));
    eprintln!();
    for s in &r.steps {
        println!("  [{}] {}: {}", s.state, s.name, s.detail);
    }
    println!("length {:.1}s -> {:.1}s", r.before_ms / 1000.0, r.after_ms / 1000.0);
    p.save(&pp)?;
    Ok(())
}
