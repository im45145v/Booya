//! Render one export frame through the same compositor the preview and the
//! export use, so the glass frame can be looked at without a screen recording.
//!
//!   cargo run -p demolens-core --example glass_demo -- in.png out.png [preset] [flat]
//!
//! `in.png` stands in for one frame of a recording. `preset` is a built-in
//! preset name (default "Frosted"). Add `flat` to render without glass.
use demolens_core::project::{Callout, Project};
use demolens_core::render::{Frame, Renderer};

fn read_png(path: &str) -> Frame {
    let dec = png::Decoder::new(std::fs::File::open(path).expect("open input"));
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).expect("frame");
    let ch = info.color_type.samples();
    let mut f = Frame::new(info.width, info.height);
    for (i, px) in f.data.chunks_exact_mut(4).enumerate() {
        let s = &buf[i * ch..i * ch + ch];
        px.copy_from_slice(&[s[0], if ch > 2 { s[1] } else { s[0] }, if ch > 2 { s[2] } else { s[0] }, 255]);
    }
    f
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (inp, out) = (&a[1], &a[2]);
    let name = a.get(3).map(String::as_str).unwrap_or("Frosted");
    let flat = a.get(4).map(|s| s == "flat").unwrap_or(false);
    let src = read_png(inp);
    let mut p = Project::new("demo.mp4".into(), None, src.w, src.h, 6000.0);
    let preset = demolens_core::preset::builtin_presets().into_iter().find(|p| p.name == name).expect("preset name");
    preset.apply(&mut p);
    if flat {
        p.style.glass.enabled = false;
    }
    p.callouts.push(Callout { text: "Frosted notes match the frame".into(), x: 0.5, y: 0.9, start_ms: 0.0, end_ms: 6000.0 });
    let r = Renderer::new(p, None, 1600, 900, None);
    let frame = r.render(&src, 1000.0);
    std::fs::write(out, frame.encode_png().expect("png")).expect("write");
    println!("wrote {out}");
}
