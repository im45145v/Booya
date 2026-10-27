//! `demolens` command line tool. Everything the desktop editor does is also
//! available here, which makes the pipeline easy to script and test.

use anyhow::{bail, Context, Result};
use demolens_core::capture::{self, CaptureTarget, RecordOptions};
use demolens_core::events::{Event, EventKind, EventLog, MouseButton};
use demolens_core::preset::{self, Preset};
use demolens_core::project::{Aspect, Format, Project};
use demolens_core::{export, ffmpeg, new_project, zoom};
use std::path::{Path, PathBuf};
use std::process::Command;

mod ai;

const HELP: &str = "DemoLens: local screen recorder for product demos

USAGE:
  demolens record [--screen N | --window ID | --area X,Y,W,H] [--mic] [--system-audio]
                  [--fps 30] [--seconds N] [--out DIR]
  demolens new <video> [--events FILE] [-o project.json]
  demolens zoom <events.jsonl> --duration MS
  demolens preview <project.json> --at MS [-o frame.png] [--quality 720]
  demolens export <project.json> -o out.mp4|out.gif [--aspect 16:9|9:16|1:1]
                  [--quality 720|1080|1440|2160] [--fps 30]
  demolens trim <project.json> START_MS END_MS
  demolens split <project.json> AT_MS
  demolens speed <project.json> CLIP_INDEX SPEED
  demolens blur <project.json> X,Y,W,H [START_MS END_MS]
  demolens preset save <project.json> NAME | apply <project.json> NAME | list
  demolens sample <dir> [--speech]   synthetic recording + event log
                  (--speech adds espeak-ng narration and on-screen secrets)

LOCAL AI (offline, models download once on first use):
  demolens models [list | download ID | delete ID]
  demolens transcribe <project.json> [--model whisper-base.en] [--lang en]
  demolens captions <project.json> [--burn on|off] [--size PX] [-o subs.srt|subs.vtt]
  demolens autocut <project.json> [--fillers] [--silences] [--min-silence MS] [--dry-run]
  demolens redact <project.json> [--every MS] [--dry-run]
  demolens docs <project.json> [-o DIR] [--no-llm]
  demolens audio <project.json> [--denoise on|off] [--mute on|off]
";

pub(crate) struct Args(pub(crate) Vec<String>);
impl Args {
    pub(crate) fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.0.iter().position(|a| a == name) {
            self.0.remove(i);
            true
        } else {
            false
        }
    }
    pub(crate) fn opt(&mut self, names: &[&str]) -> Option<String> {
        let i = self.0.iter().position(|a| names.contains(&a.as_str()))?;
        self.0.remove(i);
        (i < self.0.len()).then(|| self.0.remove(i))
    }
    pub(crate) fn pos(&mut self, what: &str) -> Result<String> {
        if self.0.is_empty() {
            bail!("missing {what}\n\n{HELP}");
        }
        Ok(self.0.remove(0))
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn nums(s: &str) -> Result<Vec<f64>> {
    s.split(',').map(|v| v.trim().parse::<f64>().context("expected numbers")).collect()
}

fn run() -> Result<()> {
    let mut a = Args(std::env::args().skip(1).collect());
    let cmd = a.0.first().cloned().unwrap_or_default();
    if !a.0.is_empty() {
        a.0.remove(0);
    }
    match cmd.as_str() {
        "record" => record(a),
        "new" => {
            let video = PathBuf::from(a.pos("video")?);
            let events = a.opt(&["--events"]).map(PathBuf::from);
            let out = a.opt(&["-o"]).map(PathBuf::from).unwrap_or_else(|| video.with_extension("demolens.json"));
            let p = new_project(&video, events.as_deref())?;
            p.save(&out)?;
            println!("project {} ({} zoom segments)", out.display(), p.zoom.len());
            Ok(())
        }
        "zoom" => {
            let ev = PathBuf::from(a.pos("events file")?);
            let dur: f64 = a.opt(&["--duration"]).context("--duration required")?.parse()?;
            let log = EventLog::load(&ev)?;
            let segs = zoom::generate_zoom_segments(&log, dur, &zoom::ZoomConfig::default());
            println!("{}", serde_json::to_string_pretty(&segs)?);
            Ok(())
        }
        "preview" => {
            let pp = PathBuf::from(a.pos("project")?);
            let at: f64 = a.opt(&["--at"]).unwrap_or("0".into()).parse()?;
            let q: u32 = a.opt(&["--quality"]).unwrap_or("720".into()).parse()?;
            let out = a.opt(&["-o"]).unwrap_or("preview.png".into());
            let p = Project::load(&pp)?;
            let (w, h) = p.export.aspect.size(q);
            std::fs::write(&out, export::preview_png(&p, at, w, h)?)?;
            println!("wrote {out}");
            Ok(())
        }
        "export" => {
            let pp = PathBuf::from(a.pos("project")?);
            let out = PathBuf::from(a.opt(&["-o"]).context("-o required")?);
            let mut p = Project::load(&pp)?;
            if let Some(v) = a.opt(&["--aspect"]) {
                p.export.aspect = Aspect::parse(&v)?;
            }
            if let Some(v) = a.opt(&["--quality"]) {
                p.export.quality = v.parse()?;
            }
            if let Some(v) = a.opt(&["--fps"]) {
                p.export.fps = v.parse()?;
            }
            p.export.format = if out.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif")) { Format::Gif } else { Format::Mp4 };
            let t0 = std::time::Instant::now();
            let mut last = -1i64;
            let st = export::export(&p, &out, |f| {
                let pct = (f * 100.0) as i64;
                if pct / 10 != last / 10 {
                    eprint!("\rexporting {pct}%");
                    last = pct;
                }
            })?;
            eprintln!();
            println!(
                "wrote {} ({}x{}, {} frames, {:.1}s video) in {:.1}s",
                out.display(), st.width, st.height, st.frames, st.seconds, t0.elapsed().as_secs_f64()
            );
            Ok(())
        }
        "trim" | "split" | "speed" | "blur" => {
            let pp = PathBuf::from(a.pos("project")?);
            let mut p = Project::load(&pp)?;
            match cmd.as_str() {
                "trim" => p.trim(a.pos("start")?.parse()?, a.pos("end")?.parse()?),
                "split" => {
                    if !p.split(a.pos("at")?.parse()?) {
                        bail!("no clip at that time");
                    }
                }
                "speed" => p.set_speed(a.pos("index")?.parse()?, a.pos("speed")?.parse()?),
                _ => {
                    let r = nums(&a.pos("x,y,w,h")?)?;
                    if r.len() != 4 {
                        bail!("blur box must be X,Y,W,H");
                    }
                    let start = a.0.first().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let end = a.0.get(1).and_then(|v| v.parse().ok()).unwrap_or(p.duration_ms);
                    p.blur.push(demolens_core::project::BlurBox { x: r[0], y: r[1], w: r[2], h: r[3], start_ms: start, end_ms: end, radius: 18 });
                }
            }
            p.validate()?;
            p.save(&pp)?;
            println!("{} clips, output {:.1}s", p.clips.len(), p.output_duration_ms() / 1000.0);
            Ok(())
        }
        "preset" => match a.pos("save|apply|list")?.as_str() {
            "list" => {
                for n in preset::list_presets() {
                    println!("{n}");
                }
                Ok(())
            }
            "save" => {
                let p = Project::load(Path::new(&a.pos("project")?))?;
                let path = preset::save_preset(&Preset::from_project(&a.pos("name")?, &p))?;
                println!("saved {}", path.display());
                Ok(())
            }
            "apply" => {
                let pp = PathBuf::from(a.pos("project")?);
                let mut p = Project::load(&pp)?;
                preset::load_preset(&a.pos("name")?)?.apply(&mut p);
                p.save(&pp)?;
                println!("applied");
                Ok(())
            }
            o => bail!("unknown preset command {o}"),
        },
        "sample" => {
            let speech = a.flag("--speech");
            sample(&PathBuf::from(a.pos("dir")?), speech)
        }
        "models" => ai::models_cmd(a),
        "transcribe" => ai::transcribe_cmd(a),
        "captions" => ai::captions_cmd(a),
        "autocut" => ai::autocut_cmd(a),
        "redact" => ai::redact_cmd(a),
        "docs" => ai::docs_cmd(a),
        "audio" => ai::audio_cmd(a),
        "" | "-h" | "--help" | "help" => {
            print!("{HELP}");
            Ok(())
        }
        o => bail!("unknown command {o}\n\n{HELP}"),
    }
}

fn record(mut a: Args) -> Result<()> {
    let target = if let Some(w) = a.opt(&["--window"]) {
        CaptureTarget::Window { id: w }
    } else if let Some(r) = a.opt(&["--area"]) {
        let v = nums(&r)?;
        if v.len() != 4 {
            bail!("--area needs X,Y,W,H");
        }
        CaptureTarget::Area { x: v[0] as i32, y: v[1] as i32, w: v[2] as u32, h: v[3] as u32 }
    } else {
        CaptureTarget::Screen { display: a.opt(&["--screen"]).map(|v| v.parse()).transpose()?.unwrap_or(0) }
    };
    let opts = RecordOptions {
        target,
        fps: a.opt(&["--fps"]).map(|v| v.parse()).transpose()?.unwrap_or(30),
        mic: a.flag("--mic"),
        system_audio: a.flag("--system-audio"),
        out_dir: a.opt(&["--out"]).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("recording")),
    };
    let seconds: Option<f64> = a.opt(&["--seconds"]).map(|v| v.parse()).transpose()?;
    let session = capture::start(&opts)?;
    for w in &session.warnings {
        eprintln!("warning: {w}");
    }
    match seconds {
        Some(s) => std::thread::sleep(std::time::Duration::from_secs_f64(s)),
        None => {
            eprintln!("recording... press Enter to stop");
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
        }
    }
    let rec = session.stop()?;
    println!("video  {}\nevents {}\n({:.1}s)", rec.video.display(), rec.events.display(), rec.duration.as_secs_f64());
    Ok(())
}

/// Build a fake "app" recording with ffmpeg and a matching synthetic event
/// log, so the whole pipeline can be exercised without a display.
fn sample(dir: &Path, speech: bool) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let dir = &dir.canonicalize()?;
    let video = dir.join("sample.mp4");
    let (w, h, secs) = (1920u32, 1080u32, 12.0f64);
    // A light "window" with a sidebar, a toolbar, a text field and buttons.
    let vf = [
        "drawbox=x=0:y=0:w=1920:h=72:color=0x1f2937:t=fill",
        "drawbox=x=0:y=72:w=320:h=1008:color=0xe5e7eb:t=fill",
        "drawbox=x=40:y=140:w=240:h=44:color=0xc7d2fe:t=fill",
        "drawbox=x=40:y=210:w=240:h=44:color=0xd1d5db:t=fill",
        "drawbox=x=40:y=280:w=240:h=44:color=0xd1d5db:t=fill",
        "drawbox=x=420:y=160:w=900:h=60:color=0x9ca3af:t=2",
        "drawbox=x=1360:y=160:w=200:h=60:color=0x4f46e5:t=fill",
        "drawbox=x=420:y=300:w=1400:h=640:color=0xf3f4f6:t=fill",
        "drawbox=x=460:y=340:w=600:h=40:color=0xd1d5db:t=fill",
        "drawbox=x=460:y=410:w=1000:h=24:color=0xe5e7eb:t=fill",
        "drawbox=x=460:y=450:w=900:h=24:color=0xe5e7eb:t=fill",
        "drawbox=x=1600:y=880:w=180:h=50:color=0x10b981:t=fill",
        // typed text appears as a growing bar after 5.2s
        "drawbox=x=432:y=176:w='min(560,max(0,(t-5.2)*420))':h=28:color=0x374151:t=fill",
        // a "toast" after clicking save
        "drawbox=x=1500:y=980:w=360:h=60:color=0x111827:t=fill:enable='gte(t,9.2)'",
    ]
    .join(",");
    let mut vf = vf;
    let audio_in = if speech {
        if let Some(font) = sample_font() {
            // Fake secrets on screen so auto redaction has something to find.
            let f = font.to_string_lossy().replace('\\', "/").replace(':', "\\:");
            for (y, text) in [(352, "Billing contact  jane.doe@acme.com"), (412, "API key  sk_live_51HxQ2aBcDeFgHiJkLmN"), (452, "Card  4242 4242 4242 4242")] {
                vf.push_str(&format!(",drawtext=fontfile='{f}':text='{text}':x=470:y={y}:fontsize=26:fontcolor=0x111827"));
            }
        } else {
            eprintln!("warning: no font found for on-screen text");
        }
        narration(dir, secs)?
    } else {
        format!("sine=frequency=440:sample_rate=48000:duration={secs}")
    };
    let mut cmd = Command::new(ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("color=c=0xffffff:s={w}x{h}:r=30:d={secs}")]);
    if speech {
        cmd.arg("-i").arg(&audio_in);
    } else {
        cmd.args(["-f", "lavfi", "-i", &audio_in]);
    }
    let status = cmd
        .args(["-vf", &vf, "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&video)
        .status()?;
    if !status.success() {
        bail!("ffmpeg failed to make the sample video");
    }

    // Synthetic input: glide to the sidebar, click, go to the search field,
    // click and type, press Ctrl+S, then click Save.
    let mut log = EventLog::new(w, h);
    let mut ev = Vec::new();
    let path = |ev: &mut Vec<Event>, t0: f64, t1: f64, a: (f64, f64), b: (f64, f64)| {
        let steps = ((t1 - t0) / 16.0) as usize;
        for i in 0..=steps {
            let k = i as f64 / steps.max(1) as f64;
            let e = k * k * (3.0 - 2.0 * k);
            // a little hand jitter
            let j = ((i * 7919) % 13) as f64 / 13.0 * 3.0 - 1.5;
            ev.push(Event { t: t0 + (t1 - t0) * k, kind: EventKind::Move { x: a.0 + (b.0 - a.0) * e + j, y: a.1 + (b.1 - a.1) * e - j } });
        }
    };
    let click = |ev: &mut Vec<Event>, t: f64, x: f64, y: f64| ev.push(Event { t, kind: EventKind::Click { x, y, button: MouseButton::Left } });
    path(&mut ev, 0.0, 1200.0, (1500.0, 700.0), (160.0, 232.0));
    click(&mut ev, 1350.0, 160.0, 232.0);
    path(&mut ev, 1500.0, 2200.0, (160.0, 232.0), (160.0, 302.0));
    click(&mut ev, 2400.0, 160.0, 302.0);
    path(&mut ev, 3800.0, 4800.0, (160.0, 302.0), (600.0, 190.0));
    click(&mut ev, 4900.0, 600.0, 190.0);
    for (i, ch) in "invoice march".chars().enumerate() {
        let key = if ch == ' ' { "Space".to_string() } else { ch.to_string() };
        ev.push(Event { t: 5200.0 + i as f64 * 105.0, kind: EventKind::Key { key, modifiers: vec![] } });
    }
    ev.push(Event { t: 7000.0, kind: EventKind::Key { key: "S".into(), modifiers: vec!["Ctrl".into()] } });
    path(&mut ev, 7600.0, 8800.0, (600.0, 190.0), (1690.0, 905.0));
    click(&mut ev, 9000.0, 1690.0, 905.0);
    path(&mut ev, 10200.0, 11500.0, (1690.0, 905.0), (1200.0, 600.0));
    ev.sort_by(|a, b| a.t.total_cmp(&b.t));
    log.events = ev;
    let events = dir.join("sample.events.jsonl");
    log.save(&events)?;

    let p = new_project(&video, Some(&events))?;
    let proj = dir.join("sample.demolens.json");
    p.save(&proj)?;
    println!("video   {}\nevents  {}\nproject {} ({} zoom segments)", video.display(), events.display(), proj.display(), p.zoom.len());
    Ok(())
}

fn sample_font() -> Option<PathBuf> {
    [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "C:/Windows/Fonts/arial.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// Narration with filler words and a long pause, spoken by espeak-ng, mixed
/// over a little background hiss so denoise has something to remove.
fn narration(dir: &Path, secs: f64) -> Result<String> {
    let lines = [
        (0.3, "Let's open the invoices page."),
        (2.4, "Um, now we search for, uh, March."),
        (7.0, "Press control S to save it."),
        (9.3, "And there it is. Saved."),
    ];
    let mut cmd = Command::new(ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-y"]);
    let mut filter = String::new();
    for (i, (at, text)) in lines.iter().enumerate() {
        let wav = dir.join(format!("line{i}.wav"));
        let ok = Command::new("espeak-ng").args(["-s", "150", "-w"]).arg(&wav).arg(text).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            bail!("--speech needs espeak-ng installed");
        }
        cmd.arg("-i").arg(&wav);
        let ms = (at * 1000.0) as u64;
        filter.push_str(&format!("[{i}:a]aresample=48000,adelay={ms}|{ms}[a{i}];"));
    }
    let n = lines.len();
    cmd.args(["-f", "lavfi", "-i", &format!("anoisesrc=color=pink:amplitude=0.02:sample_rate=48000:duration={secs}")]);
    for i in 0..n {
        filter.push_str(&format!("[a{i}]"));
    }
    filter.push_str(&format!("[{n}:a]amix=inputs={}:duration=longest:normalize=0,atrim=0:{secs}[out]", n + 1));
    let out = dir.join("narration.wav");
    let st = cmd.args(["-filter_complex", &filter, "-map", "[out]", "-ac", "1"]).arg(&out).status()?;
    if !st.success() {
        bail!("ffmpeg failed to mix the narration");
    }
    for i in 0..n {
        let _ = std::fs::remove_file(dir.join(format!("line{i}.wav")));
    }
    Ok(out.to_string_lossy().into_owned())
}
