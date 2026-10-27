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
        #[cfg(feature = "input-hooks")]
        hooks::spawn(shared.clone(), region);
        #[cfg(not(feature = "input-hooks"))]
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

#[cfg(feature = "input-hooks")]
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
