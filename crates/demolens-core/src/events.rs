//! Input event log written next to every recording.
//!
//! The format is JSON Lines: one header line followed by one event per line.
//! Timestamps are milliseconds since the first video frame. Coordinates are in
//! recorded-video pixels (the capture origin is already subtracted).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Event log format version. Version 2 never stores typed characters.
pub const EVENTS_VERSION: u32 = 2;

/// The `key` of every printable key press without Ctrl/Alt/Meta: the log
/// keeps *that* something was typed and when, never *what*.
pub const PRINTABLE: &str = "printable";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// A finger or pen on a touch screen. A tap is a click for zoom and
    /// click effects, drawn with a touch ripple instead of a mouse ring.
    Touch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EventKind {
    /// Cursor moved to (x, y).
    Move { x: f64, y: f64 },
    /// Mouse button pressed at (x, y).
    Click { x: f64, y: f64, button: MouseButton },
    /// Key pressed. `key` is a key class: [`PRINTABLE`] for typing, a named
    /// key such as "Enter", "Tab", "Space", "Up", or the key of a shortcut
    /// ("S" with modifiers ["Ctrl"]). `modifiers` holds held modifiers such as
    /// "Ctrl", "Shift", "Alt", "Meta".
    Key {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
    /// The set of modifier keys held changed. `held` lists every modifier
    /// down right now ("Ctrl", "Alt", "Shift", "Meta", "Fn"); empty means all
    /// were released. This is how modifier-only presses (holding Option) and
    /// mouse+modifier combos (Cmd-click) are shown.
    Mods {
        #[serde(default)]
        held: Vec<String>,
    },
    /// A mouse button was released at (x, y). With the matching `Click` it
    /// tells a click from a drag.
    Release { x: f64, y: f64, button: MouseButton },
    /// The presenter dropped a marker (a global shortcut) with the cursor at
    /// (x, y). It is a deliberate "zoom here" point.
    Mark { x: f64, y: f64 },
    /// The wheel or trackpad scrolled, with the cursor at (x, y). `dx` and
    /// `dy` are the amount when the OS tells it (0 when it only says that a
    /// scroll happened). Scrolling is what keeps auto zoom wide: a zoomed
    /// view that scrolls under the viewer is hard to follow.
    Scroll {
        x: f64,
        y: f64,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Milliseconds since recording start.
    pub t: f64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLogHeader {
    pub demolens_events: u32,
    pub width: u32,
    pub height: u32,
    /// Offset that was subtracted from raw screen coordinates.
    #[serde(default)]
    pub origin_x: i32,
    #[serde(default)]
    pub origin_y: i32,
    /// Display scale factor (physical / logical pixels).
    #[serde(default = "one")]
    pub scale: f64,
    /// OS the recording was made on ("macos", "windows", "linux"), so key
    /// labels use that keyboard's names wherever the project is exported.
    /// Empty in older logs.
    #[serde(default)]
    pub platform: String,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLog {
    pub header: EventLogHeader,
    pub events: Vec<Event>,
}

/// Multiplies every pointer coordinate by `k` (used when a screen was
/// recorded smaller than it is, so events match the pixels of the video).
pub fn scale_coords(events: &mut [Event], k: f64) {
    for e in events.iter_mut() {
        match &mut e.kind {
            EventKind::Move { x, y } | EventKind::Click { x, y, .. } | EventKind::Release { x, y, .. } | EventKind::Mark { x, y } | EventKind::Scroll { x, y, .. } => {
                *x *= k;
                *y *= k;
            }
            EventKind::Key { .. } | EventKind::Mods { .. } => {}
        }
    }
}

impl EventLog {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            header: EventLogHeader { demolens_events: EVENTS_VERSION, width, height, origin_x: 0, origin_y: 0, scale: 1.0, platform: std::env::consts::OS.to_string() },
            events: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut lines = BufReader::new(f).lines();
        let header_line = lines.next().context("empty event log")??;
        let header: EventLogHeader = serde_json::from_str(&header_line).context("bad event log header")?;
        let mut events = Vec::new();
        for (i, line) in lines.enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let ev: Event = serde_json::from_str(&line).with_context(|| format!("bad event on line {}", i + 2))?;
            events.push(ev);
        }
        events.sort_by(|a, b| a.t.total_cmp(&b.t));
        // Version 1 logs (DemoLens 0.1.x) stored typed characters. Reduce
        // them to key classes as they load, so they never reach the overlay.
        for e in &mut events {
            if let EventKind::Key { key, modifiers } = &mut e.kind {
                *key = crate::input::classify(key, modifiers);
            }
        }
        Ok(Self { header, events })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        // The log can hold what was typed, so only the owner may read it.
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = std::io::BufWriter::new(opts.open(path)?);
        let mut header = self.header.clone();
        header.demolens_events = EVENTS_VERSION;
        writeln!(f, "{}", serde_json::to_string(&header)?)?;
        for e in &self.events {
            writeln!(f, "{}", serde_json::to_string(e)?)?;
        }
        Ok(())
    }

    pub fn clicks(&self) -> impl Iterator<Item = (f64, f64, f64)> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::Click { x, y, .. } => Some((e.t, x, y)),
            _ => None,
        })
    }

    /// Mouse clicks only (no taps): (time, x, y).
    pub fn mouse_clicks(&self) -> impl Iterator<Item = (f64, f64, f64)> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::Click { x, y, button } if button != MouseButton::Touch => Some((e.t, x, y)),
            _ => None,
        })
    }

    /// Finger and pen touches (the start of each): (time, x, y).
    pub fn touches(&self) -> impl Iterator<Item = (f64, f64, f64)> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::Click { x, y, button: MouseButton::Touch } => Some((e.t, x, y)),
            _ => None,
        })
    }

    /// Markers dropped while recording: (time, x, y).
    /// Times (ms) of every scroll event, in order.
    pub fn scrolls(&self) -> impl Iterator<Item = f64> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::Scroll { .. } => Some(e.t),
            _ => None,
        })
    }

    pub fn marks(&self) -> impl Iterator<Item = (f64, f64, f64)> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::Mark { x, y } => Some((e.t, x, y)),
            _ => None,
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = (f64, &str, &[String])> + '_ {
        self.events.iter().filter_map(|e| match &e.kind {
            EventKind::Key { key, modifiers } => Some((e.t, key.as_str(), modifiers.as_slice())),
            _ => None,
        })
    }

    /// Raw cursor samples (moves and clicks), time ordered.
    pub fn cursor_samples(&self) -> Vec<(f64, f64, f64)> {
        self.events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Move { x, y } | EventKind::Click { x, y, .. } | EventKind::Mark { x, y } => Some((e.t, x, y)),
                _ => None,
            })
            .collect()
    }
}

/// Cursor track with linear interpolation and optional smoothing.
#[derive(Debug, Clone)]
pub struct CursorTrack {
    samples: Vec<(f64, f64, f64)>,
}

impl CursorTrack {
    pub fn new(log: &EventLog) -> Self {
        Self { samples: log.cursor_samples() }
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Raw interpolated position at time `t` (ms).
    pub fn raw_at(&self, t: f64) -> Option<(f64, f64)> {
        let s = &self.samples;
        if s.is_empty() {
            return None;
        }
        if t <= s[0].0 {
            return Some((s[0].1, s[0].2));
        }
        let last = s[s.len() - 1];
        if t >= last.0 {
            return Some((last.1, last.2));
        }
        let i = s.partition_point(|p| p.0 <= t);
        let (a, b) = (s[i - 1], s[i]);
        let span = (b.0 - a.0).max(1e-6);
        let k = (t - a.0) / span;
        Some((a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k))
    }

    /// Smoothed position: a triangular-weighted average of the raw path over
    /// `window_ms` around `t`. Deterministic, so preview and export match.
    pub fn smooth_at(&self, t: f64, window_ms: f64) -> Option<(f64, f64)> {
        if window_ms <= 1.0 {
            return self.raw_at(t);
        }
        const N: i32 = 12;
        let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
        for i in -N..=N {
            let dt = window_ms * (i as f64) / (N as f64);
            let w = 1.0 - (i.abs() as f64) / (N as f64 + 1.0);
            let (x, y) = self.raw_at(t + dt)?;
            sx += x * w;
            sy += y * w;
            sw += w;
        }
        Some((sx / sw, sy / sw))
    }
}
