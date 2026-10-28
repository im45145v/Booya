//! `demolens` command line tool. Everything the desktop editor does is also
//! available here, which makes the pipeline easy to script and test.

use anyhow::{bail, Context, Result};
use demolens_core::capture::{self, CaptureTarget, RecordOptions};
use demolens_core::events::EventLog;
use demolens_core::preset::{self, Preset};
use demolens_core::project::{Aspect, Format, Project};
use demolens_core::{export, new_project, zoom};
use std::path::{Path, PathBuf};

mod ai;
mod bench;

const HELP: &str = "DemoLens: local screen recorder for product demos

USAGE:
  demolens record [--screen N | --window ID | --area X,Y,W,H] [--mic] [--system-audio]
                  [--keys on] [--webcam]   (key logging is off by default)
                  [--fps 30] [--seconds N] [--out DIR]
  demolens sources [--thumbs dir] [--all]   list the screens and windows the picker shows (and save previews); --all also prints hidden windows and why
  demolens doctor [--seconds 3] [--prompt-keys]  check permissions, displays and ffmpeg, run a short
                  test capture and print the result as JSON (exit 1 if it failed)
  demolens bench [--seconds 8] [--size 2940x1912] [--fps 30] [--json]
                  measure decode, proxy and export speed on a synthetic recording of that size
  demolens verify <video> [events.jsonl]   A/V sync report for a finished recording
  demolens syncprobe [--capture] [--json] | --flashes FILE  Measure sound vs picture through every stage (exit 1 if off)
  demolens new <video> [--events FILE] [-o project.json]
  demolens zoom <events.jsonl> --duration MS
  demolens preview <project.json> --at MS [-o frame.png] [--quality 720]
  demolens export <project.json> -o out.mp4|out.webm|out.gif [--aspect 16:9|9:16|1:1]
                  [--quality 720|1080|1440|2160] [--fps 30]
  demolens trim <project.json> START_MS END_MS
  demolens split <project.json> AT_MS
  demolens speed <project.json> CLIP_INDEX SPEED
  demolens blur <project.json> X,Y,W,H [START_MS END_MS]
  demolens preset save <project.json> NAME | apply <project.json> NAME | list
  demolens ready <project.json> [--no-zoom] [--no-captions] [--no-cut] [--no-blur] [--no-denoise]
                  auto edit: zoom, captions, cuts, blur and clean audio in one pass
  demolens sample <dir> [--speech]   synthetic recording + event log
                  (--speech adds espeak-ng narration and on-screen secrets)

LOCAL AI (offline, models download once on first use):
  demolens models [list | download ID | verify | delete ID]
  demolens transcribe <project.json> [--model whisper-base.en] [--lang en]
  demolens captions <project.json> [--burn on|off] [--size PX] [-o subs.srt|subs.vtt]
  demolens autocut <project.json> [--fillers] [--silences] [--min-silence MS] [--dry-run]
  demolens redact <project.json> [--every MS] [--dry-run]
  demolens docs <project.json> [-o DIR] [--no-llm]
  demolens interactive <project.json> [-o demo.html] [--title T] [--cta-text T --cta-url URL]
                  [--width 1280] [--max-steps 40] [--no-badge] [--no-llm]
                  one self-contained click-through demo (HTML), built from the clicks
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
    demolens_core::input::run_keyhook_if_requested();
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
        "doctor" => {
            let secs: f64 = a.opt(&["--seconds"]).map(|v| v.parse()).transpose()?.unwrap_or(3.0);
            if a.flag("--prompt-keys") {
                demolens_core::input::key_permission_missing(true);
            }
            let mut report = capture::doctor(secs);
            if a.flag("--no-ai") {
                // capture only
            } else {
                let cloud = |t: demolens_ai::providers::Task| {
                    let cfg = demolens_ai::providers::CloudConfig::load();
                    cfg.active(t).filter(|p| std::env::var(p.env_key()).is_ok()).map(|p| p.info().name.to_string())
                };
                let mut ai = serde_json::to_value(demolens_ai::status::features(&cloud))?;
                if a.flag("--self-test") {
                    for f in ai.as_array_mut().into_iter().flatten() {
                        if f["can_test"] == true {
                            let id = f["id"].as_str().unwrap_or("").to_string();
                            f["self_test"] = serde_json::to_value(demolens_ai::status::self_test(&id))?;
                        }
                    }
                }
                report["ai"] = ai;
                report["models_dir"] = demolens_ai::models::models_dir().display().to_string().into();
                report["mics"] = serde_json::to_value(demolens_core::capture::mics::list_mics())?;
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report["ok"] != true {
                std::process::exit(1);
            }
            Ok(())
        }
        "bench" => bench::run(a),
        "sources" => {
            // What the app's picker shows. With --thumbs <dir> also writes the previews.
            use demolens_core::capture::sources;
            let dir = a.opt(&["--thumbs"]).map(PathBuf::from);
            let all = a.flag("--all");
            let list = sources::list_sources()?;
            if all {
                // Every window the system reports and why the picker hides it.
                for line in sources::window_report()? {
                    eprintln!("{line}");
                }
            }
            if let Some(d) = &dir {
                std::fs::create_dir_all(d)?;
                for (i, s) in list.iter().enumerate() {
                    match sources::thumbnail(s) {
                        Ok(t) => {
                            if let Some(img) = t.image {
                                std::fs::write(d.join(format!("{i:02}-{}.png", s.kind)), img)?;
                            }
                            if let Some(icon) = t.icon {
                                std::fs::write(d.join(format!("{i:02}-icon.png")), icon)?;
                            }
                        }
                        Err(e) => eprintln!("{i}: no preview for {}: {e}", s.title),
                    }
                }
            }
            println!("{}", serde_json::to_string_pretty(&list)?);
            Ok(())
        }
        "syncprobe" => {
            // Beep + flash through every stage that works on files, and (on
            // Linux, with --capture) through a real screen capture. Exits 1
            // when any stage moves the sound against the picture too far.
            if let Some(f) = a.opt(&["--flashes"]) {
                // Video only: how steady the flashes of a capture are.
                let (n, period, worst) = demolens_core::syncprobe::flash_regularity(Path::new(&f))?;
                println!("{n} flashes, mean period {period:.4} s (1.0000 is exact), worst deviation {worst:.0} ms");
                if (period - 1.0).abs() > 0.01 || worst > 80.0 {
                    std::process::exit(1);
                }
                return Ok(());
            }
            let dir = a.opt(&["--dir"]).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("demolens-syncprobe"));
            let json = a.0.iter().any(|v| v == "--json");
            let rows = demolens_core::syncprobe::run_file_stages(&dir)?;
            let mut bad = rows.iter().any(|r| !r.ok);
            #[allow(unused_mut)]
            let mut cap = Vec::new();
            #[cfg(target_os = "linux")]
            if a.0.iter().any(|v| v == "--capture") {
                let secs: u32 = a.opt(&["--seconds"]).and_then(|v| v.parse().ok()).unwrap_or(20);
                cap = demolens_core::syncprobe::run_capture_linux(&dir.join("capture"), secs)?;
                // Only the new method is held to the budget. The previous method is shown
                // for comparison, and its numbers vary from run to run.
                for r in cap.iter().filter(|r| r.method.contains("new")) {
                    match &r.result {
                        Some(x) => bad |= x.drift_total_ms.abs() > demolens_core::syncprobe::BUDGET_DRIFT_TOTAL_MS || x.jitter_ms > demolens_core::syncprobe::BUDGET_JITTER_MS,
                        None => bad |= r.method.contains("new"),
                    }
                }
            }
            if json {
                println!("{}", serde_json::json!({ "stages": rows, "capture": cap }));
            } else {
                println!("{}", demolens_core::syncprobe::format_rows(&rows));
                if !cap.is_empty() {
                    println!("{}", demolens_core::syncprobe::format_capture(&cap));
                }
            }
            if bad {
                std::process::exit(1);
            }
            Ok(())
        }
        "verify" => {
            // Sync report for a finished recording: A/V offset, duration, events.
            let video = PathBuf::from(a.pos("video")?);
            let events = a.0.first().filter(|v| !v.starts_with("--")).map(PathBuf::from).or_else(|| {
                let guess = video.with_file_name("recording.events.jsonl");
                guess.exists().then_some(guess)
            });
            let r = demolens_core::finalize::sync_report(&video, events.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(())
        }
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
            p.export.format = Format::from_path(&out).unwrap_or(Format::Mp4);
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
            let proj = demolens_core::sample::make(&PathBuf::from(a.pos("dir")?), speech, speech)?;
            println!("project {}", proj.display());
            Ok(())
        }
        "models" => ai::models_cmd(a),
        "transcribe" => ai::transcribe_cmd(a),
        "captions" => ai::captions_cmd(a),
        "autocut" => ai::autocut_cmd(a),
        "ready" => ai::ready_cmd(a),
        "redact" => ai::redact_cmd(a),
        "docs" => ai::docs_cmd(a),
        "interactive" => ai::interactive_cmd(a),
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
        keys: match a.opt(&["--keys"]) {
            Some(k) => demolens_core::input::KeyLogging::parse(&k).context("--keys must be on or off")?,
            None => Default::default(),
        },
        webcam: a.flag("--webcam"),
        mic_device: a.opt(&["--mic-device"]),
        max_height: a.opt(&["--max-height"]).map(|v| v.parse()).transpose().context("--max-height needs a number of pixels (0 for native)")?,
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

