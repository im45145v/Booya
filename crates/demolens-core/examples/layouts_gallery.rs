//! Renders every layout in every shape from the real renderer and the real
//! exporter, so they can be looked at (and checked by CI).
//!
//!   cargo run -p demolens-core --example layouts_gallery -- <dir>
//!
//! Writes `<dir>/shots/layout-<kind>-<aspect>.png`, and for each shape a
//! mixed-layout video `<dir>/mixed-<aspect>.mp4` with frames taken from it
//! (`<dir>/shots/export-<aspect>-<n>.png`).
use demolens_core::project::{Aspect, Project, SceneKind, SceneSegment, Webcam};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(a.get(1).expect("dir"));
    let path = demolens_core::sample::make(&dir, false, false).expect("sample");
    let mut p = Project::load(&path).expect("load");
    // A stand in for a camera: moving test pattern.
    let cam = dir.join("camera.mp4");
    let ok = demolens_core::proc::command("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=640x480:rate=30", "-t", &format!("{:.1}", p.duration_ms / 1000.0 + 1.0), "-pix_fmt", "yuv420p", "-c:v", "libx264"])
        .arg(&cam)
        .status()
        .expect("ffmpeg")
        .success();
    assert!(ok, "camera clip");
    p.webcam = Some(Webcam::new(cam));
    let shots = dir.join("shots");
    std::fs::create_dir_all(&shots).expect("shots");
    let t = (p.duration_ms * 0.4).round();
    for aspect in [Aspect::Landscape, Aspect::Portrait, Aspect::Square] {
        let name = format!("{aspect:?}").to_lowercase();
        let (w, h) = aspect.size(720);
        for kind in SceneKind::ALL {
            p.layouts.base = Some(kind);
            p.export.aspect = aspect;
            let png = demolens_core::export::preview_png(&p, t, w, h).expect("preview");
            std::fs::write(shots.join(format!("layout-{}-{name}.png", kind.name())), png).expect("write");
        }
        // One video that goes through all four layouts, with blends.
        p.layouts.base = Some(SceneKind::Screen);
        let d = p.duration_ms;
        p.layouts.segments = vec![
            SceneSegment { start_ms: d * 0.25, end_ms: d * 0.5, kind: SceneKind::Bubble },
            SceneSegment { start_ms: d * 0.5, end_ms: d * 0.75, kind: SceneKind::Split },
            SceneSegment { start_ms: d * 0.75, end_ms: d, kind: SceneKind::Camera },
        ];
        let out = dir.join(format!("mixed-{name}.mp4"));
        demolens_core::export::export(&p, &out, |_| {}).expect("export");
        for (i, f) in [0.1, 0.35, 0.6, 0.9].iter().enumerate() {
            let at = format!("{:.2}", d * f / 1000.0);
            let ok = demolens_core::proc::command("ffmpeg").args(["-y", "-v", "error", "-ss", &at, "-i"]).arg(&out).args(["-frames:v", "1"]).arg(shots.join(format!("export-{name}-{i}.png"))).status().expect("ffmpeg").success();
            assert!(ok, "frame");
        }
        p.layouts.segments.clear();
        println!("{name}: ok");
    }
}
