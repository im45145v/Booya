//! A synthetic recording (video, input log and project) so the whole app can
//! be tried without recording anything. Used by `demolens sample` and by the
//! "Try the sample" button in the desktop app.

use crate::events::{Event, EventKind, EventLog, MouseButton};
use crate::{ffmpeg, new_project};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// True when espeak-ng is installed, so the sample can have narration.
pub fn can_speak() -> bool {
    crate::proc::command("espeak-ng").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Build a fake "app" recording with ffmpeg and a matching synthetic event
/// log, so the whole pipeline can be exercised without a display.
/// Returns the path of the sample project.
pub fn make(dir: &Path, speech: bool, secrets: bool) -> Result<PathBuf> {
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
    if let Some(font) = sample_font().filter(|_| secrets) {
        // Fake secrets on screen so auto redaction has something to find.
            let f = font.to_string_lossy().replace('\\', "/").replace(':', "\\:");
            for (y, text) in [(352, "Billing contact  jane.doe@acme.com"), (412, "API key  sk_live_51HxQ2aBcDeFgHiJkLmN"), (452, "Card  4242 4242 4242 4242")] {
                vf.push_str(&format!(",drawtext=fontfile='{f}':text='{text}':x=470:y={y}:fontsize=26:fontcolor=0x111827"));
            }
    }
    let audio_in = if speech {
        narration(dir, secs)?
    } else {
        format!("sine=frequency=440:sample_rate=48000:duration={secs}")
    };
    let mut cmd = crate::proc::command(ffmpeg::ffmpeg_bin());
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
    Ok(proj)
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
pub fn narration(dir: &Path, secs: f64) -> Result<String> {
    let lines = [
        (0.3, "Let's open the invoices page."),
        (2.4, "Um, now we search for, uh, March."),
        (7.0, "Press control S to save it."),
        (9.3, "And there it is. Saved."),
    ];
    let mut cmd = crate::proc::command(ffmpeg::ffmpeg_bin());
    cmd.args(["-v", "error", "-y"]);
    let mut filter = String::new();
    for (i, (at, text)) in lines.iter().enumerate() {
        let wav = dir.join(format!("line{i}.wav"));
        let ok = crate::proc::command("espeak-ng").args(["-s", "150", "-w"]).arg(&wav).arg(text).status().map(|s| s.success()).unwrap_or(false);
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
