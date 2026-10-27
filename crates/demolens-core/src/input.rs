//! Global input logger. With the `input-hooks` feature it uses rdev to watch
//! mouse and keyboard system wide (X11 on Linux, CGEventTap on macOS which
//! needs Accessibility permission, low level hooks on Windows). Without the
//! feature it records nothing and the editor simply has no auto zoom.

use crate::capture::Region;
use crate::events::Event;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Instant;

struct Shared {
    epoch: Option<Instant>,
    events: Vec<(Instant, crate::events::EventKind)>,
    active: bool,
}

pub struct InputLogger {
    shared: Arc<Mutex<Shared>>,
}

impl InputLogger {
    pub fn start(region: Region) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared { epoch: None, events: Vec::new(), active: true }));
        #[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
        hooks::spawn(shared.clone(), region);
        #[cfg(target_os = "macos")]
        mac::spawn(shared.clone(), region);
        #[cfg(all(not(feature = "input-hooks"), not(target_os = "macos")))]
        {
            let _ = region;
            eprintln!("demolens: built without input-hooks; no event log will be captured");
        }
        Ok(Self { shared })
    }

    pub fn set_epoch(&self, t: Instant) {
        self.shared.lock().unwrap().epoch = Some(t);
    }

    pub fn stop(self) -> Vec<Event> {
        let mut s = self.shared.lock().unwrap();
        s.active = false;
        let epoch = s.epoch.unwrap_or_else(Instant::now);
        s.events
            .drain(..)
            .filter(|(t, _)| *t >= epoch)
            .map(|(t, kind)| Event { t: (t - epoch).as_secs_f64() * 1000.0, kind })
            .collect()
    }
}

#[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
mod hooks {
    use super::*;
    use crate::events::{EventKind, MouseButton};
    use rdev::{listen, Button, EventType, Key};

    fn key_label(k: Key, name: Option<String>) -> String {
        match k {
            Key::Return | Key::KpReturn => "Enter".into(),
            Key::Space => "Space".into(),
            Key::Backspace => "Backspace".into(),
            Key::Tab => "Tab".into(),
            Key::Escape => "Esc".into(),
            Key::Delete => "Delete".into(),
            Key::UpArrow => "Up".into(),
            Key::DownArrow => "Down".into(),
            Key::LeftArrow => "Left".into(),
            Key::RightArrow => "Right".into(),
            _ => match name {
                Some(n) if !n.trim().is_empty() && !n.chars().any(|c| c.is_control()) => n,
                _ => format!("{k:?}"),
            },
        }
    }

    fn modifier(k: Key) -> Option<&'static str> {
        match k {
            Key::ControlLeft | Key::ControlRight => Some("Ctrl"),
            Key::ShiftLeft | Key::ShiftRight => Some("Shift"),
            Key::Alt | Key::AltGr => Some("Alt"),
            Key::MetaLeft | Key::MetaRight => Some(if cfg!(target_os = "macos") { "Cmd" } else { "Meta" }),
            _ => None,
        }
    }

    pub fn spawn(shared: Arc<Mutex<Shared>>, region: Region) {
        std::thread::spawn(move || {
            let mut pos = (0.0, 0.0);
            let mut mods: Vec<&'static str> = Vec::new();
            let res = listen(move |ev| {
                let now = Instant::now();
                let kind = match ev.event_type {
                    EventType::MouseMove { x, y } => {
                        pos = (x - region.x as f64, y - region.y as f64);
                        Some(EventKind::Move { x: pos.0, y: pos.1 })
                    }
                    EventType::ButtonPress(b) => Some(EventKind::Click {
                        x: pos.0,
                        y: pos.1,
                        button: match b {
                            Button::Right => MouseButton::Right,
                            Button::Middle => MouseButton::Middle,
                            _ => MouseButton::Left,
                        },
                    }),
                    EventType::KeyPress(k) => {
                        if let Some(m) = modifier(k) {
                            if !mods.contains(&m) {
                                mods.push(m);
                            }
                            None
                        } else {
                            Some(EventKind::Key { key: key_label(k, ev.name.clone()), modifiers: mods.iter().map(|m| m.to_string()).collect() })
                        }
                    }
                    EventType::KeyRelease(k) => {
                        if let Some(m) = modifier(k) {
                            mods.retain(|x| *x != m);
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(kind) = kind {
                    let mut s = shared.lock().unwrap();
                    if s.active {
                        s.events.push((now, kind));
                    }
                }
            });
            if let Err(e) = res {
                eprintln!("demolens: input hooks unavailable: {e:?}");
            }
        });
    }
}

/// macOS: polls the mouse position and button state 60 times a second through
/// CoreGraphics. This needs no Accessibility or Input Monitoring permission
/// and never blocks. Key presses are not logged on macOS yet (that needs Input
/// Monitoring), so auto zoom there comes from clicks.
#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use crate::events::{EventKind, MouseButton};
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
        fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
        fn CGDisplayModeRelease(mode: *mut c_void);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    /// Points to pixels factor of the main display (2.0 on Retina).
    fn backing_scale() -> f64 {
        unsafe {
            let id = CGMainDisplayID();
            let b = CGDisplayBounds(id);
            let mode = CGDisplayCopyDisplayMode(id);
            if mode.is_null() || b.size.width <= 0.0 {
                return 1.0;
            }
            let px = CGDisplayModeGetPixelWidth(mode) as f64;
            CGDisplayModeRelease(mode);
            if px > 0.0 { px / b.size.width } else { 1.0 }
        }
    }

    fn mouse() -> (f64, f64) {
        unsafe {
            let ev = CGEventCreate(std::ptr::null());
            if ev.is_null() {
                return (0.0, 0.0);
            }
            let p = CGEventGetLocation(ev);
            CFRelease(ev);
            (p.x, p.y)
        }
    }

    pub fn spawn(shared: Arc<Mutex<Shared>>, region: Region) {
        std::thread::spawn(move || {
            let scale = backing_scale();
            let mut last = (f64::NAN, f64::NAN);
            let mut down = [false; 3];
            let mut last_change = Instant::now();
            loop {
                if !shared.lock().unwrap().active {
                    break;
                }
                let now = Instant::now();
                let (mx, my) = mouse();
                let pos = (mx * scale - region.x as f64, my * scale - region.y as f64);
                let mut evs = Vec::new();
                if pos != last {
                    evs.push(EventKind::Move { x: pos.0, y: pos.1 });
                    last = pos;
                }
                for (i, b) in [MouseButton::Left, MouseButton::Right, MouseButton::Middle].into_iter().enumerate() {
                    // 0 = combined session state
                    let is = unsafe { CGEventSourceButtonState(0, i as u32) };
                    if is && !down[i] {
                        evs.push(EventKind::Click { x: pos.0, y: pos.1, button: b });
                    }
                    down[i] = is;
                }
                if !evs.is_empty() {
                    last_change = now;
                    let mut s = shared.lock().unwrap();
                    for k in evs {
                        s.events.push((now, k));
                    }
                }
                // 60 Hz while the mouse is in use, 20 Hz once it has been still
                // for half a second (a click still lasts longer than 50 ms).
                let idle = now.duration_since(last_change) > std::time::Duration::from_millis(500);
                std::thread::sleep(std::time::Duration::from_millis(if idle { 50 } else { 16 }));
            }
        });
    }
}
