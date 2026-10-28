//! Draws the three device frames round a test picture, for a quick look:
//! `cargo run -p demolens-core --example frames_preview -- out_dir`
use demolens_core::project::{DeviceFrame, Project};
use demolens_core::render::{Frame, Renderer};

fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir)?;
    let mut src = Frame::new(1920, 1080);
    for (i, px) in src.data.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i % 1920) as f32 / 1920.0, (i / 1920) as f32 / 1080.0);
        px.copy_from_slice(&[(40.0 + 160.0 * x) as u8, (60.0 + 120.0 * y) as u8, 200, 255]);
    }
    for (name, dev) in [("laptop", DeviceFrame::Laptop), ("browser", DeviceFrame::Browser), ("phone", DeviceFrame::Phone)] {
        let mut p = Project::new("a.mp4".into(), None, 1920, 1080, 5000.0);
        p.style.frame = dev;
        let (w, h) = if dev == DeviceFrame::Phone { (720, 1280) } else { (1280, 720) };
        let r = Renderer::new(p, None, w, h, None);
        std::fs::write(dir.join(format!("frame-{name}.png")), r.render(&src, 1000.0, None).encode_png()?)?;
    }
    Ok(())
}
