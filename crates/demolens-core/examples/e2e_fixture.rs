//! Builds everything the browser tests need, from the real pipeline: a sample
//! recording with events (and speech if espeak-ng is installed), the project
//! file the editor would open, and styled preview frames from the real
//! renderer, so the UI can be driven without the desktop shell.
//!
//!   cargo run -p demolens-core --example e2e_fixture -- <dir> [speech]
use demolens_core::project::{Caption, CaptionWord, Project};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(a.get(1).expect("dir"));
    let speech = a.get(2).map(|s| s == "speech").unwrap_or(false);
    let path = demolens_core::sample::make(&dir, speech, false).expect("sample");
    let mut p = Project::load(&path).expect("load");
    if p.zoom.is_empty() {
        if let Some(ev) = &p.events {
            let log = demolens_core::events::EventLog::load(ev).expect("events");
            p.zoom = demolens_core::zoom::generate_zoom_segments(&log, p.duration_ms, &p.zoom_config.to_config());
        }
    }
    if p.captions.is_empty() {
        let w = |t: &str, s: f64| CaptionWord { text: t.into(), start_ms: s, end_ms: s + 300.0 };
        p.captions = vec![
            Caption { start_ms: 1000.0, end_ms: 3200.0, text: "This is the sample project".into(), words: vec![w("This", 1000.0), w("is", 1300.0), w("the", 1600.0), w("sample", 1900.0), w("project", 2500.0)] },
            Caption { start_ms: 5000.0, end_ms: 7400.0, text: "Click save to keep your work".into(), words: vec![] },
        ];
    }
    p.save(&dir.join("e2e.demolens")).expect("save");
    let json = serde_json::to_string(&p).expect("json");
    std::fs::write(dir.join("project.json"), json).expect("write");
    let frames = dir.join("frames");
    std::fs::create_dir_all(&frames).expect("frames");
    let (w, h) = (960u32, 540u32);
    let r = demolens_core::export::make_renderer(&p, w, h).expect("renderer");
    let (dw, dh) = r.decode_size();
    let mut dec = demolens_core::ffmpeg::PreviewDecoder::new(&p.source, dw, dh);
    let mut t = 0.0;
    while t < p.duration_ms - 40.0 {
        let src = dec.frame(t, false).expect("frame");
        let f = r.render(&src, t);
        std::fs::write(frames.join(format!("f_{}.png", t as u64)), f.encode_png().expect("png")).expect("write");
        t += 500.0;
    }
    println!("{}", dir.join("project.json").display());
}
