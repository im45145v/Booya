//! DemoLens core: event log, auto zoom, compositor, export and capture.

pub mod captions;
pub mod capture;
pub mod events;
pub mod export;
pub mod finalize;
pub mod ffmpeg;
pub mod activity;
pub mod audio;
pub mod text;
pub mod input;
pub mod keylabels;
pub mod preset;
pub mod project;
pub mod proxy;
pub mod render;
pub mod sample;
pub mod shortcuts;
pub mod storage;
pub mod bundle;
pub mod proc;
pub mod touch;
pub mod splice;
pub mod syncprobe;
pub mod zoom;

pub use events::{Event, EventKind, EventLog};
pub use project::Project;
pub use zoom::{generate_zoom_segments, ZoomConfig, ZoomSegment};

/// Create a project for a recording: probe the video, load the event log if
/// present and generate auto zoom segments.
pub fn new_project(video: &std::path::Path, events: Option<&std::path::Path>) -> anyhow::Result<Project> {
    let info = ffmpeg::probe(video)?;
    let events = events.map(|p| p.to_path_buf()).or_else(|| {
        let guess = video.with_extension("events.jsonl");
        guess.exists().then_some(guess)
    });
    let mut p = Project::new(video.to_path_buf(), events.clone(), info.width, info.height, info.duration_ms);
    if let Some(ev) = &events {
        let log = EventLog::load(ev)?;
        p.zoom = generate_zoom_segments(&log, info.duration_ms, &ZoomConfig::default());
    }
    // Stretches paused during recording are removed.
    if let Some(pauses) = std::fs::read_to_string(video.with_extension("pauses.json")).ok().and_then(|s| serde_json::from_str::<Vec<(f64, f64)>>(&s).ok()) {
        p.cut_ranges(&pauses);
    }
    // A webcam recorded with the screen sits next to it as webcam.mkv.
    if let Some(cam) = video.parent().map(|d| d.join("webcam.mkv")).filter(|c| c.exists() && c.as_path() != video) {
        let mut w = project::Webcam::new(cam.clone());
        w.offset_ms = std::fs::read_to_string(cam.with_extension("offset")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0.0);
        p.webcam = Some(w);
    }
    Ok(p)
}
