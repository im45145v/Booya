//! `demolens bench`: speed numbers for the editor on this computer.
//!
//! Makes a synthetic screen recording (default 2940x1912 at 30 fps, the size
//! of a MacBook Air display) and times: decoding preview frames from the
//! original and from the 720p proxy, building the proxy, and exporting 1080p
//! with the hardware encoder (when there is one) and with software x264.

use crate::Args;
use anyhow::{bail, Context, Result};
use demolens_core::ffmpeg::{self, Encoder, PreviewDecoder};
use demolens_core::{export, new_project, proxy};
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

fn make_source(path: &Path, secs: f64, w: u32, h: u32, fps: u32) -> Result<f64> {
    let t = Instant::now();
    let st = Command::new(ffmpeg::ffmpeg_bin())
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={w}x{h}:rate={fps}:duration={secs}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=220:sample_rate=48000:duration={secs}"))
        .args(["-af", "volume=-30dB", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "20", "-pix_fmt", "yuv420p", "-g", "30", "-c:a", "aac", "-shortest", "-movflags", "+faststart"])
        .arg(path)
        .status()
        .context("run ffmpeg")?;
    if !st.success() {
        bail!("could not make the test video");
    }
    Ok(t.elapsed().as_secs_f64())
}

/// (ms per frame while playing, ms per single seek)
fn decode_speed(path: &Path, w: u32, h: u32, secs: f64) -> Result<(f64, f64)> {
    let mut d = PreviewDecoder::new(path, w, h);
    d.frame(0.0, false)?;
    let n = 60;
    let t = Instant::now();
    for i in 0..n {
        d.frame(500.0 + i as f64 * 1000.0 / 30.0, true)?;
    }
    let play = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
    let seeks = 8;
    let t = Instant::now();
    for i in 0..seeks {
        d.frame((secs * 1000.0 - 600.0) * (i as f64 + 1.0) / (seeks as f64 + 1.0) + 17.0 * i as f64, false)?;
    }
    Ok((play, t.elapsed().as_secs_f64() * 1000.0 / seeks as f64))
}

pub fn run(mut a: Args) -> Result<()> {
    let secs: f64 = a.opt(&["--seconds"]).map(|v| v.parse()).transpose()?.unwrap_or(8.0);
    let size = a.opt(&["--size"]).unwrap_or_else(|| "2940x1912".into());
    let fps: u32 = a.opt(&["--fps"]).map(|v| v.parse()).transpose()?.unwrap_or(30);
    let as_json = a.flag("--json");
    let (w, h) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))).context("--size must look like 2940x1912")?;
    let dir = std::env::temp_dir().join(format!("demolens-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let src = dir.join("bench.mp4");
    let make_s = make_source(&src, secs, w, h, fps)?;
    let hw = ffmpeg::best_encoder();
    let mut out = json!({ "source": format!("{w}x{h}@{fps} {secs}s"), "encoder": hw.name(), "hwaccel_decode": !ffmpeg::hwaccel_args().is_empty() });

    // Preview decode size the editor uses at 540 px preview height with no zoom.
    let (pw, ph) = (960u32, ((960.0 * h as f64 / w as f64) as u32) & !1);
    let (orig_play, orig_seek) = decode_speed(&src, pw, ph, secs)?;
    out["decode_original_ms_per_frame"] = json!(orig_play);
    out["seek_original_ms"] = json!(orig_seek);

    let cache = dir.join("cache");
    let st = proxy::build(&cache, &src, |_| {})?;
    out["proxy_build_s"] = json!(st.seconds);
    out["proxy_encoder"] = json!(st.encoder);
    out["proxy_size"] = json!(format!("{}x{}", st.width, st.height));
    let (px_play, px_seek) = decode_speed(&st.path, pw.min(st.width) & !1, ((pw.min(st.width) as f64 * h as f64 / w as f64) as u32) & !1, secs)?;
    out["decode_proxy_ms_per_frame"] = json!(px_play);
    out["seek_proxy_ms"] = json!(px_seek);
    out["decode_speedup"] = json!(orig_play / px_play.max(0.001));

    // Export 1080p with the default look (not a straight copy).
    let mut p = new_project(&src, None)?;
    p.export.quality = 1080;
    let mut runs = serde_json::Map::new();
    let mut encs = vec![Encoder::X264];
    if hw != Encoder::X264 {
        encs.insert(0, hw);
    }
    for e in encs {
        let o = dir.join(format!("out-{}.mp4", e.name()));
        let t = Instant::now();
        match export::export_with(&p, &o, e, &mut |_| {}) {
            Ok(s) => {
                let el = t.elapsed().as_secs_f64();
                runs.insert(e.name().into(), json!({ "seconds": el, "fps": s.frames as f64 / el, "realtime_x": secs / el, "size": format!("{}x{}", s.width, s.height) }));
            }
            Err(err) => {
                runs.insert(e.name().into(), json!({ "error": format!("{err:#}") }));
            }
        }
    }
    out["export_1080p"] = serde_json::Value::Object(runs);
    out["make_source_s"] = json!(make_s);
    let _ = std::fs::remove_dir_all(&dir);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("DemoLens bench: {} on encoder {}", out["source"], out["encoder"]);
        println!("  decode a preview frame, original: {:.1} ms (seek {:.0} ms)", orig_play, orig_seek);
        println!("  decode a preview frame, 720p proxy: {:.1} ms (seek {:.0} ms), {:.1}x faster", px_play, px_seek, orig_play / px_play.max(0.001));
        println!("  proxy build: {:.1} s with {}", st.seconds, st.encoder);
        println!("  export 1080p: {}", serde_json::to_string(&out["export_1080p"])?);
    }
    Ok(())
}
