//! DemoLens desktop app. The web UI is only a control surface: every preview
//! frame is rendered by `demolens_core::render::Renderer`, the same code the
//! exporter runs, and shipped to the UI as a PNG.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use demolens_ai::{docs, llm::Llm, models, pipeline};
use demolens_core::captions::{self, AutoCutOptions};
use demolens_core::render::Renderer;
use demolens_core::capture::{self, CaptureTarget, RecordOptions, RecordingSession};
use demolens_core::preset::{self, Preset};
use demolens_core::project::Project;
use demolens_core::{export, new_project, zoom, EventLog, ZoomSegment};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

enum RecSlot {
    Starting,
    Running(RecordingSession),
}

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<RecSlot>>,
    /// The renderer for the last previewed project and size. Building one
    /// loads the event log and background image, so scrubbing reuses it.
    renderer: Mutex<Option<(String, u32, u32, std::sync::Arc<Renderer>)>>,
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
async fn preview(app: tauri::AppHandle, project: Project, out_ms: f64, short_side: u32) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (w, h) = project.export.aspect.size(short_side);
        let key = serde_json::to_string(&project).map_err(e)?;
        let state = app.state::<AppState>();
        let cached = state.renderer.lock().unwrap().as_ref().filter(|c| c.0 == key && c.1 == w && c.2 == h).map(|c| c.3.clone());
        let r = match cached {
            Some(r) => r,
            None => {
                let r = std::sync::Arc::new(export::make_renderer(&project, w, h).map_err(e)?);
                *state.renderer.lock().unwrap() = Some((key, w, h, r.clone()));
                r
            }
        };
        let src_t = project.source_time(out_ms).unwrap_or(project.duration_ms);
        let (dw, dh) = r.decode_size();
        let src = demolens_core::ffmpeg::frame_at(&project.source, src_t, dw, dh).map_err(e)?;
        let png = r.render(&src, src_t).encode_png().map_err(e)?;
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
fn initial_project() -> R<Option<serde_json::Value>> {
    let Some(arg) = std::env::args().nth(1) else { return Ok(None) };
    let path = PathBuf::from(arg);
    if path.extension().is_some_and(|e| e == "json") {
        let p = Project::load(&path).map_err(e)?;
        Ok(Some(serde_json::json!({ "project": p, "path": path })))
    } else {
        let p = new_project(&path, None).map_err(e)?;
        Ok(Some(serde_json::json!({ "project": p, "path": null })))
    }
}

#[tauri::command]
fn list_presets() -> serde_json::Value {
    let builtin = preset::builtin_presets();
    let user: Vec<preset::Preset> = preset::list_presets().into_iter().filter_map(|n| preset::load_preset(&n).ok()).collect();
    serde_json::json!({ "builtin": builtin, "user": user })
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

// Recording commands are async and do their work on a blocking thread, so a
// slow or stuck ffmpeg can never freeze the window.
#[tauri::command]
async fn start_recording(app: tauri::AppHandle, target: CaptureTarget, mic: bool, system_audio: bool) -> R<Vec<String>> {
    {
        let state = app.state::<AppState>();
        let mut slot = state.recording.lock().unwrap();
        match &*slot {
            Some(RecSlot::Starting) => return Err("Recording is already starting".into()),
            Some(RecSlot::Running(_)) => return Err("Already recording".into()),
            None => *slot = Some(RecSlot::Starting),
        }
    }
    // Recordings go to Videos/DemoLens/<timestamp>, not the app's working
    // directory (which is read only inside a macOS app bundle).
    let base = app.path().video_dir().or_else(|_| app.path().home_dir()).map_err(e)?.join("DemoLens");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let out_dir = base.join(format!("recording-{stamp}"));
    let res = blocking(move || capture::start(&RecordOptions { target, fps: 30, mic, system_audio, out_dir }).map_err(e)).await;
    let state = app.state::<AppState>();
    let mut slot = state.recording.lock().unwrap();
    match res {
        Ok(s) => {
            let warnings = s.warnings.clone();
            *slot = Some(RecSlot::Running(s));
            Ok(warnings)
        }
        Err(err) => {
            *slot = None;
            Err(err)
        }
    }
}

#[tauri::command]
async fn stop_recording(app: tauri::AppHandle) -> R<Project> {
    let s = {
        let state = app.state::<AppState>();
        let mut slot = state.recording.lock().unwrap();
        match slot.take() {
            Some(RecSlot::Running(s)) => s,
            other => {
                *slot = other;
                return Err("Not recording".into());
            }
        }
    };
    blocking(move || {
        let rec = s.stop().map_err(e)?;
        new_project(&rec.video, Some(&rec.events)).map_err(e)
    })
    .await
}

// ---------- Local AI ----------

#[derive(Clone, serde::Serialize)]
struct AiProgress {
    task: String,
    progress: f64,
    message: String,
}

fn progress(app: &tauri::AppHandle, task: &str, p: f64, msg: &str) {
    let _ = app.emit("ai-progress", AiProgress { task: task.into(), progress: p, message: msg.into() });
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> R<T> + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(e)?
}

#[tauri::command]
fn models_list() -> serde_json::Value {
    serde_json::json!({ "dir": models::models_dir(), "models": models::list() })
}

#[tauri::command]
async fn model_download(app: tauri::AppHandle, id: String) -> R<()> {
    blocking(move || {
        let task = format!("download:{id}");
        models::download(&id, |d, t| progress(&app, &task, if t > 0 { d as f64 / t as f64 } else { 0.0 }, &format!("{} / {}", models::human(d), models::human(t))))
            .map(|_| ())
            .map_err(e)
    })
    .await
}

#[tauri::command]
fn model_delete(id: String) -> R<()> {
    models::delete(&id).map_err(e)
}

#[derive(serde::Serialize)]
struct AiResult<T> {
    project: Project,
    report: T,
}

#[tauri::command]
async fn ai_transcribe(app: tauri::AppHandle, mut project: Project, model: Option<String>) -> R<AiResult<pipeline::TranscribeReport>> {
    blocking(move || {
        progress(&app, "transcribe", 0.1, "Listening (runs on this computer)...");
        let report = pipeline::transcribe_project(&mut project, model.as_deref(), None).map_err(e)?;
        progress(&app, "transcribe", 1.0, "Done");
        Ok(AiResult { project, report })
    })
    .await
}

#[tauri::command]
async fn ai_autocut(mut project: Project, fillers: bool, silences: bool, min_silence_ms: f64) -> R<AiResult<pipeline::CutReport>> {
    blocking(move || {
        let o = AutoCutOptions { cut_fillers: fillers, cut_silences: silences, min_silence_ms, ..Default::default() };
        let report = pipeline::autocut_project(&mut project, &o, false).map_err(e)?;
        Ok(AiResult { project, report })
    })
    .await
}

#[tauri::command]
async fn ai_redact(app: tauri::AppHandle, mut project: Project) -> R<AiResult<pipeline::RedactReport>> {
    blocking(move || {
        let report = pipeline::redact_project(&mut project, 500.0, false, |p| progress(&app, "redact", p, "Scanning frames for sensitive text...")).map_err(e)?;
        Ok(AiResult { project, report })
    })
    .await
}

#[tauri::command]
async fn ai_docs(app: tauri::AppHandle, project: Project, use_llm: bool) -> R<serde_json::Value> {
    blocking(move || {
        progress(&app, "docs", 0.1, "Looking for a local LLM...");
        let llm = if use_llm { Llm::detect().map_err(e)? } else { None };
        progress(&app, "docs", 0.3, llm.as_ref().map(|l| format!("Writing with {}...", l.label)).as_deref().unwrap_or("Writing with the rule based writer..."));
        let log = project.events.as_ref().and_then(|p| EventLog::load(p).ok());
        let d = docs::generate(&project, log.as_ref(), llm.as_ref()).map_err(e)?;
        progress(&app, "docs", 1.0, "Done");
        Ok(serde_json::json!({ "docs": d, "markdown": docs::markdown(&d), "chapters": docs::chapters_text(&d) }))
    })
    .await
}

/// Build the sample recording in the app's data folder and open it.
#[tauri::command]
async fn make_sample(app: tauri::AppHandle) -> R<serde_json::Value> {
    let dir = app.path().app_data_dir().map_err(e)?.join("sample");
    blocking(move || {
        let speech = demolens_core::sample::can_speak();
        let proj = demolens_core::sample::make(&dir, speech, true).or_else(|_| demolens_core::sample::make(&dir, false, false)).map_err(e)?;
        let project = Project::load(&proj).map_err(e)?;
        Ok(serde_json::json!({ "project": project, "path": proj }))
    })
    .await
}

#[tauri::command]
async fn ai_demo_ready(app: tauri::AppHandle, mut project: Project, zoom: bool, captions: bool, cut: bool, blur: bool, denoise: bool) -> R<AiResult<pipeline::ReadyReport>> {
    blocking(move || {
        let o = pipeline::ReadyOptions { zoom, captions, cut, blur, denoise, ..Default::default() };
        let report = pipeline::demo_ready(&mut project, &o, |p, m| progress(&app, "ready", p, m));
        Ok(AiResult { project, report })
    })
    .await
}

/// Show a file in the system file manager.
#[tauri::command]
fn reveal(path: String) -> R<()> {
    let p = std::path::PathBuf::from(&path);
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(&p).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(&p)).spawn();
    r.map(|_| ()).map_err(e)
}

#[tauri::command]
fn export_captions(project: Project, path: String) -> R<()> {
    let text = if path.to_lowercase().ends_with(".vtt") { captions::to_vtt(&project) } else { captions::to_srt(&project) };
    std::fs::write(path, text).map_err(e)
}

#[tauri::command]
fn write_text(path: String, text: String) -> R<()> {
    std::fs::write(path, text).map_err(e)
}

/// Where ffmpeg was found (bundled, PATH or a common folder), its version,
/// and the one line install command for this OS when it is missing.
#[tauri::command]
fn ffmpeg_status() -> serde_json::Value {
    let bin = demolens_core::ffmpeg::ffmpeg_bin();
    let found = demolens_core::ffmpeg::ffmpeg_found();
    let version = if found {
        std::process::Command::new(&bin)
            .arg("-version")
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(|l| l.to_string()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| std::path::Path::new(&bin).starts_with(d)))
        .unwrap_or(false);
    serde_json::json!({
        "found": found, "path": bin, "version": version, "bundled": bundled,
        "hint": demolens_core::ffmpeg::install_hint(),
        "app_version": env!("CARGO_PKG_VERSION"),
    })
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            stop_recording,
            models_list,
            model_download,
            model_delete,
            ai_transcribe,
            ai_autocut,
            ai_redact,
            ai_docs,
            export_captions,
            write_text,
            make_sample,
            ai_demo_ready,
            reveal,
            ffmpeg_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running DemoLens");
}
