//! macOS screen capture through ScreenCaptureKit.
//!
//! ffmpeg's avfoundation screen input (AVCaptureScreenInput) is deprecated and
//! delivers no frames on current macOS, so frames come from an `SCStream` and
//! are fed to ffmpeg as raw BGRA on stdin. ffmpeg still encodes and muxes, so
//! the encoder choice, the mic input and the stop logic stay as they were.
//!
//! The cursor is not captured (DemoLens draws its own) and the DemoLens
//! process is excluded from the capture, so its window and the REC badge never
//! appear in a recording.
//!
//! The stream lives on its own thread so nothing here has to be `Send`.

use super::CaptureTarget;
use screencapturekit::prelude::*;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod cg {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGSize {
        pub width: f64,
        pub height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGRect {
        pub origin: CGPoint,
        pub size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
        fn CGDisplayModeGetPixelHeight(mode: *const c_void) -> usize;
        fn CGDisplayModeRelease(mode: *mut c_void);
    }

    /// Is the Screen Recording permission granted right now?
    pub fn has_access() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    /// Shows the system prompt (first time only). Returns the current state,
    /// which is still false until the user allows it and reopens the app.
    pub fn request_access() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }

    pub fn main_display() -> u32 {
        unsafe { CGMainDisplayID() }
    }

    /// (x, y, w, h) in points.
    pub fn bounds(id: u32) -> (f64, f64, f64, f64) {
        let b = unsafe { CGDisplayBounds(id) };
        (b.origin.x, b.origin.y, b.size.width, b.size.height)
    }

    /// Native pixel size of a display (3024x1964 on a 14" MacBook Pro), or None.
    pub fn pixel_size(id: u32) -> Option<(u32, u32)> {
        unsafe {
            let mode = CGDisplayCopyDisplayMode(id);
            if mode.is_null() {
                return None;
            }
            let w = CGDisplayModeGetPixelWidth(mode);
            let h = CGDisplayModeGetPixelHeight(mode);
            CGDisplayModeRelease(mode);
            (w > 0 && h > 0).then_some((w as u32, h as u32))
        }
    }
}

extern "C" {
    /// Provided by the screencapturekit Swift bridge. Without it a process
    /// that is not an AppKit app (the CLI) can crash inside CoreGraphics.
    fn sc_initialize_core_graphics();
}

pub const PERMISSION_MESSAGE: &str = "DemoLens needs the Screen Recording permission. Open System Settings > Privacy & Security > Screen & System Audio Recording, switch DemoLens on (when you run the demolens command line tool, switch on your terminal app instead), then quit and reopen it.";

pub fn has_permission() -> bool {
    cg::has_access()
}

/// One captured display, for `demolens doctor` and the display picker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DisplayInfo {
    pub index: u32,
    pub id: u32,
    pub main: bool,
    pub points: (u32, u32),
    pub pixels: (u32, u32),
    pub scale: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContentInfo {
    pub displays: Vec<DisplayInfo>,
    pub windows: usize,
    pub applications: usize,
}

fn ordered_displays(content: &SCShareableContent) -> Vec<SCDisplay> {
    let main = cg::main_display();
    let mut d = content.displays();
    d.sort_by_key(|x| (x.display_id() != main, x.display_id()));
    d
}

fn display_info(index: u32, d: &SCDisplay) -> DisplayInfo {
    let id = d.display_id();
    let (_, _, bw, bh) = cg::bounds(id);
    let points = if bw > 0.0 && bh > 0.0 { (bw as u32, bh as u32) } else { (d.width(), d.height()) };
    let pixels = cg::pixel_size(id).unwrap_or(points);
    let scale = if points.0 > 0 { pixels.0 as f64 / points.0 as f64 } else { 1.0 };
    DisplayInfo { index, id, main: id == cg::main_display(), points, pixels, scale }
}

fn shareable() -> Result<SCShareableContent, String> {
    unsafe { sc_initialize_core_graphics() };
    if !cg::has_access() {
        // Triggers the system prompt the first time. The grant only takes
        // effect after the app is reopened, so fail with a clear message.
        let _ = cg::request_access();
    }
    SCShareableContent::get().map_err(|e| {
        if cg::has_access() {
            format!("ScreenCaptureKit could not list the screen content: {e}")
        } else {
            PERMISSION_MESSAGE.to_string()
        }
    })
}

/// Lists displays through SCShareableContent (also the permission check).
pub fn content_info() -> Result<ContentInfo, String> {
    let content = shareable()?;
    let displays = ordered_displays(&content).iter().enumerate().map(|(i, d)| display_info(i as u32, d)).collect();
    Ok(ContentInfo { displays, windows: content.windows().len(), applications: content.applications().len() })
}

/// What gets captured: stream size in pixels and the region in global pixels.
struct Source {
    filter: SCContentFilter,
    width: u32,
    height: u32,
    /// (x, y) of the captured rectangle in global capture pixels.
    origin: (i32, i32),
}

fn even(v: u32) -> u32 {
    (v & !1).max(2)
}

fn build_source(content: &SCShareableContent, target: &CaptureTarget) -> Result<Source, String> {
    // Leave DemoLens itself out of the recording.
    let me = std::process::id() as i32;
    let apps = content.applications();
    let own: Vec<&SCRunningApplication> = apps.iter().filter(|a| a.process_id() == me).collect();
    match target {
        CaptureTarget::Window { id } => {
            let wanted = id.trim();
            let windows = content.windows();
            let by_id = wanted.parse::<u32>().ok().and_then(|n| windows.iter().find(|w| w.window_id() == n));
            let lower = wanted.to_lowercase();
            let w = by_id
                .or_else(|| {
                    windows.iter().find(|w| {
                        w.is_on_screen()
                            && (w.title().is_some_and(|t| t.to_lowercase().contains(&lower))
                                || w.owning_application().is_some_and(|a| a.application_name().to_lowercase().contains(&lower)))
                    })
                })
                .ok_or_else(|| format!("No window matching \"{wanted}\" was found."))?;
            let f = w.frame();
            let scale = display_info(0, &ordered_displays(content)[0]).scale;
            let filter = SCContentFilter::create().with_window(w).build().map_err(|e| format!("could not build the capture filter: {e}"))?;
            Ok(Source {
                filter,
                width: even((f.size.width * scale).round() as u32),
                height: even((f.size.height * scale).round() as u32),
                origin: ((f.origin.x * scale) as i32, (f.origin.y * scale) as i32),
            })
        }
        other => {
            // Area captures the whole display and ffmpeg crops it.
            let idx = if let CaptureTarget::Screen { display } = other { *display } else { 0 };
            let displays = ordered_displays(content);
            let d = displays.get(idx as usize).ok_or_else(|| {
                format!("Display {idx} was not found. ScreenCaptureKit lists {} display(s).", displays.len())
            })?;
            let info = display_info(idx, d);
            let (bx, by, _, _) = cg::bounds(info.id);
            let mut b = SCContentFilter::create().with_display(d);
            b = if own.is_empty() { b.with_excluding_windows(&[]) } else { b.with_excluding_applications(&own, &[]) };
            let filter = b.build().map_err(|e| format!("could not build the capture filter: {e}"))?;
            Ok(Source {
                filter,
                width: even(info.pixels.0),
                height: even(info.pixels.1),
                origin: ((bx * info.scale) as i32, (by * info.scale) as i32),
            })
        }
    }
}

/// The newest frame, tightly packed BGRA.
#[derive(Default)]
struct Latest {
    data: Vec<u8>,
    w: usize,
    h: usize,
    seq: u64,
}

struct Shared {
    latest: Mutex<Latest>,
    problem: Mutex<Option<String>>,
}

/// A running capture. Dropping it stops the stream.
pub struct Capture {
    shared: Arc<Shared>,
    stop: Option<Sender<()>>,
    /// Size of every frame, known once the first frame arrived.
    pub width: u32,
    pub height: u32,
    /// Top left of the captured rectangle in global capture pixels.
    pub origin: (i32, i32),
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

const BGRA: u32 = 0x4247_5241; // 'BGRA'

fn stream_thread(target: CaptureTarget, fps: u32, shared: Arc<Shared>, ready: Sender<Result<(u32, u32, (i32, i32)), String>>, stop: mpsc::Receiver<()>) {
    let setup = || -> Result<(SCStream, Source), String> {
        let content = shareable()?;
        let src = build_source(&content, &target)?;
        let config = SCStreamConfiguration::new()
            .with_width(src.width)
            .with_height(src.height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(false)
            .with_fps(fps.clamp(1, 60))
            .with_queue_depth(4);
        let mut stream = SCStream::new(&src.filter, &config).map_err(|e| format!("could not create the capture stream: {e}"))?;
        let sh = shared.clone();
        stream
            .add_output_handler(
                move |sample: CMSampleBuffer, ty: SCStreamOutputType| {
                    if ty != SCStreamOutputType::Screen {
                        return;
                    }
                    // Idle and blank frames carry no pixel buffer.
                    let Some(pb) = sample.pixel_buffer() else { return };
                    if pb.pixel_format() != BGRA {
                        *sh.problem.lock().unwrap() = Some(format!("unexpected pixel format {:#x}", pb.pixel_format()));
                        return;
                    }
                    let Ok(guard) = pb.lock_read_only() else { return };
                    let (w, h, stride) = (guard.width(), guard.height(), guard.bytes_per_row());
                    let Some(src) = (unsafe { guard.as_slice() }) else { return };
                    if w == 0 || h == 0 || stride < w * 4 || src.len() < stride * (h - 1) + w * 4 {
                        return;
                    }
                    let mut l = sh.latest.lock().unwrap();
                    if l.seq > 0 && (l.w != w || l.h != h) {
                        return; // the size is fixed once ffmpeg has been told
                    }
                    l.data.resize(w * h * 4, 0);
                    for y in 0..h {
                        l.data[y * w * 4..(y + 1) * w * 4].copy_from_slice(&src[y * stride..y * stride + w * 4]);
                    }
                    l.w = w;
                    l.h = h;
                    l.seq += 1;
                },
                SCStreamOutputType::Screen,
            )
            .map_err(|e| format!("could not attach the frame handler: {e}"))?;
        stream.start_capture().map_err(|e| {
            if cg::has_access() {
                format!("could not start ScreenCaptureKit: {e}")
            } else {
                PERMISSION_MESSAGE.to_string()
            }
        })?;
        Ok((stream, src))
    };
    match setup() {
        Err(e) => {
            let _ = ready.send(Err(e));
        }
        Ok((stream, src)) => {
            let _ = ready.send(Ok((src.width, src.height, src.origin)));
            // Park until told to stop (or the Capture is dropped).
            let _ = stop.recv();
            let _ = stream.stop_capture();
        }
    }
}

/// Starts a capture and waits for the first frame.
pub fn open(target: &CaptureTarget, fps: u32, timeout: Duration) -> Result<Capture, String> {
    let shared = Arc::new(Shared { latest: Mutex::new(Latest::default()), problem: Mutex::new(None) });
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let (t, sh) = (target.clone(), shared.clone());
    std::thread::Builder::new()
        .name("demolens-sck".into())
        .spawn(move || stream_thread(t, fps, sh, ready_tx, stop_rx))
        .map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    // Dropping `cap` on any error below stops the stream.
    let (w, h, origin) = ready_rx.recv_timeout(timeout).map_err(|_| {
        let _ = stop_tx.send(());
        if cg::has_access() { "ScreenCaptureKit did not start in time.".to_string() } else { PERMISSION_MESSAGE.to_string() }
    })??;
    let mut cap = Capture { shared, stop: Some(stop_tx), width: w, height: h, origin };
    loop {
        {
            let l = cap.shared.latest.lock().unwrap();
            if l.seq > 0 {
                cap.width = l.w as u32;
                cap.height = l.h as u32;
                break;
            }
        }
        if let Some(p) = cap.shared.problem.lock().unwrap().clone() {
            return Err(p);
        }
        if t0.elapsed() > timeout {
            return Err(if cg::has_access() {
                "ScreenCaptureKit started but delivered no frames. Is the display asleep or locked?".to_string()
            } else {
                PERMISSION_MESSAGE.to_string()
            });
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(cap)
}

/// Keeps a capture running and writes its newest frame to ffmpeg at a steady
/// frame rate. ScreenCaptureKit only delivers frames when the screen changes;
/// ffmpeg timestamps them by arrival (wall clock), so repeating the newest
/// frame keeps the video moving and in sync with the microphone.
pub struct FeedGuard {
    stop: Arc<AtomicBool>,
    _capture: Capture,
}

impl Drop for FeedGuard {
    /// Does not wait: the writer exits on its next tick and closes ffmpeg's
    /// stdin. A writer stuck on a stalled ffmpeg is released when ffmpeg is
    /// killed.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub fn feed(cap: Capture, mut stdin: ChildStdin, fps: u32) -> FeedGuard {
    let stop = Arc::new(AtomicBool::new(false));
    let (flag, shared) = (stop.clone(), cap.shared.clone());
    let interval = Duration::from_secs_f64(1.0 / fps.clamp(1, 60) as f64);
    std::thread::Builder::new()
        .name("demolens-sck-feed".into())
        .spawn(move || {
            let mut buf: Vec<u8> = Vec::new();
            let mut next = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                {
                    let l = shared.latest.lock().unwrap();
                    buf.clear();
                    buf.extend_from_slice(&l.data);
                }
                if stdin.write_all(&buf).is_err() {
                    break;
                }
                next += interval;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else if now - next > interval * 4 {
                    next = now; // fell behind: do not burst to catch up
                }
            }
            // stdin drops here, which ends ffmpeg's video input.
        })
        .ok();
    FeedGuard { stop, _capture: cap }
}

/// One PNG of display 0 for the area picker: (png, width, height).
pub fn screenshot_png() -> Result<(Vec<u8>, u32, u32), String> {
    let cap = open(&CaptureTarget::Screen { display: 0 }, 5, Duration::from_secs(8))?;
    let (w, h, rgba) = {
        let l = cap.shared.latest.lock().unwrap();
        let mut rgba = l.data.clone();
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        (l.w as u32, l.h as u32, rgba)
    };
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok((out, w, h))
}

// ---------- Source picker: displays, windows, previews, app icons ----------

use super::sources::{encode_png, sort_sources, SourceInfo, Thumb, THUMB_W};

/// Displays and on screen windows of normal apps (not DemoLens, menu bar
/// items or tiny helper windows).
pub fn list_sources() -> Result<Vec<SourceInfo>, String> {
    let content = shareable()?;
    let me = std::process::id() as i32;
    let mut v = Vec::new();
    for (i, d) in ordered_displays(&content).iter().enumerate() {
        let info = display_info(i as u32, d);
        let title = if info.main { "Main display".to_string() } else { format!("Display {}", i + 1) };
        v.push(SourceInfo {
            kind: "display".into(),
            id: i.to_string(),
            title,
            app: String::new(),
            bundle: String::new(),
            x: 0,
            y: 0,
            w: info.pixels.0,
            h: info.pixels.1,
            z: 0,
        });
    }
    let scale = ordered_displays(&content).first().map(|d| display_info(0, d).scale).unwrap_or(1.0);
    // ScreenCaptureKit returns windows front to back, so the index is the stacking order.
    for (z, w) in content.windows().iter().enumerate() {
        let f = w.frame();
        if !w.is_on_screen() || w.window_layer() != 0 || f.size.width < 120.0 || f.size.height < 80.0 {
            continue;
        }
        let Some(app) = w.owning_application() else { continue };
        if app.process_id() == me {
            continue;
        }
        let name = app.application_name();
        if name.is_empty() {
            continue;
        }
        let title = w.title().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| name.clone());
        v.push(SourceInfo {
            kind: "window".into(),
            id: w.window_id().to_string(),
            title,
            app: name,
            bundle: app.bundle_identifier(),
            x: (f.origin.x * scale) as i32,
            y: (f.origin.y * scale) as i32,
            w: (f.size.width * scale) as u32,
            h: (f.size.height * scale) as u32,
            z: z as u32,
        });
    }
    sort_sources(&mut v);
    Ok(v)
}

/// A preview of one display or window (needs macOS 14 or newer; older
/// systems get a placeholder in the picker) and the app icon.
pub fn thumbnail(src: &SourceInfo) -> Result<Thumb, String> {
    use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};
    let content = shareable()?;
    let (filter, w, h) = if src.kind == "display" {
        let idx: usize = src.id.parse().unwrap_or(0);
        let displays = ordered_displays(&content);
        let d = displays.get(idx).ok_or("display not found")?;
        let info = display_info(idx as u32, d);
        let me = std::process::id() as i32;
        let apps = content.applications();
        let own: Vec<&SCRunningApplication> = apps.iter().filter(|a| a.process_id() == me).collect();
        let mut b = SCContentFilter::create().with_display(d);
        b = if own.is_empty() { b.with_excluding_windows(&[]) } else { b.with_excluding_applications(&own, &[]) };
        (b.build().map_err(|e| e.to_string())?, info.pixels.0, info.pixels.1)
    } else {
        let id: u32 = src.id.parse().map_err(|_| "bad window id".to_string())?;
        let windows = content.windows();
        let win = windows.iter().find(|w| w.window_id() == id).ok_or("window is gone")?;
        let f = win.frame();
        (SCContentFilter::create().with_window(win).build().map_err(|e| e.to_string())?, f.size.width.max(1.0) as u32, f.size.height.max(1.0) as u32)
    };
    let tw = THUMB_W;
    let th = even((tw as f64 * h as f64 / w.max(1) as f64).round() as u32);
    let config = SCStreamConfiguration::new().with_width(tw).with_height(th).with_pixel_format(PixelFormat::BGRA).with_shows_cursor(false);
    let img = SCScreenshotManager::capture_image(&filter, &config).map_err(|e| format!("no preview: {e}"))?;
    let (iw, ih) = (img.width() as u32, img.height() as u32);
    let rgba = img.rgba_data().map_err(|e| e.to_string())?;
    let image = encode_png(&rgba, iw, ih).map_err(|e| e.to_string())?;
    let icon = if src.kind == "window" { app_icon(&src.bundle) } else { None };
    Ok(Thumb { image: Some(image), icon })
}

/// The app's icon as PNG, through system tools (mdfind finds the bundle,
/// sips converts the .icns). Cached for the session.
fn app_icon(bundle: &str) -> Option<Vec<u8>> {
    use std::collections::HashMap;
    use std::process::Command;
    static CACHE: Mutex<Option<HashMap<String, Option<Vec<u8>>>>> = Mutex::new(None);
    if bundle.is_empty() || !bundle.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') {
        return None;
    }
    if let Some(hit) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(bundle) {
        return hit.clone();
    }
    let find = || -> Option<Vec<u8>> {
        let out = Command::new("/usr/bin/mdfind").arg(format!("kMDItemCFBundleIdentifier == '{bundle}'")).output().ok()?;
        let app = String::from_utf8_lossy(&out.stdout).lines().find(|l| l.ends_with(".app"))?.to_string();
        let plist = format!("{app}/Contents/Info");
        let name = Command::new("/usr/bin/defaults").args(["read", &plist, "CFBundleIconFile"]).output().ok()?;
        let name = String::from_utf8_lossy(&name.stdout).trim().to_string();
        if name.is_empty() {
            return None;
        }
        let name = if name.ends_with(".icns") { name } else { format!("{name}.icns") };
        let icns = format!("{app}/Contents/Resources/{name}");
        let tmp = std::env::temp_dir().join(format!("demolens-icon-{}-{bundle}.png", std::process::id()));
        let ok = Command::new("/usr/bin/sips").args(["-s", "format", "png", "-Z", "64", &icns, "--out"]).arg(&tmp).output().ok()?.status.success();
        let bytes = if ok { std::fs::read(&tmp).ok() } else { None };
        let _ = std::fs::remove_file(&tmp);
        bytes
    };
    let r = find();
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(bundle.to_string(), r.clone());
    r
}
