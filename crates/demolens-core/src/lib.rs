//! DemoLens core: event log, auto zoom, compositor, export and capture.

pub mod capture;
pub mod events;
pub mod export;
pub mod ffmpeg;
pub mod font;
pub mod input;
pub mod preset;
pub mod project;
pub mod render;
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
    Ok(p)
}
