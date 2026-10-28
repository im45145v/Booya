//! Input logging for auto zoom.
//!
//! * **Mouse** position and clicks are *polled* on every OS (CoreGraphics on
//!   macOS, XQueryPointer on X11, GetCursorPos on Windows). Polling needs no
//!   global hook and no special permission, and stops when recording stops.
//! * **Keys** are logged only when the user opts in (`KeyLogging::On`), and
//!   never as characters: printable keys become the class
//!   [`crate::events::PRINTABLE`]; only named keys (Enter, Tab, arrows) and
//!   shortcuts with Ctrl/Alt/Meta keep their name. On Linux and Windows the
//!   global keyboard hook (rdev) runs in a short lived child process
//!   (`<exe> --demolens-keyhook`) that classifies keys before they leave it,
//!   and is killed when recording stops or pauses, so no hook outlives a
//!   recording. macOS has no hook at all: a poller reads the key state
//!   through CoreGraphics (needs Input Monitoring, nothing else), and only
//!   while recording with key logging on.

use crate::capture::Region;
use crate::events::Event;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Whether key presses are logged at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyLogging {
    /// No key events, and no keyboard hook is installed. The default.
    #[default]
    Off,
    /// Shortcuts, named keys and the *timing* of typing (no characters).
    On,
}

impl KeyLogging {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "no" | "false" => Some(Self::Off),
            "on" | "yes" | "true" | "masked" | "timing" => Some(Self::On),
            _ => None,
        }
    }
}

/// True on macOS when key logging was asked for but Input Monitoring is not
/// granted, so typing and shortcuts would silently read as "no keys". With
/// `prompt` the system permission dialog is shown (once).
pub fn key_permission_missing(prompt: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        !mac::input_monitoring(prompt)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        false
    }
}

/// Reduce one key press to what auto zoom and the shortcut overlay need.
/// A printable key without Ctrl/Alt/Meta becomes [`crate::events::PRINTABLE`].
pub fn classify(key: &str, modifiers: &[String]) -> String {
    let shortcut = modifiers.iter().any(|m| !m.eq_ignore_ascii_case("shift"));
    let key = key.strip_prefix("Key").filter(|k| k.len() == 1).unwrap_or(key);
    let key = key.strip_prefix("Num").filter(|k| k.len() == 1 && k.chars().all(|c| c.is_ascii_digit())).unwrap_or(key);
    if key.chars().count() == 1 {
        let c = key.chars().next().unwrap();
        if c.is_whitespace() {
            return "Space".into();
        }
        return if shortcut { c.to_uppercase().collect() } else { crate::events::PRINTABLE.into() };
    }
    key.to_string()
}

struct Shared {
    epoch: Option<Instant>,
    events: Vec<(Instant, crate::events::EventKind)>,
    active: bool,
    /// Start of the current pause, if paused.
    paused_at: Option<Instant>,
    pauses: Vec<(Instant, Instant)>,
    /// Last cursor position seen, relative to the recorded region.
    last_pos: (f64, f64),
}

impl Shared {
    fn push(&mut self, now: Instant, kind: crate::events::EventKind) {
        if self.active && self.paused_at.is_none() {
            self.events.push((now, kind));
        }
    }
}

pub struct InputLogger {
    shared: Arc<Mutex<Shared>>,
    keys: KeyLogging,
    hook: Option<std::process::Child>,
    poller: Option<std::thread::JoinHandle<()>>,
    /// macOS key state poller: a stop flag and its thread.
    keypoll: Option<(Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>)>,
}

/// What the logger saw, in ms since the epoch.
pub struct Logged {
    pub events: Vec<Event>,
    /// Paused stretches (start, end), cut out of the project automatically.
    pub pauses: Vec<(f64, f64)>,
}

impl InputLogger {
    pub fn start(region: Region, keys: KeyLogging) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared { epoch: None, events: Vec::new(), active: true, paused_at: None, pauses: Vec::new(), last_pos: (region.w as f64 / 2.0, region.h as f64 / 2.0) }));
        let poller = poll::spawn(shared.clone(), region);
        let mut me = Self { shared, keys, hook: None, poller, keypoll: None };
        me.start_hook();
        Ok(me)
    }

    /// A logger that records nothing (used as a placeholder).
    pub fn idle() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared { epoch: None, events: Vec::new(), active: false, paused_at: None, pauses: Vec::new(), last_pos: (0.0, 0.0) })),
            keys: KeyLogging::Off,
            hook: None,
            poller: None,
            keypoll: None,
        }
    }

    fn start_hook(&mut self) {
        if self.keys == KeyLogging::Off {
            return;
        }
        #[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
        {
            self.hook = keyhook::spawn(self.shared.clone());
        }
        #[cfg(target_os = "macos")]
        {
            self.keypoll = mac_keys::spawn(self.shared.clone());
        }
    }

    fn stop_hook(&mut self) {
        if let Some(mut c) = self.hook.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some((stop, h)) = self.keypoll.take() {
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = h.join();
        }
    }

    /// Is the keyboard hook process running right now?
    pub fn hook_running(&mut self) -> bool {
        self.keypoll.as_ref().is_some_and(|(_, h)| !h.is_finished()) || self.hook.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    pub fn set_epoch(&self, t: Instant) {
        self.shared.lock().unwrap().epoch = Some(t);
    }

    /// Pause or resume logging. Pausing also kills the keyboard hook.
    pub fn set_paused(&mut self, paused: bool) {
        let now = Instant::now();
        {
            let mut s = self.shared.lock().unwrap();
            match (paused, s.paused_at) {
                (true, None) => s.paused_at = Some(now),
                (false, Some(a)) => {
                    s.pauses.push((a, now));
                    s.paused_at = None;
                }
                _ => return,
            }
        }
        if paused {
            self.stop_hook();
        } else {
            self.start_hook();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.shared.lock().unwrap().paused_at.is_some()
    }

    /// Drop a marker at the cursor's current position. Returns false (and
    /// records nothing) while paused or not recording.
    pub fn mark(&self) -> bool {
        let mut s = self.shared.lock().unwrap();
        if !s.active || s.paused_at.is_some() {
            return false;
        }
        let (x, y) = s.last_pos;
        s.push(Instant::now(), crate::events::EventKind::Mark { x, y });
        true
    }

    /// Total time spent paused so far (finished pauses plus the current one).
    pub fn paused_total(&self) -> std::time::Duration {
        let s = self.shared.lock().unwrap();
        let done: std::time::Duration = s.pauses.iter().map(|(a, b)| b.saturating_duration_since(*a)).sum();
        done + s.paused_at.map(|a| a.elapsed()).unwrap_or_default()
    }

    pub fn stop(mut self) -> Logged {
        self.stop_hook();
        let mut s = self.shared.lock().unwrap();
        s.active = false;
        let now = Instant::now();
        if let Some(a) = s.paused_at.take() {
            s.pauses.push((a, now));
        }
        let epoch = s.epoch.unwrap_or(now);
        let ms = |t: Instant| t.saturating_duration_since(epoch).as_secs_f64() * 1000.0;
        let events = s
            .events
            .drain(..)
            .filter(|(t, _)| *t >= epoch)
            .map(|(t, kind)| Event { t: ms(t), kind })
            .collect();
        let pauses = s.pauses.iter().map(|&(a, b)| (ms(a), ms(b))).collect();
        drop(s);
        if let Some(h) = self.poller.take() {
            let _ = h.join();
        }
        Logged { events, pauses }
    }
}

impl Drop for InputLogger {
    fn drop(&mut self) {
        self.stop_hook();
        self.shared.lock().unwrap().active = false;
    }
}

/// Call first thing in `main`. When the process was started as the keyboard
/// hook helper it runs the hook and never returns.
pub fn run_keyhook_if_requested() {
    if std::env::args().nth(1).as_deref() != Some(KEYHOOK_ARG) {
        return;
    }
    #[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
    keyhook::run();
    #[allow(unreachable_code)]
    std::process::exit(0);
}

pub const KEYHOOK_ARG: &str = "--demolens-keyhook";

#[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
mod keyhook {
    use super::*;
    use crate::events::EventKind;
    use rdev::{listen, EventType, Key};
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    pub fn spawn(shared: Arc<Mutex<Shared>>) -> Option<std::process::Child> {
        let exe = std::env::current_exe().ok()?;
        let mut cmd = Command::new(exe);
        cmd.arg(KEYHOOK_ARG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn().map_err(|e| eprintln!("demolens: key logging unavailable: {e}")).ok()?;
        let out = child.stdout.take()?;
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                let now = Instant::now();
                let mut parts = line.splitn(2, '\t');
                let key = parts.next().unwrap_or_default().to_string();
                let modifiers: Vec<String> = parts.next().unwrap_or_default().split(',').filter(|m| !m.is_empty()).map(String::from).collect();
                if key.is_empty() {
                    continue;
                }
                // Classify again on this side: the helper is our own binary,
                // but the log must never hold a character either way.
                let key = classify(&key, &modifiers);
                shared.lock().unwrap().push(now, EventKind::Key { key, modifiers });
            }
        });
        Some(child)
    }

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
            Key::MetaLeft | Key::MetaRight => Some("Meta"),
            _ => None,
        }
    }

    /// The helper process: classify every key press and write one line per
    /// key to stdout. Exits as soon as the parent closes stdin or dies.
    pub fn run() {
        std::thread::spawn(|| {
            let mut sink = String::new();
            while std::io::stdin().read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
                sink.clear();
            }
            std::process::exit(0);
        });
        let mut mods: Vec<&'static str> = Vec::new();
        let res = listen(move |ev| match ev.event_type {
            EventType::KeyPress(k) => {
                if let Some(m) = modifier(k) {
                    if !mods.contains(&m) {
                        mods.push(m);
                    }
                    return;
                }
                let modifiers: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
                let key = classify(&key_label(k, ev.name.clone()), &modifiers);
                let mut out = std::io::stdout().lock();
                if writeln!(out, "{key}\t{}", modifiers.join(",")).and_then(|_| out.flush()).is_err() {
                    std::process::exit(0);
                }
            }
            EventType::KeyRelease(k) => {
                if let Some(m) = modifier(k) {
                    mods.retain(|x| *x != m);
                }
            }
            _ => {}
        });
        if let Err(e) = res {
            eprintln!("demolens: keyboard hook unavailable: {e:?}");
        }
    }
}

/// Mouse polling, shared by every OS. `pointer()` returns the position in
/// physical screen pixels and the left/right/middle button state.
mod poll {
    use super::*;
    use crate::events::{EventKind, MouseButton};
    use std::time::Duration;

    pub fn spawn(shared: Arc<Mutex<Shared>>, region: Region) -> Option<std::thread::JoinHandle<()>> {
        let mut src = platform::Pointer::open()?;
        Some(std::thread::spawn(move || {
            let mut last = (f64::NAN, f64::NAN);
            let mut down = [false; 3];
            let mut last_change = Instant::now();
            loop {
                if !shared.lock().unwrap().active {
                    break;
                }
                let now = Instant::now();
                let Some((mx, my, buttons)) = src.read() else {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                };
                let pos = (mx - region.x as f64, my - region.y as f64);
                let mut evs = Vec::new();
                if pos != last {
                    evs.push(EventKind::Move { x: pos.0, y: pos.1 });
                    last = pos;
                }
                for (i, b) in [MouseButton::Left, MouseButton::Right, MouseButton::Middle].into_iter().enumerate() {
                    if buttons[i] && !down[i] {
                        evs.push(EventKind::Click { x: pos.0, y: pos.1, button: b });
                    }
                    down[i] = buttons[i];
                }
                if !evs.is_empty() {
                    last_change = now;
                    let mut s = shared.lock().unwrap();
                    s.last_pos = pos;
                    for k in evs {
                        s.push(now, k);
                    }
                }
                // 60 Hz while the mouse is in use, 20 Hz once it has been still
                // for half a second (a click still lasts longer than 50 ms).
                let idle = now.duration_since(last_change) > Duration::from_millis(500);
                std::thread::sleep(Duration::from_millis(if idle { 50 } else { 16 }));
            }
        }))
    }

    #[cfg(all(target_os = "linux", feature = "input-hooks"))]
    mod platform {
        use x11::xlib;
        pub struct Pointer {
            dpy: *mut xlib::Display,
        }
        // The display connection is only used from the polling thread.
        unsafe impl Send for Pointer {}
        impl Pointer {
            pub fn open() -> Option<Self> {
                let dpy = unsafe { xlib::XOpenDisplay(std::ptr::null()) };
                if dpy.is_null() {
                    eprintln!("demolens: no X11 display, so no cursor log (Wayland is not supported yet)");
                    return None;
                }
                Some(Self { dpy })
            }
            pub fn read(&mut self) -> Option<(f64, f64, [bool; 3])> {
                unsafe {
                    let root = xlib::XDefaultRootWindow(self.dpy);
                    let (mut r, mut c) = (0, 0);
                    let (mut rx, mut ry, mut wx, mut wy) = (0, 0, 0, 0);
                    let mut mask = 0u32;
                    if xlib::XQueryPointer(self.dpy, root, &mut r, &mut c, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask) == 0 {
                        return None;
                    }
                    Some((rx as f64, ry as f64, [mask & xlib::Button1Mask != 0, mask & xlib::Button3Mask != 0, mask & xlib::Button2Mask != 0]))
                }
            }
        }
        impl Drop for Pointer {
            fn drop(&mut self) {
                unsafe { xlib::XCloseDisplay(self.dpy) };
            }
        }
    }

    #[cfg(all(windows, feature = "input-hooks"))]
    mod platform {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        pub struct Pointer;
        impl Pointer {
            pub fn open() -> Option<Self> {
                Some(Self)
            }
            pub fn read(&mut self) -> Option<(f64, f64, [bool; 3])> {
                let mut p = POINT { x: 0, y: 0 };
                if unsafe { GetCursorPos(&mut p) } == 0 {
                    return None;
                }
                let down = |vk: i32| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0;
                // VK_LBUTTON, VK_RBUTTON, VK_MBUTTON
                Some((p.x as f64, p.y as f64, [down(1), down(2), down(4)]))
            }
        }
    }

    #[cfg(target_os = "macos")]
    mod platform {
        pub struct Pointer {
            scale: f64,
        }
        impl Pointer {
            pub fn open() -> Option<Self> {
                Some(Self { scale: super::super::mac::backing_scale() })
            }
            pub fn read(&mut self) -> Option<(f64, f64, [bool; 3])> {
                let (x, y) = super::super::mac::mouse();
                let b = super::super::mac::buttons();
                Some((x * self.scale, y * self.scale, b))
            }
        }
    }

    #[cfg(not(any(target_os = "macos", all(feature = "input-hooks", any(target_os = "linux", windows)))))]
    mod platform {
        pub struct Pointer;
        impl Pointer {
            pub fn open() -> Option<Self> {
                eprintln!("demolens: built without input-hooks; no event log will be captured");
                None
            }
            pub fn read(&mut self) -> Option<(f64, f64, [bool; 3])> {
                None
            }
        }
    }
}

/// macOS pointer access through CoreGraphics. Needs no Accessibility or
/// Input Monitoring permission. Key state is polled here too, see `mac_keys`.
#[cfg(target_os = "macos")]
mod mac {
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
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
        fn CGEventSourceFlagsState(state: i32) -> u64;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
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
    pub fn backing_scale() -> f64 {
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

    pub fn mouse() -> (f64, f64) {
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

    /// Is the key with this virtual key code held right now (hardware state)?
    pub fn key_down(code: u16) -> bool {
        unsafe { CGEventSourceKeyState(1, code) }
    }

    /// Modifier flags of the hardware state. The hardware (HID) state sees
    /// what was physically pressed, including combos the system keeps for
    /// itself such as Cmd+Tab and Cmd+Space, which never reach an app or a
    /// session level event tap. The combined session state does not.
    pub fn flags() -> u64 {
        unsafe { CGEventSourceFlagsState(1) }
    }

    /// Whether Input Monitoring is granted. Without it key state reads as "up".
    pub fn input_monitoring(prompt: bool) -> bool {
        unsafe {
            if CGPreflightListenEventAccess() {
                return true;
            }
            if prompt {
                CGRequestListenEventAccess();
            }
            false
        }
    }

    pub fn buttons() -> [bool; 3] {
        // 0 = combined session state; buttons 0, 1, 2 = left, right, middle.
        [0u32, 1, 2].map(|i| unsafe { CGEventSourceButtonState(0, i) })
    }
}

/// US layout names for macOS virtual key codes. Names only, never what the
/// user typed with another layout, and no call into the text input services
/// (they are what made the old hook crash). Modifier keys have no entry.
pub mod mackeys {
    pub fn name(code: u16) -> Option<&'static str> {
        Some(match code {
            0 => "a", 1 => "s", 2 => "d", 3 => "f", 4 => "h", 5 => "g", 6 => "z", 7 => "x", 8 => "c", 9 => "v",
            11 => "b", 12 => "q", 13 => "w", 14 => "e", 15 => "r", 16 => "y", 17 => "t",
            18 => "1", 19 => "2", 20 => "3", 21 => "4", 22 => "6", 23 => "5", 24 => "=", 25 => "9", 26 => "7",
            27 => "-", 28 => "8", 29 => "0", 30 => "]", 31 => "o", 32 => "u", 33 => "[", 34 => "i", 35 => "p",
            36 | 76 => "Enter", 37 => "l", 38 => "j", 39 => "'", 40 => "k", 41 => ";", 42 => "\\", 43 => ",", 44 => "/",
            45 => "n", 46 => "m", 47 => ".", 48 => "Tab", 49 => "Space", 50 => "`", 51 => "Backspace", 53 => "Esc",
            117 => "Delete", 123 => "Left", 124 => "Right", 125 => "Down", 126 => "Up",
            122 => "F1", 120 => "F2", 99 => "F3", 118 => "F4", 96 => "F5", 97 => "F6", 98 => "F7", 100 => "F8",
            101 => "F9", 109 => "F10", 103 => "F11", 111 => "F12",
            _ => return None,
        })
    }

    /// Modifier names from CoreGraphics event flags.
    pub fn modifiers(flags: u64) -> Vec<String> {
        let mut m = Vec::new();
        for (bit, n) in [(0x40000u64, "Ctrl"), (0x80000, "Alt"), (0x100000, "Meta"), (0x20000, "Shift")] {
            if flags & bit != 0 {
                m.push(n.to_string());
            }
        }
        m
    }

    /// Codes worth polling.
    pub const CODES: [u16; 62] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
        46, 47, 48, 49, 50, 51, 53, 76, 117, 123, 124, 125, 126, 122, 120, 99, 118,
    ];
}

/// Polls the key state on macOS. Only runs while key logging is on and
/// recording is active; it records key *classes* (see [`classify`]).
#[cfg(target_os = "macos")]
mod mac_keys {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub fn spawn(shared: Arc<Mutex<Shared>>) -> Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)> {
        if !mac::input_monitoring(true) {
            eprintln!("demolens: Input Monitoring is not granted, so typing and shortcuts are not logged (System Settings > Privacy & Security > Input Monitoring)");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        // F5 to F12 are added here so the base table stays one sorted list.
        let mut codes: Vec<u16> = mackeys::CODES.to_vec();
        codes.extend([96u16, 97, 98, 100, 101, 109, 103, 111]);
        let h = std::thread::spawn(move || {
            let mut down = std::collections::HashSet::<u16>::new();
            while !flag.load(Ordering::SeqCst) {
                if !shared.lock().unwrap().active {
                    break;
                }
                let now = Instant::now();
                let mods = mackeys::modifiers(mac::flags());
                for &c in &codes {
                    let d = mac::key_down(c);
                    if d && down.insert(c) {
                        if let Some(n) = mackeys::name(c) {
                            let key = classify(n, &mods);
                            shared.lock().unwrap().push(now, crate::events::EventKind::Key { key, modifiers: mods.clone() });
                        }
                    } else if !d {
                        down.remove(&c);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(12));
            }
        });
        Some((stop, h))
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, KeyLogging};
    use crate::events::PRINTABLE;
    #[test]
    fn mac_key_codes_map_to_names_and_modifiers() {
        use super::mackeys;
        assert_eq!(mackeys::name(0), Some("a"));
        assert_eq!(mackeys::name(36), Some("Enter"));
        assert_eq!(mackeys::name(55), None, "modifier keys are read from flags, not codes");
        assert_eq!(mackeys::modifiers(0x100000 | 0x20000), vec!["Meta".to_string(), "Shift".to_string()]);
        // Cmd+S is a shortcut, plain s is just "typing".
        assert_eq!(classify("s", &mackeys::modifiers(0x100000)), "S");
        assert_eq!(classify("s", &[]), PRINTABLE);
        for c in mackeys::CODES {
            assert!(mackeys::name(c).is_some(), "code {c} has no name");
        }
    }

    #[test]
    fn keys_are_classified_never_stored_as_characters() {
        assert_eq!(classify("p", &[]), PRINTABLE);
        assert_eq!(classify("P", &["Shift".into()]), PRINTABLE);
        assert_eq!(classify("7", &[]), PRINTABLE);
        assert_eq!(classify("s", &["Ctrl".into()]), "S");
        assert_eq!(classify("KeyS", &["Ctrl".into()]), "S");
        assert_eq!(classify("Enter", &[]), "Enter");
        assert_eq!(classify(" ", &[]), "Space");
        assert_eq!(KeyLogging::default(), KeyLogging::Off);
        assert_eq!(KeyLogging::parse("on"), Some(KeyLogging::On));
    }
}
