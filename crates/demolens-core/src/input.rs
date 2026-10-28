//! Input logging for auto zoom.
//!
//! * **Mouse** position and clicks are *polled* on every OS (CoreGraphics on
//!   macOS, XQueryPointer on X11, GetCursorPos on Windows). Polling needs no
//!   global hook and no special permission, and stops when recording stops.
//! * **Keys** are logged only when the user opts in (`KeyLogging::On`), and
//!   never as characters, except as part of a shortcut: a printable key
//!   typed with Ctrl/Alt/Meta held keeps its name (Cmd+W is stored as W),
//!   and without one becomes the class [`crate::events::PRINTABLE`]. Named
//!   keys (Enter, Tab, arrows) keep their name. On Linux and Windows the
//!   global keyboard hook (rdev) runs in a short lived child process
//!   (`<exe> --demolens-keyhook`) that classifies keys before they leave it,
//!   and is killed when recording stops or pauses, so no hook outlives a
//!   recording. macOS uses a listen-only CGEventTap (needs Input Monitoring,
//!   nothing else), with a key state poller as fallback, and only while
//!   recording with key logging on.

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
    /// Shortcuts (with their key), named keys and the *timing* of typing (no
    /// characters outside shortcuts).
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
    let shortcut = crate::keycombo::makes_shortcut(modifiers);
    let key = key.strip_prefix("Key").filter(|k| k.len() == 1).unwrap_or(key);
    let key = key.strip_prefix("Num").filter(|k| k.len() == 1 && k.chars().all(|c| c.is_ascii_digit())).unwrap_or(key);
    if key.chars().count() == 1 {
        let c = key.chars().next().unwrap();
        if c.is_whitespace() {
            return "Space".into();
        }
        return if shortcut { c.to_uppercase().collect() } else { crate::events::PRINTABLE.into() };
    }
    // Named keys keep one spelling whatever source they came from.
    crate::keylabels::canonical_key(key)
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
    /// Finger and pen input (Windows Raw Input, X11 XInput2).
    touch: Option<crate::touch::Watcher>,
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
        let touch = start_touch(shared.clone(), region);
        let mut me = Self { shared, keys, hook: None, poller, touch, keypoll: None };
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
            touch: None,
            keypoll: None,
        }
    }

    fn start_hook(&mut self) {
        // Off: on Linux and Windows the helper still runs, in wheel only mode,
        // because scrolling cannot be polled there. It never sees a key.
        #[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
        {
            self.hook = keyhook::spawn(self.shared.clone(), self.keys != KeyLogging::Off);
        }
        if self.keys == KeyLogging::Off {
            return;
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
        self.touch.take();
        let mut events: Vec<Event> = events;
        crate::touch::drop_emulated_mouse(&mut events);
        Logged { events, pauses }
    }
}

/// Logs taps and drags from the touch screen as touch clicks.
fn start_touch(shared: Arc<Mutex<Shared>>, region: Region) -> Option<crate::touch::Watcher> {
    use crate::events::{EventKind, MouseButton};
    use crate::touch::Phase;
    crate::touch::start(Box::new(move |phase, x, y| {
        let now = Instant::now();
        let (x, y) = (x - region.x as f64, y - region.y as f64);
        let mut s = shared.lock().unwrap();
        s.last_pos = (x, y);
        match phase {
            Phase::Down => {
                s.push(now, EventKind::Move { x, y });
                s.push(now, EventKind::Click { x, y, button: MouseButton::Touch });
            }
            Phase::Move => s.push(now, EventKind::Move { x, y }),
            Phase::Up => s.push(now, EventKind::Release { x, y, button: MouseButton::Touch }),
        }
    }))
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
/// Second argument of the helper: report scrolling only, never keys.
pub const WHEEL_ONLY_ARG: &str = "wheel-only";

#[cfg(all(feature = "input-hooks", not(target_os = "macos")))]
mod keyhook {
    use super::*;
    use crate::events::EventKind;
    use rdev::{listen, EventType, Key};
    use std::io::{BufRead, Write};
    use std::process::Stdio;

    /// `keys` false starts the helper in wheel only mode: it reports scrolling
    /// and drops every key press the moment it sees one.
    pub fn spawn(shared: Arc<Mutex<Shared>>, keys: bool) -> Option<std::process::Child> {
        let exe = std::env::current_exe().ok()?;
        let mut cmd = crate::proc::command(exe);
        cmd.arg(KEYHOOK_ARG);
        if !keys {
            cmd.arg(WHEEL_ONLY_ARG);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
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
                let mut parts = line.splitn(3, '\t');
                let tag = parts.next().unwrap_or_default();
                let list = |s: Option<&str>| -> Vec<String> { s.unwrap_or_default().split(',').filter(|m| !m.is_empty()).map(String::from).collect() };
                match tag {
                    // `M<TAB>mods`: the set of held modifiers changed.
                    "M" => {
                        let held = list(parts.next());
                        shared.lock().unwrap().push(now, EventKind::Mods { held });
                    }
                    // `S<TAB>dx<TAB>dy`: the wheel or trackpad scrolled.
                    "S" => {
                        let num = |s: Option<&str>| s.and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                        let (dx, dy) = (num(parts.next()), num(parts.next()));
                        let mut g = shared.lock().unwrap();
                        let (x, y) = g.last_pos;
                        g.push(now, EventKind::Scroll { x, y, dx, dy });
                    }
                    // `K<TAB>key<TAB>mods`: a key was pressed.
                    "K" => {
                        let key = parts.next().unwrap_or_default().to_string();
                        let modifiers = list(parts.next());
                        if key.is_empty() {
                            continue;
                        }
                        // Classify again on this side: the helper is our own
                        // binary, but the log must never hold a character.
                        let key = classify(&key, &modifiers);
                        shared.lock().unwrap().push(now, EventKind::Key { key, modifiers });
                    }
                    _ => {}
                }
            }
        });
        Some(child)
    }

    /// Media and extra keys the rdev enum has no name for, by OS key code.
    pub fn unknown_key_name(code: u32) -> Option<String> {
        #[cfg(windows)]
        {
            // Windows virtual-key codes.
            return Some(match code {
                0xAD => "Mute".into(),
                0xAE => "VolumeDown".into(),
                0xAF => "VolumeUp".into(),
                0xB0 => "NextTrack".into(),
                0xB1 => "PrevTrack".into(),
                0xB2 => "MediaStop".into(),
                0xB3 => "PlayPause".into(),
                0x7C..=0x87 => format!("F{}", code - 0x7C + 13),
                _ => return None,
            });
        }
        #[cfg(not(windows))]
        {
            // X11 key codes (evdev code + 8).
            Some(match code {
                121 => "Mute".into(),
                122 => "VolumeDown".into(),
                123 => "VolumeUp".into(),
                171 => "NextTrack".into(),
                172 => "PlayPause".into(),
                173 => "PrevTrack".into(),
                174 => "MediaStop".into(),
                232 => "BrightnessDown".into(),
                233 => "BrightnessUp".into(),
                191..=202 => format!("F{}", code - 191 + 13),
                _ => return None,
            })
        }
    }

    /// The name of a non-modifier key, or None for keys we do not log.
    pub fn key_label(k: Key, name: Option<String>) -> Option<String> {
        Some(match k {
            Key::Return | Key::KpReturn => "Enter".into(),
            Key::Space => "Space".into(),
            Key::Backspace => "Backspace".into(),
            Key::Tab => "Tab".into(),
            Key::Escape => "Esc".into(),
            Key::Delete | Key::KpDelete => "Delete".into(),
            Key::UpArrow => "Up".into(),
            Key::DownArrow => "Down".into(),
            Key::LeftArrow => "Left".into(),
            Key::RightArrow => "Right".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::PageUp => "PageUp".into(),
            Key::PageDown => "PageDown".into(),
            Key::Insert => "Insert".into(),
            Key::CapsLock => "CapsLock".into(),
            Key::NumLock => "NumLock".into(),
            Key::ScrollLock => "ScrollLock".into(),
            Key::PrintScreen => "PrintScreen".into(),
            Key::Pause => "Pause".into(),
            Key::Unknown(code) => return unknown_key_name(code),
            _ => match name {
                Some(n) if !n.trim().is_empty() && !n.chars().any(|c| c.is_control()) => n,
                _ => match symbol(k) {
                    Some(sym) => sym.into(),
                    None => format!("{k:?}"),
                },
            },
        })
    }

    /// The character printed on a punctuation or keypad key, for shortcuts
    /// (Ctrl+, or Win+= have no text of their own in the hook).
    fn symbol(k: Key) -> Option<&'static str> {
        Some(match k {
            Key::Minus | Key::KpMinus => "-",
            Key::Equal => "=",
            Key::Comma => ",",
            Key::Dot => ".",
            Key::Slash | Key::KpDivide => "/",
            Key::BackSlash | Key::IntlBackslash => "\\",
            Key::LeftBracket => "[",
            Key::RightBracket => "]",
            Key::Quote => "'",
            Key::SemiColon => ";",
            Key::BackQuote => "`",
            Key::KpPlus => "+",
            Key::KpMultiply => "*",
            _ => return None,
        })
    }

    /// The modifier a key is, if it is one. AltGr is left out: on Windows it
    /// is sent as Ctrl+Alt and would read as a shortcut while typing.
    pub fn modifier(k: Key) -> Option<&'static str> {
        match k {
            Key::ControlLeft | Key::ControlRight => Some("Ctrl"),
            Key::ShiftLeft | Key::ShiftRight => Some("Shift"),
            Key::Alt => Some("Alt"),
            Key::MetaLeft | Key::MetaRight => Some("Meta"),
            Key::Function => Some("Fn"),
            _ => None,
        }
    }

    fn emit(line: &str) {
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            std::process::exit(0);
        }
    }

    /// The helper process: classify every key press and write one line per
    /// key (`K`) or modifier change (`M`) to stdout. Exits as soon as the
    /// parent closes stdin or dies.
    pub fn run() {
        std::thread::spawn(|| {
            let mut sink = String::new();
            while std::io::stdin().read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
                sink.clear();
            }
            std::process::exit(0);
        });
        let start = std::time::Instant::now();
        let wheel_only = std::env::args().nth(2).as_deref() == Some(WHEEL_ONLY_ARG);
        let mut combo = crate::keycombo::ComboBuilder::new();
        let res = listen(move |ev| {
            let now_ms = start.elapsed().as_secs_f64() * 1000.0;
            match ev.event_type {
                EventType::Wheel { delta_x, delta_y } => emit(&format!("S\t{delta_x}\t{delta_y}")),
                // In wheel only mode keys are never looked at, let alone sent.
                EventType::KeyPress(_) | EventType::KeyRelease(_) if wheel_only => {}
                EventType::KeyPress(k) => {
                    if let Some(m) = modifier(k) {
                        // Holding a modifier repeats the press; only a change counts.
                        if let Some(crate::events::EventKind::Mods { held }) = combo.modifier_down(m) {
                            emit(&format!("M\t{}", held.join(",")));
                        }
                        return;
                    }
                    // With a real modifier held, name the physical key: the
                    // text a hook reports changes with Shift or a layout.
                    let name = if crate::keycombo::makes_shortcut(combo.held()) { None } else { ev.name.clone() };
                    let Some(label) = key_label(k, name) else { return };
                    let id = format!("{k:?}");
                    if let Some(crate::events::EventKind::Key { key, modifiers }) = combo.key_down(&id, &label, false, None, now_ms) {
                        emit(&format!("K\t{key}\t{}", modifiers.join(",")));
                    }
                }
                EventType::KeyRelease(k) => {
                    if let Some(m) = modifier(k) {
                        if let Some(crate::events::EventKind::Mods { held }) = combo.modifier_up(m) {
                            emit(&format!("M\t{}", held.join(",")));
                        }
                    } else {
                        combo.key_up(&format!("{k:?}"));
                    }
                }
                _ => {}
            }
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
            let mut scrolls: Option<u32> = None;
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
                // macOS counts the scroll events the system posts (no permission needed).
                if let Some(n) = src.scroll_count() {
                    if let Some(prev) = scrolls {
                        if n != prev {
                            evs.push(EventKind::Scroll { x: pos.0, y: pos.1, dx: 0.0, dy: n.wrapping_sub(prev) as f64 });
                        }
                    }
                    scrolls = Some(n);
                }
                if pos != last {
                    evs.push(EventKind::Move { x: pos.0, y: pos.1 });
                    last = pos;
                }
                for (i, b) in [MouseButton::Left, MouseButton::Right, MouseButton::Middle].into_iter().enumerate() {
                    if buttons[i] && !down[i] {
                        evs.push(EventKind::Click { x: pos.0, y: pos.1, button: b });
                    } else if !buttons[i] && down[i] {
                        evs.push(EventKind::Release { x: pos.0, y: pos.1, button: b });
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
            /// Scrolling comes from the wheel helper process on this OS.
            pub fn scroll_count(&mut self) -> Option<u32> {
                None
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
            /// Scrolling comes from the wheel helper process on this OS.
            pub fn scroll_count(&mut self) -> Option<u32> {
                None
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
            pub fn scroll_count(&mut self) -> Option<u32> {
                Some(super::super::mac::scroll_counter())
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
            pub fn scroll_count(&mut self) -> Option<u32> {
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
        fn CGEventSourceCounterForEventType(state: i32, event_type: u32) -> u32;
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

    /// How many scroll events the system has posted since login. It needs no
    /// permission (unlike an event tap), and a change means "scrolling now".
    pub fn scroll_counter() -> u32 {
        // 1 = hardware (HID) state, 22 = kCGEventScrollWheel.
        unsafe { CGEventSourceCounterForEventType(1, 22) }
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
            115 => "Home", 119 => "End", 116 => "PageUp", 121 => "PageDown", 114 => "Insert",
            72 => "VolumeUp", 73 => "VolumeDown", 74 => "Mute",
            105 => "F13", 107 => "F14", 113 => "F15", 106 => "F16", 64 => "F17", 79 => "F18", 80 => "F19", 90 => "F20",
            65 => ".", 67 => "*", 69 => "+", 75 => "/", 78 => "-", 81 => "=",
            82 => "0", 83 => "1", 84 => "2", 85 => "3", 86 => "4", 87 => "5", 88 => "6", 89 => "7", 91 => "8", 92 => "9",
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

    /// The Fn / Globe key. It is a modifier, so it has no name above.
    pub const FN_CODE: u16 = 63;
    /// Caps Lock is a toggle: its flag changes each time it is pressed.
    pub const CAPS_FLAG: u64 = 0x10000;

    /// Codes worth polling: every code that has a name.
    pub fn codes() -> Vec<u16> {
        (0u16..128).filter(|c| name(*c).is_some()).collect()
    }
}

/// A listen-only CGEventTap for the keyboard (macOS). It sees every key down,
/// key up and modifier change with the modifier flags *of that very event*,
/// so the second key of Cmd+W is always paired with the Cmd that was held,
/// even when Cmd comes up first. It needs Input Monitoring; without it the
/// tap cannot be created and `mac_keys` falls back to polling.
#[cfg(target_os = "macos")]
mod mac_tap {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    type TapCallback = extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(tap: u32, place: u32, options: u32, events_of_interest: u64, callback: TapCallback, user: *mut c_void) -> *mut c_void;
        fn CGEventTapEnable(tap: *mut c_void, enable: bool);
        fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
        fn CGEventGetFlags(event: *mut c_void) -> u64;
        fn CGEventKeyboardGetUnicodeString(event: *mut c_void, max: usize, actual: *mut usize, buf: *mut u16);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopCommonModes: *const c_void;
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: *mut c_void, order: isize) -> *mut c_void;
        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopRemoveSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
        fn CFRelease(cf: *const c_void);
    }

    const KEY_DOWN: u32 = 10;
    const KEY_UP: u32 = 11;
    const FLAGS_CHANGED: u32 = 12;
    const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
    const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;
    const FIELD_KEYCODE: u32 = 9;
    const FIELD_AUTOREPEAT: u32 = 8;
    const CAPS_LOCK_CODE: i64 = 57;
    const SHIFT: u64 = 0x20000;
    const OPTION: u64 = 0x80000;

    struct State {
        shared: Arc<Mutex<Shared>>,
        combo: crate::keycombo::ComboBuilder,
        tap: *mut c_void,
        start: Instant,
        fn_down: bool,
    }

    /// The name of a key for the label: a named key by its name, a letter or
    /// symbol by what it types without Option (so Cmd+W is W on any layout
    /// that can type it), and the US position as the last resort.
    fn key_name(code: u16, event: *mut c_void, flags: u64) -> Option<String> {
        let table = mackeys::name(code);
        if let Some(n) = table {
            if n.chars().count() > 1 {
                return Some(n.to_string());
            }
        }
        // Shift changes the character (4 becomes $); the label wants the key.
        if flags & (SHIFT | OPTION) == 0 {
            let mut buf = [0u16; 4];
            let mut n = 0usize;
            unsafe { CGEventKeyboardGetUnicodeString(event, 4, &mut n, buf.as_mut_ptr()) };
            if n == 1 {
                if let Some(c) = char::from_u32(buf[0] as u32).filter(|c| c.is_ascii_graphic()) {
                    return Some(c.to_string());
                }
            }
        }
        table.map(|n| n.to_string())
    }

    extern "C" fn callback(_proxy: *mut c_void, ty: u32, event: *mut c_void, user: *mut c_void) -> *mut c_void {
        if user.is_null() {
            return event;
        }
        let st = unsafe { &mut *(user as *mut State) };
        if ty == TAP_DISABLED_TIMEOUT || ty == TAP_DISABLED_USER {
            // The system switches a slow tap off. Switch it back on.
            if !st.tap.is_null() {
                unsafe { CGEventTapEnable(st.tap, true) };
            }
            return event;
        }
        let now = Instant::now();
        let now_ms = st.start.elapsed().as_secs_f64() * 1000.0;
        let code = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) };
        let flags = unsafe { CGEventGetFlags(event) };
        let mut push = |kind: crate::events::EventKind| {
            if let Ok(mut g) = st.shared.lock() {
                g.push(now, kind);
            }
        };
        let mut mods = mackeys::modifiers(flags);
        match ty {
            FLAGS_CHANGED => {
                if code == mackeys::FN_CODE as i64 {
                    st.fn_down = !st.fn_down;
                }
                if st.fn_down {
                    mods.insert(0, "Fn".to_string());
                }
                if code == CAPS_LOCK_CODE {
                    push(crate::events::EventKind::Key { key: "CapsLock".into(), modifiers: Vec::new() });
                }
                if let Some(ev) = st.combo.set_mods(&mods) {
                    push(ev);
                }
            }
            KEY_DOWN => {
                if st.fn_down {
                    mods.insert(0, "Fn".to_string());
                }
                // Flags of this event win over what was tracked.
                let repeat = unsafe { CGEventGetIntegerValueField(event, FIELD_AUTOREPEAT) } != 0;
                if let Some(name) = key_name(code as u16, event, flags) {
                    if let Some(ev) = st.combo.key_down(&code.to_string(), &name, repeat, Some(&mods), now_ms) {
                        push(ev);
                    }
                }
            }
            KEY_UP => st.combo.key_up(&code.to_string()),
            _ => {}
        }
        event
    }

    /// Runs the tap on this thread until `stop` is set or recording ends.
    /// Returns false when the tap could not be created (no permission).
    pub fn run(shared: &Arc<Mutex<Shared>>, stop: &AtomicBool) -> bool {
        let state = Box::into_raw(Box::new(State {
            shared: shared.clone(),
            combo: crate::keycombo::ComboBuilder::new(),
            tap: std::ptr::null_mut(),
            start: Instant::now(),
            fn_down: false,
        }));
        // kCGHIDEventTap, head insert, listen only; key down, up and flags.
        let mask: u64 = (1 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED);
        let tap = unsafe { CGEventTapCreate(0, 0, 1, mask, callback, state as *mut c_void) };
        if tap.is_null() {
            drop(unsafe { Box::from_raw(state) });
            return false;
        }
        unsafe { (*state).tap = tap };
        let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };
        if source.is_null() {
            unsafe { CFRelease(tap) };
            drop(unsafe { Box::from_raw(state) });
            return false;
        }
        unsafe {
            let rl = CFRunLoopGetCurrent();
            CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            while !stop.load(Ordering::SeqCst) {
                if !shared.lock().map(|g| g.active).unwrap_or(false) {
                    break;
                }
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false);
            }
            CGEventTapEnable(tap, false);
            CFRunLoopRemoveSource(rl, source, kCFRunLoopCommonModes);
            CFRelease(source);
            CFRelease(tap);
            drop(Box::from_raw(state));
        }
        true
    }
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
        let codes = mackeys::codes();
        let h = std::thread::spawn(move || {
            // The event tap sees the flags of each key press. Polling is the
            // fallback when the tap cannot be created.
            if mac_tap::run(&shared, &flag) {
                return;
            }
            eprintln!("demolens: could not start the key event tap, polling the key state instead");
            let mut down = std::collections::HashSet::<u16>::new();
            let first = mac::flags();
            let mut prev_held: Vec<String> = Vec::new();
            let mut prev_caps = first & mackeys::CAPS_FLAG != 0;
            while !flag.load(Ordering::SeqCst) {
                if !shared.lock().unwrap().active {
                    break;
                }
                let now = Instant::now();
                let flags = mac::flags();
                let mut mods = mackeys::modifiers(flags);
                // Fn is read as a key: the Fn flag is also set by arrow keys.
                if mac::key_down(mackeys::FN_CODE) {
                    mods.insert(0, "Fn".to_string());
                }
                if mods != prev_held {
                    shared.lock().unwrap().push(now, crate::events::EventKind::Mods { held: mods.clone() });
                    prev_held = mods.clone();
                }
                let caps = flags & mackeys::CAPS_FLAG != 0;
                if caps != prev_caps {
                    prev_caps = caps;
                    shared.lock().unwrap().push(now, crate::events::EventKind::Key { key: "CapsLock".into(), modifiers: Vec::new() });
                }
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
        let codes = mackeys::codes();
        assert!(codes.len() > 100, "every named key is polled, got {}", codes.len());
        for c in [53u16, 122, 111, 105, 115, 119, 116, 121, 117, 51, 48, 49, 36, 123, 124, 125, 126, 72, 73, 74] {
            assert!(codes.contains(&c), "code {c} is not polled");
        }
        assert_eq!(mackeys::name(111), Some("F12"));
        assert_eq!(mackeys::name(105), Some("F13"));
        assert_eq!(mackeys::name(115), Some("Home"));
        assert_eq!(mackeys::name(121), Some("PageDown"));
        assert_eq!(mackeys::name(74), Some("Mute"));
        assert_eq!(mackeys::name(mackeys::FN_CODE), None, "Fn is read as a modifier");
    }

    #[test]
    fn keys_are_classified_never_stored_as_characters() {
        assert_eq!(classify("p", &[]), PRINTABLE);
        assert_eq!(classify("P", &["Shift".into()]), PRINTABLE);
        assert_eq!(classify("7", &[]), PRINTABLE);
        assert_eq!(classify("s", &["Ctrl".into()]), "S");
        assert_eq!(classify("KeyS", &["Ctrl".into()]), "S");
        assert_eq!(classify("Enter", &[]), "Enter");
        assert_eq!(classify("Escape", &[]), "Esc");
        assert_eq!(classify("UpArrow", &[]), "Up");
        assert_eq!(classify("F5", &[]), "F5");
        assert_eq!(classify("VolumeUp", &[]), "VolumeUp");
        assert_eq!(classify("e", &["Fn".into()]), PRINTABLE, "Fn alone is not a shortcut");
        assert_eq!(classify("e", &["Alt".into()]), "E");
        assert_eq!(classify("e", &["Win".into()]), "E");
        assert_eq!(classify("e", &["Super".into()]), "E");
        assert_eq!(classify(" ", &[]), "Space");
        assert_eq!(KeyLogging::default(), KeyLogging::Off);
        assert_eq!(KeyLogging::parse("on"), Some(KeyLogging::On));
    }
}
