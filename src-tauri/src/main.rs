//! DemoLens desktop app. The web UI is only a control surface: every preview
//! frame is rendered by `demolens_core::render::Renderer`, the same code the
//! exporter runs, and shipped to the UI as a PNG.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use demolens_core::capture::{self, CaptureTarget, RecordOptions, RecordingSession};
use demolens_core::preset::{self, Preset};
use demolens_core::project::Project;
use demolens_core::{export, new_project, zoom, EventLog, ZoomSegment};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, State};

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<RecordingSession>>,
}

type R<T> = Result<T, String>;
fn e<E: std::fmt::Display>(err: E) -> String {
    format!("{err:#}")
}

#[tauri::command]
fn open_recording(video: String, events: Option<String>) -> R<Project> {
    new_project(&PathBuf::from(video), events.filter(|s| !s.is_empty()).map(PathBuf::from).as_deref()).map_err(e)
}

#[tauri::command]
fn load_project(path: String) -> R<Project> {
    Project::load(&PathBuf::from(path)).map_err(e)
}

#[tauri::command]
fn save_project(path: String, project: Project) -> R<()> {
    project.save(&PathBuf::from(path)).map_err(e)
}

#[tauri::command]
fn regenerate_zoom(project: Project) -> R<Vec<ZoomSegment>> {
    let Some(ev) = &project.events else { return Ok(vec![]) };
    let log = EventLog::load(ev).map_err(e)?;
    Ok(zoom::generate_zoom_segments(&log, project.duration_ms, &zoom::ZoomConfig::default()))
}

/// Render one preview frame at output time `out_ms`. Returns a data URL.
#[tauri::command]
async fn preview(project: Project, out_ms: f64, short_side: u32) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (w, h) = project.export.aspect.size(short_side);
        let png = export::preview_png(&project, out_ms, w, h).map_err(e)?;
        Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
    })
    .await
    .map_err(e)?
}

#[tauri::command]
async fn export_video(app: tauri::AppHandle, project: Project, out: String) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = export::export(&project, &PathBuf::from(&out), |p| {
            let _ = app.emit("export-progress", p);
        })
        .map_err(e)?;
        Ok(format!("{out} ({}x{}, {:.1}s)", st.width, st.height, st.seconds))
    })
    .await
    .map_err(e)?
}

/// Project or video passed on the command line, e.g. `demolens-app demo.demolens.json`.
#[tauri::command]
fn initial_project() -> R<Option<Project>> {
    let Some(arg) = std::env::args().nth(1) else { return Ok(None) };
    let path = PathBuf::from(arg);
    if path.extension().is_some_and(|e| e == "json") {
        Project::load(&path).map(Some).map_err(e)
    } else {
        new_project(&path, None).map(Some).map_err(e)
    }
}

#[tauri::command]
fn list_presets() -> Vec<String> {
    preset::list_presets()
}

#[tauri::command]
fn save_preset(name: String, project: Project) -> R<()> {
    preset::save_preset(&Preset::from_project(&name, &project)).map(|_| ()).map_err(e)
}

#[tauri::command]
fn apply_preset(name: String, mut project: Project) -> R<Project> {
    preset::load_preset(&name).map_err(e)?.apply(&mut project);
    Ok(project)
}

#[tauri::command]
fn start_recording(state: State<AppState>, target: CaptureTarget, mic: bool, system_audio: bool, out_dir: String) -> R<Vec<String>> {
    let mut slot = state.recording.lock().unwrap();
    if slot.is_some() {
        return Err("already recording".into());
    }
    let s = capture::start(&RecordOptions { target, fps: 30, mic, system_audio, out_dir: out_dir.into() }).map_err(e)?;
    let warnings = s.warnings.clone();
    *slot = Some(s);
    Ok(warnings)
}

#[tauri::command]
fn stop_recording(state: State<AppState>) -> R<Project> {
    let s = state.recording.lock().unwrap().take().ok_or("not recording")?;
    let rec = s.stop().map_err(e)?;
    new_project(&rec.video, Some(&rec.events)).map_err(e)
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_recording,
            initial_project,
            load_project,
            save_project,
            regenerate_zoom,
            preview,
            export_video,
            list_presets,
            save_preset,
            apply_preset,
            start_recording,
            stop_recording
        ])
        .run(tauri::generate_context!())
        .expect("error while running DemoLens");
}
