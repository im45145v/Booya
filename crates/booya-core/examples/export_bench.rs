//! Times an export of the built-in sample, so export speed can be compared
//! before and after a change without the desktop app.
//!
//! cargo run --release -p booya-core --example export_bench -- [dir] [quality] [webcam]

use booya_core::{export, project::Project};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| std::env::temp_dir().join("booya-bench").display().to_string()));
    std::fs::create_dir_all(&dir)?;
    let pp = booya_core::sample::make(&dir, false, false)?;
    let mut p = Project::load(&pp)?;
    if let Some(q) = args.get(2) {
        p.export.quality = q.parse()?;
    }
    if std::env::var("BOOYA_BENCH").as_deref() == Ok("render") {
        return render_only(&p, p.export.quality);
    }
    let out = dir.join("bench.mp4");
    let t0 = std::time::Instant::now();
    let st = export::export(&p, &out, |_| {})?;
    let secs = t0.elapsed().as_secs_f64();
    println!("{} frames at {}x{} in {:.2}s ({:.1} fps)", st.frames, st.width, st.height, secs, st.frames as f64 / secs);
    Ok(())
}

/// Render only (no decode or encode): `BOOYA_BENCH=render`.
#[allow(dead_code)]
fn render_only(p: &Project, q: u32) -> anyhow::Result<()> {
    let (ow, oh) = p.export.aspect.size(q);
    let r = export::make_renderer(p, ow, oh)?;
    let (dw, dh) = r.decode_size();
    let mut rd = booya_core::ffmpeg::FrameReader::open(&p.source, 0.0, 30.0, dw, dh)?;
    let mut frames = Vec::new();
    while let Some(f) = rd.next()? {
        frames.push(f);
        if frames.len() >= 90 {
            break;
        }
    }
    let t0 = std::time::Instant::now();
    let mut sum = 0u64;
    for (i, f) in frames.iter().enumerate() {
        let out = r.render(f, i as f64 * 1000.0 / 30.0 + 3000.0, None);
        // A cheap checksum, so two builds can be checked for identical output.
        sum = out.data.iter().fold(sum, |h, &b| h.wrapping_mul(31).wrapping_add(b as u64));
    }
    println!("checksum {sum:016x}");
    let s = t0.elapsed().as_secs_f64();
    println!("render only: {:.1} ms per {}x{} frame", s * 1000.0 / frames.len() as f64, ow, oh);
    Ok(())
}
