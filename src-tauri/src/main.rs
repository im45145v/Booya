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
use demolens_ai::providers::{self, CloudConfig, Provider, Task};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

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
    /// One ffmpeg decoder kept open while the preview plays.
    decoder: Mutex<Option<demolens_core::ffmpeg::PreviewDecoder>>,
    /// Files the user picked in a native dialog this session. Commands that
    /// write files only write to these paths, so a compromised web view
    /// cannot write anywhere it likes.
    granted: Mutex<HashSet<PathBuf>>,
}

fn grant(app: &tauri::AppHandle, p: &Path) {
    app.state::<AppState>().granted.lock().unwrap().insert(p.to_path_buf());
}

/// Allow a write only to a path picked in a dialog, with an allowed extension.
fn check_write(app: &tauri::AppHandle, path: &str, exts: &[&str]) -> R<PathBuf> {
    let p = PathBuf::from(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !exts.contains(&ext.as_str()) {
        return Err(format!("DemoLens only writes {} files here", exts.join(", ")));
    }
    if !app.state::<AppState>().granted.lock().unwrap().contains(&p) {
        return Err("Pick the file with the Save dialog first".into());
    }
    Ok(p)
}

/// Native save dialog run from Rust. The chosen path is remembered so the
/// matching write command accepts it.
#[tauri::command]
async fn pick_save(app: tauri::AppHandle, default_path: Option<String>, name: String, extensions: Vec<String>) -> R<Option<String>> {
    let a2 = app.clone();
    blocking(move || {
        let exts: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
        let mut d = a2.dialog().file().add_filter(&name, &exts);
        if let Some(dp) = default_path.filter(|d| !d.is_empty()) {
            let dp = PathBuf::from(dp);
            if let Some(dir) = dp.parent().filter(|d| d.is_dir()) {
                d = d.set_directory(dir);
            }
            if let Some(f) = dp.file_name() {
                d = d.set_file_name(f.to_string_lossy());
            }
        }
        let Some(fp) = d.blocking_save_file() else { return Ok(None) };
        let mut p = fp.into_path().map_err(e)?;
        if p.extension().is_none() {
            if let Some(x) = extensions.first() {
                p.set_extension(x);
            }
        }
        grant(&a2, &p);
        Ok(Some(p.to_string_lossy().to_string()))
    })
    .await
}

/// Native open dialog run from Rust.
#[tauri::command]
async fn pick_open(app: tauri::AppHandle, name: String, extensions: Vec<String>) -> R<Option<String>> {
    let a2 = app.clone();
    blocking(move || {
        let exts: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
        let Some(fp) = a2.dialog().file().add_filter(&name, &exts).blocking_pick_file() else { return Ok(None) };
        let p = fp.into_path().map_err(e)?;
        grant(&a2, &p);
        Ok(Some(p.to_string_lossy().to_string()))
    })
    .await
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
fn load_project(app: tauri::AppHandle, path: String) -> R<Project> {
    let p = PathBuf::from(path);
    let pr = Project::load(&p).map_err(e)?;
    // Saving back over the project the user opened is allowed.
    grant(&app, &p);
    Ok(pr)
}

#[tauri::command]
fn save_project(app: tauri::AppHandle, path: String, project: Project) -> R<()> {
    let p = check_write(&app, &path, &["json"])?;
    project.save(&p).map_err(e)
}

#[tauri::command]
fn regenerate_zoom(project: Project) -> R<Vec<ZoomSegment>> {
    let Some(ev) = &project.events else { return Ok(vec![]) };
    let log = EventLog::load(ev).map_err(e)?;
    Ok(zoom::generate_zoom_segments(&log, project.duration_ms, &zoom::ZoomConfig::default()))
}

/// Render one preview frame at output time `out_ms`. Returns a data URL.
#[tauri::command]
async fn preview(app: tauri::AppHandle, project: Project, out_ms: f64, short_side: u32, playing: Option<bool>) -> R<String> {
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
        let src = {
            let mut dec = state.decoder.lock().unwrap();
            if !dec.as_ref().is_some_and(|d| d.matches(&project.source, dw, dh)) {
                *dec = Some(demolens_core::ffmpeg::PreviewDecoder::new(&project.source, dw, dh));
            }
            dec.as_mut().unwrap().frame(src_t, playing.unwrap_or(false)).map_err(e)?
        };
        let png = r.render(&src, src_t).encode_png().map_err(e)?;
        Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
    })
    .await
    .map_err(e)?
}

#[tauri::command]
async fn export_video(app: tauri::AppHandle, project: Project, out: String) -> R<String> {
    check_write(&app, &out, &["mp4", "gif", "webm"])?;
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
fn initial_project(app: tauri::AppHandle) -> R<Option<serde_json::Value>> {
    let Some(arg) = std::env::args().nth(1) else { return Ok(None) };
    let path = PathBuf::from(arg);
    if path.extension().is_some_and(|e| e == "json") {
        let p = Project::load(&path).map_err(e)?;
        grant(&app, &path);
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
async fn start_recording(app: tauri::AppHandle, target: CaptureTarget, mic: bool, system_audio: bool, keys: Option<demolens_core::input::KeyLogging>, webcam: Option<bool>) -> R<Vec<String>> {
    let keys = keys.unwrap_or_default();
    let webcam = webcam.unwrap_or(false);
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
    let res = blocking(move || capture::start(&RecordOptions { target, fps: 30, mic, system_audio, out_dir, keys, webcam }).map_err(e)).await;
    let state = app.state::<AppState>();
    let mut slot = state.recording.lock().unwrap();
    match res {
        Ok(s) => {
            let warnings = s.warnings.clone();
            *slot = Some(RecSlot::Running(s));
            drop(slot);
            show_indicator(&app);
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
    hide_indicator(&app);
    blocking(move || {
        let rec = s.stop().map_err(e)?;
        new_project(&rec.video, Some(&rec.events)).map_err(e)
    })
    .await
}

/// The untouched source frame at `src_ms`, for drawing blur boxes in
/// source pixels.
#[tauri::command]
async fn raw_frame(project: Project, src_ms: f64) -> R<serde_json::Value> {
    blocking(move || {
        let (sw, sh) = (project.source_width, project.source_height);
        let scale = (1280.0 / sw.max(1) as f64).min(1.0);
        let (w, h) = (((sw as f64 * scale) as u32).max(2) & !1, ((sh as f64 * scale) as u32).max(2) & !1);
        let f = demolens_core::ffmpeg::frame_at(&project.source, src_ms, w, h).map_err(e)?;
        let png = f.encode_png().map_err(e)?;
        Ok(serde_json::json!({ "url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)), "w": sw, "h": sh }))
    })
    .await
}

/// Screenshot for the visual area picker (data URL plus capture size).
#[tauri::command]
async fn screen_snapshot() -> R<serde_json::Value> {
    blocking(|| {
        let (png, w, h) = capture::screenshot().map_err(e)?;
        Ok(serde_json::json!({ "url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)), "w": w, "h": h }))
    })
    .await
}

/// Pause or resume. While paused the key hook process is stopped and the
/// paused stretch is cut from the video automatically.
#[tauri::command]
fn pause_recording(app: tauri::AppHandle, paused: Option<bool>) -> R<bool> {
    let state = app.state::<AppState>();
    let mut slot = state.recording.lock().unwrap();
    let Some(RecSlot::Running(s)) = &mut *slot else { return Err("Not recording".into()) };
    let want = paused.unwrap_or(!s.is_paused());
    s.set_paused(want);
    let _ = app.emit("rec-paused", want);
    Ok(want)
}

/// A small always on top "recording" badge. It is content protected, so
/// macOS and Windows leave it out of the recording.
fn show_indicator(app: &tauri::AppHandle) {
    if app.get_webview_window("indicator").is_some() {
        return;
    }
    let w = tauri::WebviewWindowBuilder::new(app, "indicator", tauri::WebviewUrl::App("indicator.html".into()))
        .title("DemoLens is recording")
        .inner_size(230.0, 44.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .content_protected(true)
        .build();
    if let Ok(w) = w {
        if let Ok(Some(m)) = w.current_monitor() {
            let sz = m.size();
            let sf = m.scale_factor();
            let _ = w.set_position(tauri::PhysicalPosition::new(sz.width as i32 - (250.0 * sf) as i32, (16.0 * sf) as i32));
        }
    }
}

fn hide_indicator(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("indicator") {
        let _ = w.close();
    }
}

fn recordings_dir(app: &tauri::AppHandle) -> R<PathBuf> {
    Ok(app.path().video_dir().or_else(|_| app.path().home_dir()).map_err(e)?.join("DemoLens"))
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
#[serde(default)]
struct Settings {
    /// Delete recordings older than this many days at startup. None keeps them.
    retention_days: Option<u32>,
}

fn settings_path() -> PathBuf {
    demolens_core::preset::presets_dir().parent().map(|p| p.join("settings.json")).unwrap_or_else(|| "settings.json".into())
}

fn load_settings() -> Settings {
    std::fs::read_to_string(settings_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

#[tauri::command]
fn get_settings() -> Settings {
    load_settings()
}

#[tauri::command]
fn set_retention(app: tauri::AppHandle, days: Option<u32>) -> R<usize> {
    let mut st = load_settings();
    st.retention_days = days.filter(|d| *d > 0);
    let p = settings_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(e)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(&st).map_err(e)?).map_err(e)?;
    Ok(apply_retention(&app))
}

/// Remove recording folders older than the retention setting.
fn apply_retention(app: &tauri::AppHandle) -> usize {
    let Some(days) = load_settings().retention_days else { return 0 };
    let Ok(dir) = recordings_dir(app) else { return 0 };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(days as u64 * 86400);
    let mut n = 0;
    for ent in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = ent.path();
        let is_rec = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("recording-"));
        let old = ent.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
        if p.is_dir() && is_rec && old && std::fs::remove_dir_all(&p).is_ok() {
            n += 1;
        }
    }
    n
}

#[derive(serde::Serialize)]
struct RecordingInfo {
    path: PathBuf,
    bytes: u64,
    modified: u64,
}

#[tauri::command]
fn list_recordings(app: tauri::AppHandle) -> R<Vec<RecordingInfo>> {
    let dir = recordings_dir(&app)?;
    let mut out = Vec::new();
    for ent in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = ent.path();
        if !p.is_dir() || !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("recording-")) {
            continue;
        }
        let bytes = std::fs::read_dir(&p).into_iter().flatten().flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum();
        let modified = ent.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.push(RecordingInfo { path: p, bytes, modified });
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.modified));
    Ok(out)
}

/// Delete one recording folder (video, audio, event log). Only folders
/// inside the DemoLens recordings directory can be deleted.
#[tauri::command]
fn delete_recording(app: tauri::AppHandle, path: String) -> R<()> {
    let root = recordings_dir(&app)?.canonicalize().map_err(e)?;
    let p = PathBuf::from(&path);
    let p = if p.is_file() { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) } else { p };
    let c = p.canonicalize().map_err(e)?;
    if c.parent() != Some(root.as_path()) || !c.file_name().is_some_and(|n| n.to_string_lossy().starts_with("recording-")) {
        return Err("Only DemoLens recording folders can be deleted here".into());
    }
    std::fs::remove_dir_all(&c).map_err(e)
}

/// "Delete my data": every recording, the transcript cache and the sample.
/// With `include_keys`, also the saved API keys and cloud settings.
/// Downloaded models and projects saved elsewhere are kept.
#[tauri::command]
fn delete_all_data(app: tauri::AppHandle, include_keys: bool) -> R<usize> {
    let mut n = 0;
    for r in list_recordings(app.clone())? {
        if std::fs::remove_dir_all(&r.path).is_ok() {
            n += 1;
        }
    }
    demolens_ai::transcribe::clear_cache().map_err(e)?;
    let _ = std::fs::remove_dir_all(app.path().app_data_dir().map_err(e)?.join("sample"));
    if include_keys {
        for p in providers::ALL {
            if let Ok(k) = keyring_entry(p.id()) {
                let _ = k.delete_credential();
            }
        }
        if let Ok(k) = keyring_entry("s3-secret") {
            let _ = k.delete_credential();
        }
        let _ = std::fs::remove_file(providers::config_path());
        let _ = std::fs::remove_file(s3_path());
    }
    Ok(n)
}

// ---------- Cloud providers (opt in) ----------

fn keyring_entry(id: &str) -> R<keyring::Entry> {
    keyring::Entry::new("dev.demolens.app", id).map_err(e)
}

fn get_key(id: &str) -> Option<String> {
    keyring_entry(id).ok().and_then(|k| k.get_password().ok())
}

fn mask(k: &str) -> String {
    let n = k.chars().count();
    if n <= 8 {
        "*".repeat(n)
    } else {
        format!("{}****{}", k.chars().take(3).collect::<String>(), k.chars().skip(n - 4).collect::<String>())
    }
}

/// Providers, whether a key is stored (masked, never the key itself),
/// consents with dates and the current choice per task.
#[tauri::command]
fn cloud_status() -> serde_json::Value {
    let cfg = CloudConfig::load();
    let list: Vec<serde_json::Value> = providers::ALL
        .iter()
        .map(|p| {
            let key = get_key(p.id());
            let consents: std::collections::BTreeMap<&str, String> = [Task::Stt, Task::Llm, Task::Tts].iter().filter_map(|t| cfg.consent_date(*p, *t).map(|d| (t.id(), d.clone()))).collect();
            serde_json::json!({
                "id": p.id(),
                "info": p.info(),
                "key_set": key.is_some(),
                "key_masked": key.as_deref().map(mask),
                "consents": consents,
            })
        })
        .collect();
    serde_json::json!({ "providers": list, "choice": cfg.choice.iter().map(|(k, v)| (k.clone(), v.id())).collect::<std::collections::BTreeMap<_, _>>(), "custom_base": cfg.custom_base, "models": cfg.models, "voice": cfg.voice })
}

#[tauri::command]
fn cloud_set_key(provider: String, key: String) -> R<String> {
    let p = Provider::parse(&provider).map_err(e)?;
    let key = key.trim();
    if key.is_empty() {
        return Err("Empty key".into());
    }
    keyring_entry(p.id())?.set_password(key).map_err(|err| format!("Could not save the key in the system keychain: {err}"))?;
    Ok(mask(key))
}

#[tauri::command]
fn cloud_remove_key(provider: String) -> R<()> {
    let p = Provider::parse(&provider).map_err(e)?;
    match keyring_entry(p.id())?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(e(err)),
    }
}

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Turn a provider on (records today's date) or off for one task.
#[tauri::command]
fn cloud_consent(provider: String, task: String, accepted: bool) -> R<()> {
    let p = Provider::parse(&provider).map_err(e)?;
    let t = Task::parse(&task).map_err(e)?;
    let mut cfg = CloudConfig::load();
    if accepted {
        cfg.consent(p, t, &today()).map_err(e)?;
    } else {
        cfg.revoke(p, t);
    }
    cfg.save().map_err(e)
}

#[tauri::command]
fn cloud_configure(custom_base: Option<String>, models: Option<std::collections::BTreeMap<String, String>>, voice: Option<String>) -> R<()> {
    let mut cfg = CloudConfig::load();
    cfg.custom_base = custom_base.filter(|s| !s.trim().is_empty());
    if let Some(m) = models {
        cfg.models = m.into_iter().filter(|(_, v)| !v.trim().is_empty()).collect();
    }
    cfg.voice = voice.filter(|s| !s.trim().is_empty());
    if cfg.custom_base.is_some() {
        cfg.base(Provider::Custom).map_err(e)?;
    }
    cfg.save().map_err(e)
}

/// Exactly what a cloud call would send, for the confirm dialog.
#[tauri::command]
fn cloud_preview(task: String, project: Project, text: Option<String>) -> R<Option<providers::SendPreview>> {
    let t = Task::parse(&task).map_err(e)?;
    let cfg = CloudConfig::load();
    let Some(p) = cfg.active(t) else { return Ok(None) };
    let (data, bytes) = match t {
        Task::Stt => ("The audio track of this recording as 16 kHz mono WAV (no video, no event log)".to_string(), (project.duration_ms / 1000.0 * 32000.0) as u64 + 44),
        Task::Llm => {
            let words: usize = project.captions.iter().map(|c| c.text.split_whitespace().count()).sum();
            (format!("The transcript text ({words} words), chapter timings and the project file name. No video, audio or key log."), (words as u64) * 7 + 1500)
        }
        Task::Tts => {
            let tx = text.unwrap_or_default();
            (format!("The voiceover text ({} characters)", tx.chars().count()), tx.len() as u64)
        }
    };
    let mut pv = providers::preview(&cfg, p, t, &data, bytes).map_err(e)?;
    if get_key(p.id()).is_none() {
        pv.data = format!("{} (no API key saved for {} yet)", pv.data, pv.provider);
    }
    Ok(Some(pv))
}

fn cloud_key_for(t: Task) -> R<Option<(CloudConfig, Provider, String)>> {
    let cfg = CloudConfig::load();
    let Some(p) = cfg.active(t) else { return Ok(None) };
    let key = get_key(p.id()).ok_or_else(|| format!("No API key saved for {}. Add one in Settings > Cloud AI or switch back to local.", p.info().name))?;
    Ok(Some((cfg, p, key)))
}

/// Cloud text to speech of `text` into an MP3 the user picked.
#[tauri::command]
async fn cloud_tts(app: tauri::AppHandle, text: String, path: String) -> R<String> {
    let out = check_write(&app, &path, &["mp3"])?;
    blocking(move || {
        let (cfg, p, key) = cloud_key_for(Task::Tts)?.ok_or("Pick a text to speech provider in Settings > Cloud AI first")?;
        let mp3 = providers::speak(&cfg, p, &key, &text).map_err(e)?;
        std::fs::write(&out, mp3).map_err(e)?;
        Ok(out.to_string_lossy().to_string())
    })
    .await
}

fn s3_path() -> PathBuf {
    demolens_core::preset::presets_dir().parent().map(|p| p.join("storage.json")).unwrap_or_else(|| "storage.json".into())
}

#[tauri::command]
fn s3_get() -> serde_json::Value {
    let cfg: demolens_ai::s3::S3Config = std::fs::read_to_string(s3_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    serde_json::json!({ "config": cfg, "secret_set": get_key("s3-secret").is_some() })
}

#[tauri::command]
fn s3_set(config: demolens_ai::s3::S3Config, secret: Option<String>) -> R<()> {
    config.validate().map_err(e)?;
    std::fs::write(s3_path(), serde_json::to_string_pretty(&config).map_err(e)?).map_err(e)?;
    if let Some(sct) = secret.filter(|s| !s.trim().is_empty()) {
        keyring_entry("s3-secret")?.set_password(sct.trim()).map_err(e)?;
    }
    Ok(())
}

/// Upload an exported file (never the event log) and return a link that
/// expires.
#[tauri::command]
async fn s3_upload(app: tauri::AppHandle, path: String) -> R<demolens_ai::s3::Uploaded> {
    let p = check_write(&app, &path, &["mp4", "webm", "gif"])?;
    blocking(move || {
        let raw = std::fs::read_to_string(s3_path()).map_err(|_| "Set up storage in Settings first".to_string())?;
        let cfg: demolens_ai::s3::S3Config = serde_json::from_str(&raw).map_err(e)?;
        let secret = get_key("s3-secret").ok_or("No storage secret saved")?;
        demolens_ai::s3::upload(&cfg, &secret, &p).map_err(e)
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
async fn ai_transcribe(app: tauri::AppHandle, mut project: Project, model: Option<String>, cloud: Option<bool>) -> R<AiResult<pipeline::TranscribeReport>> {
    blocking(move || {
        let report = match cloud_key_for(Task::Stt)?.filter(|_| cloud.unwrap_or(false)) {
            Some((cfg, p, key)) => {
                progress(&app, "transcribe", 0.1, &format!("Sending the audio to {}...", p.info().name));
                pipeline::transcribe_project_cloud(&mut project, &cfg, p, &key, None).map_err(e)?
            }
            None => {
                progress(&app, "transcribe", 0.1, "Listening (runs on this computer)...");
                pipeline::transcribe_project(&mut project, model.as_deref(), None).map_err(e)?
            }
        };
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
async fn ai_docs(app: tauri::AppHandle, project: Project, use_llm: bool, cloud: Option<bool>, confirm_remote: Option<bool>) -> R<serde_json::Value> {
    blocking(move || {
        progress(&app, "docs", 0.1, "Looking for a local LLM...");
        let cloud_llm = match cloud_key_for(Task::Llm)?.filter(|_| use_llm && cloud.unwrap_or(false)) {
            Some((cfg, p, key)) => Some(providers::llm(&cfg, p, &key).map_err(e)?),
            None => None,
        };
        let llm = match cloud_llm {
            Some(l) => Some(l),
            None if use_llm => Llm::detect().map_err(e)?,
            None => None,
        };
        // A DEMOLENS_LLM_URL that points off this machine needs a yes each time.
        if let Some(l) = &llm {
            if l.is_remote() && cloud != Some(true) && confirm_remote != Some(true) {
                return Err(format!("REMOTE_LLM:{}", l.host()));
            }
        }
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
    let app2 = app.clone();
    blocking(move || {
        let speech = demolens_core::sample::can_speak();
        let proj = demolens_core::sample::make(&dir, speech, true).or_else(|_| demolens_core::sample::make(&dir, false, false)).map_err(e)?;
        let project = Project::load(&proj).map_err(e)?;
        grant(&app2, &proj);
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
fn reveal(app: tauri::AppHandle, path: String) -> R<()> {
    let p = std::path::PathBuf::from(&path);
    let in_recordings = recordings_dir(&app).ok().and_then(|r| r.canonicalize().ok()).is_some_and(|r| p.canonicalize().is_ok_and(|c| c.starts_with(r)));
    if !in_recordings && !app.state::<AppState>().granted.lock().unwrap().contains(&p) {
        return Err("Only files you saved or recorded can be shown".into());
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(&p).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(&p)).spawn();
    r.map(|_| ()).map_err(e)
}

#[tauri::command]
fn export_captions(app: tauri::AppHandle, project: Project, path: String) -> R<()> {
    let p = check_write(&app, &path, &["srt", "vtt"])?;
    let text = if path.to_lowercase().ends_with(".vtt") { captions::to_vtt(&project) } else { captions::to_srt(&project) };
    std::fs::write(p, text).map_err(e)
}

#[tauri::command]
fn write_text(app: tauri::AppHandle, path: String, text: String) -> R<()> {
    let p = check_write(&app, &path, &["md", "txt"])?;
    std::fs::write(p, text).map_err(e)
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
    demolens_core::input::run_keyhook_if_requested();
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
    let pause_key = Shortcut::new(Some(Modifiers::SHIFT | if cfg!(target_os = "macos") { Modifiers::SUPER } else { Modifiers::CONTROL }), Code::KeyP);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, sc, ev| {
                    if *sc == pause_key && ev.state() == ShortcutState::Pressed {
                        let _ = pause_recording(app.clone(), None);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // Ctrl/Cmd+Shift+P pauses input capture from any app.
            if let Err(err) = app.global_shortcut().register(pause_key) {
                eprintln!("pause shortcut unavailable: {err}");
            }
            let removed = apply_retention(app.handle());
            if removed > 0 {
                eprintln!("retention: removed {removed} old recordings");
            }
            Ok(())
        })
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
            ffmpeg_status,
            pick_save,
            pick_open,
            pause_recording,
            screen_snapshot,
            raw_frame,
            get_settings,
            set_retention,
            list_recordings,
            delete_recording,
            delete_all_data,
            cloud_status,
            cloud_set_key,
            cloud_remove_key,
            cloud_consent,
            cloud_configure,
            cloud_preview,
            cloud_tts,
            s3_get,
            s3_set,
            s3_upload
        ])
        .run(tauri::generate_context!())
        .expect("error while running DemoLens");
}
