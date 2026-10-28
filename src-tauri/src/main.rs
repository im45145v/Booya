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
use demolens_core::shortcuts;
use demolens_core::{export, new_project, zoom, EventLog, ZoomSegment};
use demolens_ai::providers::{self, CloudConfig, Provider, Task};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

mod keys;
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
    /// Microphone level meter (before and during a recording).
    meter: Mutex<Option<demolens_core::capture::mics::MicMeter>>,
    /// Sources a proxy is being built for right now.
    proxying: Mutex<HashSet<String>>,
    /// Set to abort the 3-2-1 countdown.
    cancel_countdown: std::sync::atomic::AtomicBool,
}

fn cache_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_cache_dir().unwrap_or_else(|_| std::env::temp_dir().join("demolens-cache"))
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
    Ok(zoom::generate_zoom_segments(&log, project.duration_ms, &project.zoom_config.to_config()))
}

/// What the keystroke overlay shows and when, for the timeline's key lane.
#[tauri::command]
fn key_track(project: Project) -> R<Vec<demolens_core::render::KeyLabel>> {
    let Some(ev) = &project.events else { return Ok(vec![]) };
    let log = EventLog::load(ev).map_err(e)?;
    Ok(demolens_core::render::build_key_labels(&log, project.keys.hold_ms, project.keys.shortcuts_only))
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
        // Never ask for a frame past the last one; the decoder would fail there.
        let src_t = project.source_time(out_ms).unwrap_or(project.duration_ms).min((project.duration_ms - 40.0).max(0.0));
        let (dw, dh) = r.decode_size();
        // The editor decodes the 720p proxy when it is ready and big enough.
        let decode_path = demolens_core::proxy::existing(&cache_dir(&app), &project.source)
            .filter(|_| demolens_core::proxy::fits(demolens_core::proxy::proxy_size(project.source_width, project.source_height), (dw, dh)))
            .unwrap_or_else(|| project.source.clone());
        let src = {
            let mut dec = state.decoder.lock().unwrap();
            if !dec.as_ref().is_some_and(|d| d.matches(&decode_path, dw, dh)) {
                *dec = Some(demolens_core::ffmpeg::PreviewDecoder::new(&decode_path, dw, dh));
            }
            dec.as_mut().unwrap().frame(src_t, playing.unwrap_or(false)).map_err(e)?
        };
        let mut frame = r.render(&src, src_t);
        // The camera is not in the proxy, so it is read from its own file.
        export::draw_preview_webcam(&r, &project, &mut frame, src_t);
        let png = frame.encode_png().map_err(e)?;
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
async fn start_recording(
    app: tauri::AppHandle,
    target: CaptureTarget,
    mic: bool,
    system_audio: bool,
    keys: Option<demolens_core::input::KeyLogging>,
    webcam: Option<bool>,
    mic_device: Option<String>,
    countdown: Option<u32>,
) -> R<Vec<String>> {
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
        state.cancel_countdown.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    let fail_reset = |app: &tauri::AppHandle| {
        *app.state::<AppState>().recording.lock().unwrap() = None;
    };
    // A 3-2-1 countdown on screen, so there is time to switch to the thing
    // being recorded. Clicking the number cancels it.
    let secs = countdown.unwrap_or(0).min(10);
    if secs > 0 {
        let a2 = app.clone();
        let cancelled = blocking(move || Ok(run_countdown(&a2, secs))).await.unwrap_or(true);
        if cancelled {
            fail_reset(&app);
            return Err("Recording cancelled".into());
        }
    }
    // Recordings go to Videos/DemoLens/<timestamp>. The folder is only
    // created once capture really starts (see capture::start).
    let base = recordings_dir(&app)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let out_dir = base.join(format!("recording-{stamp}"));
    let mic_device = mic_device.filter(|m| !m.is_empty());
    let res = blocking(move || capture::start(&RecordOptions { target, fps: 30, mic, system_audio, out_dir, keys, webcam, mic_device }).map_err(e)).await;
    let state = app.state::<AppState>();
    let mut slot = state.recording.lock().unwrap();
    match res {
        Ok(s) => {
            let warnings = s.warnings.clone();
            *slot = Some(RecSlot::Running(s));
            drop(slot);
            show_indicator(&app);
            // Get the editor out of the picture while recording.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.minimize();
            }
            Ok(warnings)
        }
        Err(err) => {
            *slot = None;
            Err(err)
        }
    }
}

/// Show the countdown window and wait. True when the user cancelled.
fn run_countdown(app: &tauri::AppHandle, secs: u32) -> bool {
    use std::sync::atomic::Ordering;
    let w = tauri::WebviewWindowBuilder::new(app, "countdown", tauri::WebviewUrl::App(format!("countdown.html?n={secs}").into()))
        .title("DemoLens countdown")
        .inner_size(200.0, 200.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .center()
        .content_protected(true)
        .build();
    let Ok(w) = w else { return false };
    let state = app.state::<AppState>();
    let end = std::time::Instant::now() + std::time::Duration::from_millis(secs as u64 * 1000 + 150);
    let mut cancelled = false;
    while std::time::Instant::now() < end {
        if state.cancel_countdown.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = w.close();
    // Let the window actually disappear before the first frame is captured.
    std::thread::sleep(std::time::Duration::from_millis(250));
    cancelled
}

#[tauri::command]
fn cancel_countdown(app: tauri::AppHandle) {
    app.state::<AppState>().cancel_countdown.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// State for the recording badge: whether recording, paused, and time.
#[tauri::command]
fn rec_status(app: tauri::AppHandle) -> serde_json::Value {
    let state = app.state::<AppState>();
    let slot = state.recording.lock().unwrap();
    match &*slot {
        Some(RecSlot::Running(s)) => serde_json::json!({
            "recording": true, "paused": s.is_paused(),
            "elapsed_ms": s.elapsed().as_millis() as u64, "paused_ms": s.paused().as_millis() as u64,
        }),
        Some(RecSlot::Starting) => serde_json::json!({ "recording": false, "starting": true }),
        None => serde_json::json!({ "recording": false }),
    }
}

/// The badge's Stop button: bring the editor back and let it run the same
/// stop flow as its own Stop button.
#[tauri::command]
fn request_stop(app: tauri::AppHandle) {
    restore_main(&app);
    let _ = app.emit("rec-stop-request", ());
}

fn restore_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Stop and throw the recording away.
#[tauri::command]
async fn discard_recording(app: tauri::AppHandle) -> R<()> {
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
    restore_main(&app);
    let r = blocking(move || s.discard().map_err(e)).await;
    let _ = app.emit("rec-discarded", ());
    r
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
    restore_main(&app);
    let app2 = app.clone();
    blocking(move || {
        let rec = s.stop().map_err(e)?;
        // The 720p editing proxy is built in the background while the editor opens.
        let src = rec.video.clone();
        let a3 = app2.clone();
        std::thread::spawn(move || {
            let _ = build_proxy(&a3, &src);
        });
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

/// Displays and windows the picker can show (names and sizes only, fast).
#[tauri::command]
async fn capture_sources() -> R<serde_json::Value> {
    blocking(|| Ok(serde_json::to_value(capture::sources::list_sources().map_err(e)?).map_err(e)?)).await
}

/// Preview image and app icon for one source, as data URLs. Fetched one at a
/// time by the picker as items scroll into view.
#[tauri::command]
async fn capture_thumb(kind: String, id: String, bundle: String, x: i32, y: i32, w: u32, h: u32) -> R<serde_json::Value> {
    blocking(move || {
        let src = capture::sources::SourceInfo { kind, id, title: String::new(), app: String::new(), bundle, x, y, w, h, z: 0 };
        let t = capture::sources::thumbnail(&src).map_err(e)?;
        let url = |b: Option<Vec<u8>>| b.map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
        Ok(serde_json::json!({ "image": url(t.image), "icon": url(t.icon) }))
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

/// The recording pill: always on top, draggable, left out of the recording
/// (content protected), frosted on macOS. It sits at the top centre of the
/// screen with the cursor on it.
fn show_indicator(app: &tauri::AppHandle) {
    if app.get_webview_window("indicator").is_some() {
        return;
    }
    let w = tauri::WebviewWindowBuilder::new(app, "indicator", tauri::WebviewUrl::App("indicator.html".into()))
        .title("DemoLens is recording")
        .inner_size(384.0, 52.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .content_protected(true)
        .build();
    if let Ok(w) = w {
        #[cfg(target_os = "macos")]
        {
            use tauri::window::{Effect, EffectState, EffectsBuilder};
            let _ = w.set_effects(EffectsBuilder::new().effect(Effect::HudWindow).state(EffectState::Active).radius(26.0).build());
        }
        if let Ok(Some(m)) = w.current_monitor().or_else(|_| w.primary_monitor()) {
            let sz = m.size();
            let sf = m.scale_factor();
            let x = m.position().x + ((sz.width as f64 - 384.0 * sf) / 2.0) as i32;
            let y = m.position().y + (18.0 * sf) as i32;
            let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
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

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(default)]
struct Settings {
    /// Delete recordings older than this many days at startup. None keeps them.
    retention_days: Option<u32>,
    /// Frosted glass look for the app itself (macOS vibrancy, or a CSS blur elsewhere).
    glass_ui: bool,
    /// Show DemoLens in the menu bar (macOS) or the system tray (Windows).
    menu_bar: bool,
    /// Shortcut overrides by action id. Missing means the default.
    shortcuts: demolens_core::shortcuts::Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self { retention_days: None, glass_ui: false, menu_bar: true, shortcuts: Default::default() }
    }
}

fn settings_path() -> PathBuf {
    demolens_core::preset::presets_dir().parent().map(|p| p.join("settings.json")).unwrap_or_else(|| "settings.json".into())
}

fn load_settings() -> Settings {
    let mut st: Settings = std::fs::read_to_string(settings_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    // A hand edited file must not break startup or register bad keys.
    st.shortcuts = demolens_core::shortcuts::sanitize(&st.shortcuts);
    st
}

fn save_settings(st: &Settings) -> R<()> {
    let p = settings_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(e)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(st).map_err(e)?).map_err(e)?;
    Ok(())
}

#[tauri::command]
fn get_settings() -> Settings {
    load_settings()
}

#[tauri::command]
fn set_retention(app: tauri::AppHandle, days: Option<u32>) -> R<usize> {
    let mut st = load_settings();
    st.retention_days = days.filter(|d| *d > 0);
    save_settings(&st)?;
    Ok(apply_retention(&app))
}

/// Remember the glass look choice. The native effect is applied separately
/// (`glass_native`), because Reduce Transparency can pause it without
/// changing the choice.
#[tauri::command]
fn set_glass_ui(enabled: bool) -> R<()> {
    let mut st = load_settings();
    st.glass_ui = enabled;
    save_settings(&st)
}

/// Turn the macOS vibrancy behind the editor on or off. Other platforms use
/// the CSS fallback only, so this does nothing there.
#[tauri::command]
fn glass_native(app: tauri::AppHandle, on: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        use tauri::window::{Effect, EffectState, EffectsBuilder};
        if let Some(w) = app.get_webview_window("main") {
            let r = if on {
                w.set_effects(EffectsBuilder::new().effect(Effect::Sidebar).state(EffectState::FollowsWindowActiveState).build())
            } else {
                w.set_effects(Option::<tauri::utils::config::WindowEffectsConfig>::None)
            };
            return on && r.is_ok();
        }
    }
    let _ = (&app, on);
    false
}

/// Is the system "Reduce transparency" setting on? Only macOS exposes it
/// outside the web view; elsewhere the web view's own media query is used.
#[tauri::command]
fn os_reduce_transparency() -> bool {
    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("defaults")
            .args(["read", "com.apple.universalaccess", "reduceTransparency"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
            .unwrap_or(false);
    }
    #[allow(unreachable_code)]
    {
        false
    }
}

/// Show or hide the menu bar / tray item.
#[tauri::command]
fn set_menu_bar(app: tauri::AppHandle, enabled: bool) -> R<()> {
    let mut st = load_settings();
    st.menu_bar = enabled;
    save_settings(&st)?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if let Some(t) = app.tray_by_id("main") {
            let _ = t.set_visible(enabled);
        }
    }
    let _ = &app;
    Ok(())
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
    let p = check_write(&app, &path, &["mp4", "webm", "gif", "html"])?;
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


/// Build a self-contained click-through demo (one HTML file) from the clicks.
/// Runs fully on this computer; secrets already blurred in the project are
/// blurred in every screenshot.
#[tauri::command]
async fn export_interactive(app: tauri::AppHandle, project: Project, out: String, title: Option<String>, cta_text: Option<String>, cta_url: Option<String>, badge: Option<bool>) -> R<String> {
    let outp = check_write(&app, &out, &["html"])?;
    blocking(move || {
        let log = project.events.as_ref().and_then(|p| EventLog::load(p).ok());
        let o = demolens_ai::interactive::Options {
            title: title.filter(|t| !t.trim().is_empty()),
            cta_text: cta_text.filter(|t| !t.trim().is_empty()),
            cta_url: cta_url.filter(|t| !t.trim().is_empty()),
            badge: badge.unwrap_or(true),
            ..Default::default()
        };
        let (html, n) = demolens_ai::interactive::generate(&project, log.as_ref(), None, &o, |i, total| {
            progress(&app, "interactive", if total == 0 { 0.0 } else { i as f64 / total as f64 }, &format!("Capturing step {} of {}", (i + 1).min(total), total));
        })
        .map_err(e)?;
        std::fs::write(&outp, &html).map_err(e)?;
        Ok(format!("{n} steps, {:.1} MB", html.len() as f64 / 1e6))
    })
    .await
}


// ---------- Background photos ----------

fn recents_path(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from(".")).join("recent-backgrounds.json")
}

fn load_recents(app: &tauri::AppHandle) -> Vec<String> {
    std::fs::read_to_string(recents_path(app))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| PathBuf::from(p).is_file())
        .collect()
}

/// Copy a chosen photo (png, jpg, heic, webp, bmp, gif) into the recording's
/// folder as a JPEG or PNG at most 3840 px wide, so the project keeps working
/// if the original moves or is deleted. Returns the copy's path.
#[tauri::command]
async fn background_import(app: tauri::AppHandle, path: String, project_source: String) -> R<String> {
    blocking(move || {
        let src = PathBuf::from(&path);
        let ext = src.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "webp", "heic", "heif", "bmp", "gif"].contains(&ext.as_str()) {
            return Err("Pick a png, jpg, heic, webp, bmp or gif photo".into());
        }
        let meta = std::fs::metadata(&src).map_err(|_| "Could not read that file".to_string())?;
        if meta.len() > 80_000_000 {
            return Err("That photo is over 80 MB. Pick a smaller one.".into());
        }
        let dir = PathBuf::from(&project_source).parent().map(|p| p.to_path_buf()).filter(|p| p.is_dir()).ok_or("Open a recording first")?;
        let stem: String = src.file_stem().and_then(|x| x.to_str()).unwrap_or("photo").chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).take(40).collect();
        let out_ext = if ext == "png" { "png" } else { "jpg" };
        let dest = dir.join(format!("background-{stem}.{out_ext}"));
        let run = |bin: &str, args: Vec<std::ffi::OsString>| std::process::Command::new(bin).args(args).output().map(|o| o.status.success() && dest.is_file()).unwrap_or(false);
        let mut ok = run(
            &demolens_core::ffmpeg::ffmpeg_bin(),
            vec!["-v".into(), "error".into(), "-y".into(), "-i".into(), src.clone().into_os_string(), "-vf".into(), "scale='min(3840,iw)':-2".into(), "-frames:v".into(), "1".into(), "-q:v".into(), "2".into(), dest.clone().into_os_string()],
        );
        // Not every ffmpeg build reads HEIC. macOS can convert it itself.
        if !ok && cfg!(target_os = "macos") {
            ok = run("/usr/bin/sips", vec!["-s".into(), "format".into(), "jpeg".into(), "-Z".into(), "3840".into(), src.clone().into_os_string(), "--out".into(), dest.clone().into_os_string()]);
        }
        if !ok {
            return Err(if ext == "heic" || ext == "heif" {
                "This computer could not read that HEIC photo. Export it as JPEG from Photos and pick that.".into()
            } else {
                "Could not read that image. It may be damaged.".into()
            });
        }
        let mut rec = load_recents(&app);
        let d = dest.to_string_lossy().to_string();
        rec.retain(|r| r != &d);
        rec.insert(0, d.clone());
        rec.truncate(12);
        let _ = std::fs::create_dir_all(recents_path(&app).parent().unwrap_or(&PathBuf::from(".")));
        let _ = std::fs::write(recents_path(&app), serde_json::to_string(&rec).unwrap_or_default());
        Ok(d)
    })
    .await
}

#[tauri::command]
fn background_recents(app: tauri::AppHandle) -> Vec<String> {
    load_recents(&app)
}

/// Small preview of a photo as a data URL, for the recents strip.
#[tauri::command]
async fn background_thumb(path: String) -> R<String> {
    blocking(move || {
        let f = demolens_core::ffmpeg::load_image_fit(&PathBuf::from(path), 160, 90, demolens_core::project::BgFit::Fill).map_err(e)?;
        let png = f.encode_png().map_err(e)?;
        Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
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


// ---------- Editing proxy ----------

/// Build (or find) the 720p proxy for `src`, telling the UI how far along it is.
fn build_proxy(app: &tauri::AppHandle, src: &Path) -> R<demolens_core::proxy::ProxyStats> {
    let key = src.to_string_lossy().to_string();
    {
        let st = app.state::<AppState>();
        if !st.proxying.lock().unwrap().insert(key.clone()) {
            return Err("A proxy is already being built for this video".into());
        }
    }
    let a2 = app.clone();
    let k2 = key.clone();
    let r = demolens_core::proxy::build(&cache_dir(app), src, move |p| {
        let _ = a2.emit("proxy-progress", serde_json::json!({ "source": k2, "progress": p, "done": false }));
    });
    app.state::<AppState>().proxying.lock().unwrap().remove(&key);
    match &r {
        Ok(st) => {
            let _ = app.emit("proxy-progress", serde_json::json!({ "source": key, "progress": 1.0, "done": true, "path": st.path, "seconds": st.seconds, "encoder": st.encoder }));
        }
        Err(err) => {
            let _ = app.emit("proxy-progress", serde_json::json!({ "source": key, "progress": 0.0, "done": true, "error": format!("{err:#}") }));
        }
    }
    r.map_err(e)
}

/// Make sure the editing proxy exists for a video (runs in the background).
#[tauri::command]
async fn proxy_ensure(app: tauri::AppHandle, source: String) -> R<serde_json::Value> {
    let src = PathBuf::from(source);
    blocking(move || {
        let st = build_proxy(&app, &src)?;
        Ok(serde_json::json!({ "path": st.path, "seconds": st.seconds, "encoder": st.encoder, "cached": st.cached, "width": st.width, "height": st.height }))
    })
    .await
}

/// Path of the finished proxy, if there is one. The editor plays its audio.
#[tauri::command]
fn proxy_path(app: tauri::AppHandle, source: String) -> Option<String> {
    demolens_core::proxy::existing(&cache_dir(&app), &PathBuf::from(source)).map(|p| p.to_string_lossy().to_string())
}

// ---------- Microphone ----------

#[tauri::command]
async fn mic_list() -> R<Vec<String>> {
    blocking(|| Ok(demolens_core::capture::mics::list_mics())).await
}

/// Start the level meter on a microphone (None = the default one). Levels
/// arrive as `mic-level` events about ten times a second.
#[tauri::command]
async fn mic_meter_start(app: tauri::AppHandle, device: Option<String>) -> R<()> {
    let a2 = app.clone();
    blocking(move || {
        let state = a2.state::<AppState>();
        if let Some(mut m) = state.meter.lock().unwrap().take() {
            m.stop();
        }
        let a3 = a2.clone();
        let device = device.filter(|d| !d.is_empty());
        let m = demolens_core::capture::mics::MicMeter::start(device.as_deref(), move |rms, peak| {
            let _ = a3.emit("mic-level", serde_json::json!({ "rms": rms, "peak": peak }));
        })
        .map_err(e)?;
        *state.meter.lock().unwrap() = Some(m);
        Ok(())
    })
    .await
}

#[tauri::command]
fn mic_meter_stop(app: tauri::AppHandle) {
    if let Some(mut m) = app.state::<AppState>().meter.lock().unwrap().take() {
        m.stop();
    }
}

// ---------- Storage ----------

#[tauri::command]
async fn storage_report(app: tauri::AppHandle) -> R<demolens_core::storage::StorageReport> {
    let root = recordings_dir(&app)?;
    let cache = cache_dir(&app);
    blocking(move || Ok(demolens_core::storage::scan(&root, Some(&cache)))).await
}

/// Delete the given recording folders (the UI asked the user first). Each one
/// is checked again here: it must be a real `recording-*` folder inside the
/// recordings folder.
#[tauri::command]
async fn storage_delete(app: tauri::AppHandle, paths: Vec<String>) -> R<serde_json::Value> {
    let root = recordings_dir(&app)?;
    blocking(move || {
        let (mut n, mut bytes, mut failed) = (0u32, 0u64, Vec::new());
        for p in paths {
            match demolens_core::storage::delete_recording(&root, &PathBuf::from(&p)) {
                Ok(b) => {
                    n += 1;
                    bytes += b;
                }
                Err(err) => failed.push(format!("{}: {err:#}", p)),
            }
        }
        Ok(serde_json::json!({ "deleted": n, "bytes": bytes, "failed": failed }))
    })
    .await
}

/// Remove cached editing proxies (they are rebuilt when needed).
#[tauri::command]
fn storage_clear_proxies(app: tauri::AppHandle) -> u64 {
    demolens_core::proxy::prune(&cache_dir(&app), 0)
}

// ---------- AI status ----------

fn cloud_provider_name(t: Task) -> Option<String> {
    cloud_key_for(t).ok().flatten().map(|(_, p, _)| p.info().name.to_string())
}

#[tauri::command]
fn ai_status() -> serde_json::Value {
    serde_json::json!({
        "features": demolens_ai::status::features(&cloud_provider_name),
        "recommended_missing": demolens_ai::status::missing_recommended(),
        "models_dir": models::models_dir(),
    })
}

#[tauri::command]
async fn ai_self_test(id: String) -> R<demolens_ai::status::TestResult> {
    blocking(move || Ok(demolens_ai::status::self_test(&id))).await
}

/// Download every recommended model that is missing, one after the other.
/// Progress arrives as `ai-progress` events with the task `download:all`.
#[tauri::command]
async fn models_download_recommended(app: tauri::AppHandle) -> R<usize> {
    blocking(move || {
        let todo = demolens_ai::status::missing_recommended();
        let total: u64 = todo.iter().map(|m| m.size).sum::<u64>().max(1);
        let mut done_before = 0u64;
        let mut n = 0;
        for m in &todo {
            let base = done_before;
            let id = m.id;
            let title = m.title;
            models::download(id, |d, _t| {
                let all = (base + d) as f64 / total as f64;
                progress(&app, "download:all", all.min(1.0), &format!("{title} ({} of {})", models::human(base + d), models::human(total)));
            })
            .map_err(|err| format!("{title}: {err:#}"))?;
            done_before += m.size;
            n += 1;
        }
        progress(&app, "download:all", 1.0, "Done");
        Ok(n)
    })
    .await
}

// ---------- Menu (macOS) ----------

#[cfg(target_os = "macos")]
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    let item = |id: &str, text: &str, acc: Option<&str>| -> tauri::Result<tauri::menu::MenuItem<tauri::Wry>> {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = acc {
            b = b.accelerator(a);
        }
        b.build(app)
    };
    let settings = item("settings", "Settings...", Some("Cmd+,"))?;
    let app_menu = SubmenuBuilder::new(app, "DemoLens")
        .about(None)
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&item("open", "Open...", Some("Cmd+O"))?)
        .item(&item("save", "Save", Some("Cmd+S"))?)
        .item(&item("save-as", "Save As...", Some("Shift+Cmd+S"))?)
        .separator()
        .item(&item("export", "Export...", Some("Cmd+E"))?)
        .separator()
        .close_window()
        .build()?;
    // Undo and Redo are DemoLens commands (the web view routes them to a text
    // field when one has focus), so the standard Cmd+Z actually works.
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&item("undo", "Undo", Some("Cmd+Z"))?)
        .item(&item("redo", "Redo", Some("Shift+Cmd+Z"))?)
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    // The Record menu lists every shortcut with the keys currently bound. The
    // keys are text, not menu accelerators: the global shortcut already fires
    // them, and a second registration would run each action twice.
    let ov = load_settings().shortcuts;
    let rec_items = shortcuts::ACTIONS
        .iter()
        .map(|a| item(&format!("sc:{}", a.id), &menu_text(a.label, &shortcuts::effective(&ov, a.id), true), None))
        .collect::<tauri::Result<Vec<_>>>()?;
    let mut rec = SubmenuBuilder::new(app, "Record");
    for it in &rec_items {
        rec = rec.item(it);
    }
    let rec = rec.build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().separator().fullscreen().build()?;
    let help = SubmenuBuilder::new(app, "Help").item(&item("shortcuts", "Keyboard Shortcuts", Some("Cmd+/"))?).item(&item("ai-status", "AI Status", None)?).build()?;
    let menu = MenuBuilder::new(app).item(&app_menu).item(&file).item(&edit).item(&rec).item(&window).item(&help).build()?;
    app.set_menu(menu)?;
    Ok(())
}

/// "Pause or Resume  (Shift+Cmd+P)", or just the label when unbound.
#[allow(dead_code)]
fn menu_text(label: &str, combo: &str, mac: bool) -> String {
    let k = shortcuts::label(combo, mac);
    if k.is_empty() {
        label.to_string()
    } else {
        format!("{label}  ({k})")
    }
}

/// Shortcuts the OS refused to register, by action id, with the reason.
static SHORTCUT_FAILURES: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
/// The shortcuts registered right now, so the handler can tell which action fired.
static REGISTERED: Mutex<Vec<(String, tauri_plugin_global_shortcut::Shortcut)>> = Mutex::new(Vec::new());

/// Register every bound shortcut with the OS, replacing whatever was
/// registered before. Failures (another app owns the keys) are remembered
/// per action and shown on the Shortcuts page.
fn apply_shortcuts(app: &tauri::AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let gs = app.global_shortcut();
    let old: Vec<_> = REGISTERED.lock().unwrap().drain(..).collect();
    for (_, sc) in old {
        let _ = gs.unregister(sc);
    }
    let mut failures = BTreeMap::new();
    let overrides = load_settings().shortcuts;
    for (id, combo) in shortcuts::all_effective(&overrides) {
        if combo.is_empty() {
            continue;
        }
        let Some(sc) = keys::to_shortcut(&combo) else {
            failures.insert(id.to_string(), "That key combination is not supported.".to_string());
            continue;
        };
        match gs.register(sc) {
            Ok(()) => REGISTERED.lock().unwrap().push((id.to_string(), sc)),
            Err(err) => {
                eprintln!("shortcut {id} ({combo}) unavailable: {err}");
                failures.insert(id.to_string(), err.to_string());
            }
        }
    }
    *SHORTCUT_FAILURES.lock().unwrap() = failures;
}

/// Rebuild what shows the keys: the macOS Record menu and the tray menu.
fn refresh_menus(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = build_menu(app);
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if let Some(t) = app.tray_by_id("main") {
            if let Ok(m) = tray_menu(app) {
                let _ = t.set_menu(Some(m));
            }
        }
    }
    let _ = app;
}

/// Run a shortcut action (from a global shortcut, the menu or the tray).
/// Pause and markers act on the recording directly; everything else goes to
/// the editor, which owns the source, microphone and picker choices.
fn run_action(app: &tauri::AppHandle, id: &str) {
    match id {
        "pause" => {
            let _ = pause_recording(app.clone(), None);
        }
        "marker" => {
            let _ = drop_marker(app.clone());
        }
        "record_toggle" => {
            restore_main_if_idle(app);
            let _ = app.emit("global-record", ());
        }
        "record_area" | "record_window" | "record_screen" => {
            restore_main_if_idle(app);
            let _ = app.emit("shortcut", id);
        }
        other => {
            let _ = app.emit("shortcut", other);
        }
    }
}

/// Drop a marker at the cursor: a deliberate "zoom here" point in the editor.
#[tauri::command]
fn drop_marker(app: tauri::AppHandle) -> R<bool> {
    let state = app.state::<AppState>();
    let slot = state.recording.lock().unwrap();
    let Some(RecSlot::Running(s)) = &*slot else { return Err("Not recording".into()) };
    let ok = s.mark();
    if ok {
        let _ = app.emit("rec-marker", ());
    }
    Ok(ok)
}

fn shortcuts_json() -> serde_json::Value {
    let ov = load_settings().shortcuts;
    let failed = SHORTCUT_FAILURES.lock().unwrap().clone();
    let mac = cfg!(target_os = "macos");
    let rows: Vec<_> = shortcuts::ACTIONS
        .iter()
        .map(|a| {
            let combo = shortcuts::effective(&ov, a.id);
            serde_json::json!({
                "id": a.id, "label": a.label, "hint": a.hint,
                "default": a.default, "combo": combo,
                "text": shortcuts::label(&combo, mac),
                "default_text": shortcuts::label(a.default, mac),
                "custom": ov.contains_key(a.id),
                "error": failed.get(a.id),
            })
        })
        .collect();
    serde_json::json!({ "mac": mac, "actions": rows })
}

/// Every action with its keys, whether they are custom, and any registration error.
#[tauri::command]
fn get_shortcuts() -> serde_json::Value {
    shortcuts_json()
}

/// Rebind one action. An empty combo unbinds it. A combo the OS refuses is
/// rolled back, so the old keys keep working.
#[tauri::command]
fn set_shortcut(app: tauri::AppHandle, action: String, combo: String) -> R<serde_json::Value> {
    let mut st = load_settings();
    let prev = st.shortcuts.clone();
    shortcuts::set_binding(&mut st.shortcuts, &action, &combo)?;
    save_settings(&st)?;
    apply_shortcuts(&app);
    let failed = SHORTCUT_FAILURES.lock().unwrap().get(&action).cloned();
    if let Some(err) = failed {
        st.shortcuts = prev;
        save_settings(&st)?;
        apply_shortcuts(&app);
        refresh_menus(&app);
        let name = shortcuts::info(&action).map(|a| a.label).unwrap_or("this action");
        return Err(format!("{} cannot use those keys: the system refused them, usually because another app has them. Pick different keys. ({err})", name));
    }
    refresh_menus(&app);
    Ok(shortcuts_json())
}

/// Release every global shortcut while the recorder is listening (so the OS
/// does not swallow the combo being captured), and register them again after.
#[tauri::command]
fn shortcuts_pause(app: tauri::AppHandle, paused: bool) {
    if paused {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let old: Vec<_> = REGISTERED.lock().unwrap().drain(..).collect();
        for (_, sc) in old {
            let _ = app.global_shortcut().unregister(sc);
        }
    } else {
        apply_shortcuts(&app);
    }
}

/// Put every shortcut back to its default.
#[tauri::command]
fn reset_shortcuts(app: tauri::AppHandle) -> R<serde_json::Value> {
    let mut st = load_settings();
    st.shortcuts.clear();
    save_settings(&st)?;
    apply_shortcuts(&app);
    refresh_menus(&app);
    Ok(shortcuts_json())
}

/// Global shortcuts that could not be registered (empty when all work).
#[tauri::command]
fn shortcut_status() -> String {
    SHORTCUT_FAILURES.lock().unwrap().keys().cloned().collect::<Vec<_>>().join(",")
}

/// When idle, a record shortcut brings the editor forward so the choices (and
/// the picker) are visible. While recording the editor stays out of the way.
fn restore_main_if_idle(app: &tauri::AppHandle) {
    let recording = matches!(&*app.state::<AppState>().recording.lock().unwrap(), Some(RecSlot::Running(_)));
    if !recording {
        restore_main(app);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn tray_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    let ov = load_settings().shortcuts;
    let mac = cfg!(target_os = "macos");
    let mut m = MenuBuilder::new(app);
    for id in ["record_toggle", "pause", "record_area", "record_window", "record_screen", "marker"] {
        if let Some(a) = shortcuts::info(id) {
            m = m.item(&MenuItemBuilder::with_id(format!("sc:{id}"), menu_text(a.label, &shortcuts::effective(&ov, id), mac)).build(app)?);
        }
    }
    m.separator()
        .item(&MenuItemBuilder::with_id("tray-show", "Show DemoLens").build(app)?)
        .item(&MenuItemBuilder::with_id("tray-quit", "Quit DemoLens").build(app)?)
        .build()
}

/// The menu bar (macOS) or tray (Windows) item with quick record.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;
    let mut b = TrayIconBuilder::with_id("main").menu(&tray_menu(app)?).tooltip("DemoLens");
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    let tray = b.build(app)?;
    let _ = tray.set_visible(load_settings().menu_bar);
    Ok(())
}

fn main() {
    demolens_core::input::run_keyhook_if_requested();
    use tauri_plugin_global_shortcut::ShortcutState;
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, sc, ev| {
                    if ev.state() != ShortcutState::Pressed {
                        return;
                    }
                    let id = REGISTERED.lock().unwrap().iter().find(|(_, s)| s == sc).map(|(a, _)| a.clone());
                    if let Some(id) = id {
                        run_action(app, &id);
                    }
                })
                .build(),
        )
        .on_menu_event(|app, event| {
            let id = event.id().0.clone();
            if let Some(action) = id.strip_prefix("sc:") {
                run_action(app, action);
                return;
            }
            match id.as_str() {
                "tray-show" => restore_main(app),
                "tray-quit" => {
                    // Never quit under a running recording: bring the editor up so it can be stopped.
                    let recording = matches!(&*app.state::<AppState>().recording.lock().unwrap(), Some(RecSlot::Running(_)));
                    if recording {
                        restore_main(app);
                    } else {
                        app.exit(0);
                    }
                }
                "pause" => {
                    let _ = pause_recording(app.clone(), None);
                }
                "record" => {
                    let _ = app.emit("global-record", ());
                }
                _ => {
                    let _ = app.emit("menu", id);
                }
            }
        })
        .setup(move |app| {
            // Bound shortcuts work from any app. A failure (another app owns the
            // keys) is shown on Settings > Shortcuts.
            apply_shortcuts(app.handle());
            #[cfg(target_os = "macos")]
            build_menu(app.handle())?;
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                if let Err(err) = build_tray(app.handle()) {
                    eprintln!("tray unavailable: {err}");
                }
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
            regenerate_zoom, key_track,
            preview,
            export_interactive,
            background_import,
            background_recents,
            background_thumb,
            export_video,
            list_presets,
            save_preset,
            apply_preset,
            start_recording,
            stop_recording,
            cancel_countdown,
            rec_status,
            request_stop,
            discard_recording,
            proxy_ensure,
            proxy_path,
            mic_list,
            mic_meter_start,
            mic_meter_stop,
            storage_report,
            storage_delete,
            storage_clear_proxies,
            ai_status,
            ai_self_test,
            models_download_recommended,
            shortcut_status, get_shortcuts, set_shortcut, reset_shortcuts, shortcuts_pause, drop_marker,
            set_glass_ui, glass_native, os_reduce_transparency, set_menu_bar,
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
            screen_snapshot, capture_sources, capture_thumb,
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
