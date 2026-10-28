//! Names and symbols for the keystroke overlay.
//!
//! The event log holds only key *classes* (never typed characters): named
//! keys ("Esc", "F5", "PlayPause"), shortcut keys ("S" with modifiers) and
//! snapshots of the modifiers held (`Mods`). This module turns them into the
//! labels shown on the video: `⌘ ⇧ 4` or `Cmd + Shift + 4` on a Mac,
//! `Ctrl + Shift + S` on Windows and Linux, `⌥ Option` for a modifier held on
//! its own, `⌘ Click` and `⌥ Drag` for mouse and modifier combos.

use crate::events::{EventKind, EventLog, MouseButton, PRINTABLE};
use crate::render::KeyLabel;

/// Which keyboard a recording was made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "macos" | "mac" | "darwin" => Some(Self::Mac),
            "windows" | "win32" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }
    pub fn current() -> Self {
        Self::from_name(std::env::consts::OS).unwrap_or(Self::Linux)
    }
}

/// The modifier names the log may hold, in the order they are shown.
pub const MODIFIER_ORDER: [&str; 5] = ["Fn", "Ctrl", "Alt", "Shift", "Meta"];

/// How the overlay is built.
#[derive(Debug, Clone, Copy)]
pub struct LabelOptions {
    pub hold_ms: f64,
    /// Hide typing; show shortcuts, modifiers and special keys only.
    pub shortcuts_only: bool,
    /// Use the Mac symbols (⌘ ⌥ ⇧ ⌃ ⌫ ...) instead of words. Only applies to
    /// recordings made on a Mac; other keyboards have no such keycaps.
    pub symbols: bool,
}

fn rank(m: &str) -> usize {
    MODIFIER_ORDER.iter().position(|x| x.eq_ignore_ascii_case(m)).unwrap_or(MODIFIER_ORDER.len())
}

/// Sorted, de-duplicated, canonical modifier list.
pub fn ordered_mods(mods: &[String]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for m in mods {
        let canon = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "controlleft" | "controlright" => "Ctrl",
            "alt" | "option" | "opt" | "altgr" => "Alt",
            "shift" | "shiftleft" | "shiftright" => "Shift",
            "meta" | "cmd" | "command" | "win" | "super" | "metaleft" | "metaright" => "Meta",
            "fn" | "function" | "globe" => "Fn",
            _ => continue,
        };
        if !v.contains(&canon) {
            v.push(canon);
        }
    }
    v.sort_by_key(|m| rank(m));
    v
}

/// One modifier as shown.
pub fn mod_text(m: &str, p: Platform, symbols: bool) -> String {
    let symbols = symbols && p == Platform::Mac;
    match (m, p, symbols) {
        ("Ctrl", Platform::Mac, true) => "\u{2303}".into(),
        ("Alt", Platform::Mac, true) => "\u{2325}".into(),
        ("Shift", Platform::Mac, true) => "\u{21E7}".into(),
        ("Meta", Platform::Mac, true) => "\u{2318}".into(),
        ("Fn", _, _) => "fn".into(),
        ("Meta", Platform::Mac, false) => "Cmd".into(),
        ("Alt", Platform::Mac, false) => "Option".into(),
        ("Meta", Platform::Windows, _) => "Win".into(),
        ("Meta", Platform::Linux, _) => "Super".into(),
        _ => m.to_string(),
    }
}

/// Normalise the names different sources use for the same key (rdev's
/// `Escape`, `UpArrow`, `Return`, macOS key table names, and so on).
pub fn canonical_key(k: &str) -> String {
    let s = k.trim();
    let c = match s.to_ascii_lowercase().as_str() {
        "esc" | "escape" => "Esc",
        "return" | "enter" | "kpreturn" => "Enter",
        "backspace" => "Backspace",
        "delete" | "forwarddelete" | "kpdelete" => "Delete",
        "tab" => "Tab",
        "space" | " " => "Space",
        "up" | "uparrow" => "Up",
        "down" | "downarrow" => "Down",
        "left" | "leftarrow" => "Left",
        "right" | "rightarrow" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" | "prior" => "PageUp",
        "pagedown" | "pgdn" | "next" => "PageDown",
        "insert" => "Insert",
        "capslock" | "caps" => "CapsLock",
        "numlock" => "NumLock",
        "scrolllock" => "ScrollLock",
        "printscreen" | "prtsc" => "PrintScreen",
        "pause" => "Pause",
        "fn" | "function" | "globe" => "Fn",
        "playpause" | "mediaplaypause" => "PlayPause",
        "nexttrack" | "medianext" | "medianexttrack" => "NextTrack",
        "prevtrack" | "mediaprevious" | "mediaprevioustrack" => "PrevTrack",
        "mediastop" | "stop" => "MediaStop",
        "volumeup" => "VolumeUp",
        "volumedown" => "VolumeDown",
        "mute" | "volumemute" => "Mute",
        "brightnessup" => "BrightnessUp",
        "brightnessdown" => "BrightnessDown",
        _ => return s.to_string(),
    };
    c.to_string()
}

/// One key as shown.
pub fn key_text(key: &str, p: Platform, symbols: bool) -> String {
    let key = canonical_key(key);
    let sym = symbols && p == Platform::Mac;
    let mac = p == Platform::Mac;
    let t: &str = match (key.as_str(), sym, mac) {
        ("Enter", true, _) => "\u{21A9}",
        ("Enter", false, true) => "Return",
        ("Tab", true, _) => "\u{21E5}",
        ("Backspace", true, _) => "\u{232B}",
        ("Backspace", false, true) => "Delete",
        ("Delete", true, _) => "\u{2326}",
        ("Delete", false, true) => "Forward Delete",
        ("Esc", true, _) => "esc",
        ("Up", true, _) => "\u{2191}",
        ("Down", true, _) => "\u{2193}",
        ("Left", true, _) => "\u{2190}",
        ("Right", true, _) => "\u{2192}",
        ("Home", true, _) => "\u{2196}",
        ("End", true, _) => "\u{2198}",
        ("PageUp", true, _) => "\u{21DE}",
        ("PageDown", true, _) => "\u{21DF}",
        ("CapsLock", true, _) => "\u{21EA}",
        ("CapsLock", false, _) => "Caps Lock",
        ("PageUp", false, _) => "Page Up",
        ("PageDown", false, _) => "Page Down",
        ("NumLock", _, _) => "Num Lock",
        ("ScrollLock", _, _) => "Scroll Lock",
        ("PrintScreen", _, _) => "Print Screen",
        ("PlayPause", _, _) => "Play/Pause",
        ("NextTrack", _, _) => "Next track",
        ("PrevTrack", _, _) => "Previous track",
        ("MediaStop", _, _) => "Stop",
        ("VolumeUp", _, _) => "Volume up",
        ("VolumeDown", _, _) => "Volume down",
        ("Mute", _, _) => "Mute",
        ("BrightnessUp", _, _) => "Brightness up",
        ("BrightnessDown", _, _) => "Brightness down",
        ("Fn", _, _) => "fn",
        _ => return if key.chars().count() == 1 { key.to_uppercase() } else { key },
    };
    t.to_string()
}

/// Join parts: symbols run together with thin gaps, words use " + ".
fn join(parts: Vec<String>, symbols: bool, p: Platform) -> String {
    if symbols && p == Platform::Mac {
        parts.join(" ")
    } else {
        parts.join(" + ")
    }
}

/// `⌘ ⇧ 4` or `Cmd + Shift + 4`.
pub fn combo_text(mods: &[String], key: &str, p: Platform, symbols: bool) -> String {
    let mut parts: Vec<String> = ordered_mods(mods).into_iter().map(|m| mod_text(m, p, symbols)).collect();
    parts.push(key_text(key, p, symbols));
    join(parts, symbols, p)
}

/// A modifier held on its own: `⌥ Option`, or `⌘ ⇧` for several.
pub fn mods_only_text(mods: &[&str], p: Platform, symbols: bool) -> String {
    let sym = symbols && p == Platform::Mac;
    if sym && mods.len() == 1 {
        let word = mod_text(mods[0], p, false);
        let word = match mods[0] {
            "Ctrl" => "Control".to_string(),
            "Meta" => "Command".to_string(),
            "Fn" => return "fn".into(),
            _ => word,
        };
        return format!("{} {}", mod_text(mods[0], p, true), word);
    }
    let parts: Vec<String> = mods.iter().map(|m| mod_text(m, p, symbols)).collect();
    join(parts, symbols, p)
}

fn is_special(key: &str) -> bool {
    key != PRINTABLE && key.chars().count() > 1 && !key.eq_ignore_ascii_case("space")
}

/// Typing-ish keys that are folded into a "Typing..." label while typing.
fn is_edit_key(key: &str) -> bool {
    matches!(key, "Space" | "Enter" | "Backspace" | "Delete")
}

fn button_word(b: MouseButton, symbols: bool) -> &'static str {
    let _ = symbols;
    match b {
        MouseButton::Left => "Click",
        MouseButton::Right => "Right click",
        MouseButton::Middle => "Middle click",
        MouseButton::Touch => "Tap",
    }
}

const TYPING_GAP_MS: f64 = 1200.0;

pub fn build_key_labels(log: &EventLog, o: &LabelOptions) -> Vec<KeyLabel> {
    let p = Platform::from_name(&log.header.platform).unwrap_or_else(Platform::current);
    let sym = o.symbols && p == Platform::Mac;
    let hold = o.hold_ms;
    let mut out: Vec<KeyLabel> = Vec::new();

    // Pass 1: keys.
    let mut typing_open = false;
    let mut last_typed = f64::NEG_INFINITY;
    for e in &log.events {
        let EventKind::Key { key, modifiers } = &e.kind else { continue };
        let t = e.t;
        let key = canonical_key(key);
        let mods = ordered_mods(modifiers);
        let non_shift = mods.iter().any(|m| *m != "Shift");
        let shifted_named = mods.contains(&"Shift") && is_special(&key);
        if non_shift || shifted_named {
            let ms: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
            push_label(&mut out, t, hold, combo_text(&ms, &key, p, sym));
            typing_open = false;
            continue;
        }
        let typed = key == PRINTABLE || key.chars().count() == 1;
        let recent = t - last_typed < TYPING_GAP_MS;
        if typed || (is_edit_key(&key) && recent) {
            if typed {
                last_typed = t;
            }
            if o.shortcuts_only {
                continue;
            }
            // The log holds no characters, so typing shows as one label.
            if let Some(last) = out.last_mut() {
                if typing_open && t - (last.end_ms - hold) < TYPING_GAP_MS {
                    last.end_ms = t + hold;
                    continue;
                }
            }
            out.push(KeyLabel { start_ms: t, end_ms: t + hold, text: crate::render::TYPING.into() });
            typing_open = true;
        } else {
            push_label(&mut out, t, hold, key_text(&key, p, sym));
            typing_open = false;
        }
    }

    // Snapshots of held modifiers, for modifier-only presses and mouse combos.
    let snaps: Vec<(f64, Vec<&'static str>)> = log
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Mods { held } => Some((e.t, ordered_mods(held))),
            _ => None,
        })
        .collect();
    let held_at = |t: f64| -> Vec<&'static str> {
        let i = snaps.partition_point(|(st, _)| *st <= t);
        if i == 0 { Vec::new() } else { snaps[i - 1].1.clone() }
    };
    let busy = |a: f64, b: f64| -> bool {
        log.events.iter().any(|e| {
            e.t >= a
                && e.t <= b
                && match &e.kind {
                    EventKind::Key { .. } => true,
                    EventKind::Click { .. } => true,
                    _ => false,
                }
        })
    };

    // Pass 2: modifiers held without a key or click.
    let mut span: Option<(f64, Vec<&'static str>)> = None;
    let mut spans: Vec<(f64, f64, Vec<&'static str>)> = Vec::new();
    for (t, held) in &snaps {
        match (&mut span, held.is_empty()) {
            (None, false) => span = Some((*t, held.clone())),
            (Some((_, u)), false) => {
                for m in held {
                    if !u.contains(m) {
                        u.push(m);
                    }
                }
            }
            (Some(_), true) => {
                let (t0, mut u) = span.take().unwrap();
                u.sort_by_key(|m| rank(m));
                spans.push((t0, *t, u));
            }
            (None, true) => {}
        }
    }
    let open_span = span.take();
    for (t0, t1, u) in spans {
        if busy(t0 - 30.0, t1 + 30.0) {
            continue;
        }
        let end = (t1 + 250.0).max(t0 + (hold * 0.7).max(700.0));
        out.push(KeyLabel { start_ms: t0, end_ms: end, text: mods_only_text(&u, p, sym) });
    }
    if let Some((t0, mut u)) = open_span {
        u.sort_by_key(|m| rank(m));
        if !busy(t0 - 30.0, f64::INFINITY) {
            out.push(KeyLabel { start_ms: t0, end_ms: t0 + hold, text: mods_only_text(&u, p, sym) });
        }
    }

    // Pass 3: mouse buttons with modifiers held (Cmd-click, Option-drag).
    let clicks: Vec<(f64, f64, f64, MouseButton)> = log
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Click { x, y, button } => Some((e.t, x, y, button)),
            _ => None,
        })
        .collect();
    for (i, &(t, x, y, b)) in clicks.iter().enumerate() {
        let held = held_at(t);
        if held.is_empty() {
            continue;
        }
        let next_click = clicks.iter().skip(i + 1).find(|c| c.3 == b).map(|c| c.0).unwrap_or(f64::INFINITY);
        let release = log.events.iter().find_map(|e| match e.kind {
            EventKind::Release { x: rx, y: ry, button } if button == b && e.t >= t && e.t < next_click => Some((e.t, rx, ry)),
            _ => None,
        });
        let ms: Vec<String> = held.iter().map(|m| m.to_string()).collect();
        let join_word = |word: &str| -> String {
            let mut parts: Vec<String> = held.iter().map(|m| mod_text(m, p, sym)).collect();
            parts.push(word.to_string());
            let _ = &ms;
            join(parts, sym, p)
        };
        match release {
            Some((rt, rx, ry)) if ((rx - x).powi(2) + (ry - y).powi(2)).sqrt() > 12.0 || rt - t > 350.0 => {
                let word = if b == MouseButton::Left || b == MouseButton::Touch { "Drag".to_string() } else { format!("{} drag", button_word(b, sym).trim_end_matches(" click")) };
                out.push(KeyLabel { start_ms: t, end_ms: rt + 400.0, text: join_word(&word) });
            }
            _ => out.push(KeyLabel { start_ms: t, end_ms: t + hold, text: join_word(button_word(b, sym)) }),
        }
    }

    out.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    // Earlier labels end when the next one starts.
    for i in 1..out.len() {
        let next = out[i].start_ms;
        if out[i - 1].end_ms > next {
            out[i - 1].end_ms = next;
        }
    }
    out.retain(|l| l.end_ms > l.start_ms + 1.0);
    out
}

fn push_label(out: &mut Vec<KeyLabel>, t: f64, hold: f64, text: String) {
    if let Some(last) = out.last_mut() {
        // The same key pressed again before its label is gone extends it.
        if last.text == text && t < last.end_ms {
            last.end_ms = t + hold;
            return;
        }
    }
    out.push(KeyLabel { start_ms: t, end_ms: t + hold, text });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Event;

    fn log(platform: &str, evs: Vec<(f64, EventKind)>) -> EventLog {
        let mut l = EventLog::new(1000, 1000);
        l.header.platform = platform.into();
        l.events = evs.into_iter().map(|(t, kind)| Event { t, kind }).collect();
        l
    }
    fn key(k: &str, m: &[&str]) -> EventKind {
        EventKind::Key { key: k.into(), modifiers: m.iter().map(|s| s.to_string()).collect() }
    }
    fn mods(m: &[&str]) -> EventKind {
        EventKind::Mods { held: m.iter().map(|s| s.to_string()).collect() }
    }
    fn opts(symbols: bool, shortcuts_only: bool) -> LabelOptions {
        LabelOptions { hold_ms: 1000.0, shortcuts_only, symbols }
    }
    fn texts(l: &EventLog, o: &LabelOptions) -> Vec<String> {
        build_key_labels(l, o).into_iter().map(|x| x.text).collect()
    }

    #[test]
    fn mac_symbols_and_words() {
        let l = log("macos", vec![(0.0, key("4", &["Meta", "Shift"]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["\u{21E7} \u{2318} 4"]);
        assert_eq!(texts(&l, &opts(false, true)), vec!["Shift + Cmd + 4"]);
        let l = log("macos", vec![(0.0, key("S", &["Ctrl", "Alt", "Shift", "Meta"]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["\u{2303} \u{2325} \u{21E7} \u{2318} S"]);
        assert_eq!(texts(&l, &opts(false, true)), vec!["Ctrl + Option + Shift + Cmd + S"]);
    }

    #[test]
    fn windows_and_linux_always_use_words() {
        let l = log("windows", vec![(0.0, key("S", &["Ctrl", "Shift"])), (3000.0, key("L", &["Meta"]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["Ctrl + Shift + S", "Win + L"]);
        let l = log("linux", vec![(0.0, key("T", &["Meta", "Alt"]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["Alt + Super + T"]);
    }

    #[test]
    fn special_keys_show_even_when_typing_is_hidden() {
        let evs = vec![
            (0.0, key("Esc", &[])),
            (1500.0, key("Tab", &[])),
            (3000.0, key("Up", &[])),
            (4500.0, key("F5", &[])),
            (6000.0, key("PlayPause", &[])),
            (7500.0, key("VolumeUp", &[])),
            (9000.0, key("CapsLock", &[])),
            (10500.0, key("Space", &[])),
            (12000.0, key("Backspace", &[])),
            (13500.0, key("Enter", &[])),
            (15000.0, key("Delete", &[])),
        ];
        let w = texts(&log("windows", evs.clone()), &opts(false, true));
        assert_eq!(w, vec!["Esc", "Tab", "Up", "F5", "Play/Pause", "Volume up", "Caps Lock", "Space", "Backspace", "Enter", "Delete"]);
        let m = texts(&log("macos", evs.clone()), &opts(true, true));
        assert_eq!(m, vec!["esc", "\u{21E5}", "\u{2191}", "F5", "Play/Pause", "Volume up", "\u{21EA}", "Space", "\u{232B}", "\u{21A9}", "\u{2326}"]);
        let mw = texts(&log("macos", evs), &opts(false, true));
        assert_eq!(mw[0], "Esc");
        assert_eq!(mw[8], "Delete");
        assert_eq!(mw[9], "Return");
        assert_eq!(mw[10], "Forward Delete");
    }

    #[test]
    fn typing_is_one_label_and_edit_keys_join_it() {
        let l = log("linux", vec![(0.0, key(PRINTABLE, &[])), (100.0, key(PRINTABLE, &[])), (200.0, key("Space", &[])), (300.0, key(PRINTABLE, &[])), (400.0, key("Backspace", &[])), (500.0, key("Enter", &[]))]);
        assert_eq!(texts(&l, &opts(false, false)), vec!["Typing..."]);
        assert!(texts(&l, &opts(false, true)).is_empty());
    }

    #[test]
    fn shift_with_a_named_key_is_a_combo_but_capitals_are_typing() {
        let l = log("linux", vec![(0.0, key("Tab", &["Shift"])), (3000.0, key(PRINTABLE, &["Shift"]))]);
        assert_eq!(texts(&l, &opts(false, false)), vec!["Shift + Tab", "Typing..."]);
    }

    #[test]
    fn modifier_held_alone_is_shown_and_not_when_part_of_a_combo() {
        let l = log("macos", vec![(1000.0, mods(&["Alt"])), (2500.0, mods(&[]))]);
        let b = build_key_labels(&l, &opts(true, true));
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "\u{2325} Option");
        assert_eq!(b[0].start_ms, 1000.0);
        assert!(b[0].end_ms >= 2750.0);
        assert_eq!(texts(&l, &opts(false, true)), vec!["Option"]);
        // Cmd held while S is pressed: only the combo label.
        let l = log("macos", vec![(1000.0, mods(&["Meta"])), (1200.0, key("S", &["Meta"])), (1500.0, mods(&[]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["\u{2318} S"]);
        let l = log("macos", vec![(1000.0, mods(&["Meta"])), (1100.0, mods(&["Meta", "Shift"])), (2000.0, mods(&[]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["\u{21E7} \u{2318}"]);
        let l = log("macos", vec![(1000.0, mods(&["Fn"])), (1500.0, mods(&[]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["fn"]);
    }

    #[test]
    fn modifier_clicks_and_drags() {
        let click = |x: f64| EventKind::Click { x, y: 50.0, button: MouseButton::Left };
        let rel = |x: f64| EventKind::Release { x, y: 50.0, button: MouseButton::Left };
        let l = log("macos", vec![(500.0, mods(&["Meta"])), (800.0, click(100.0)), (880.0, rel(100.0)), (1000.0, mods(&[]))]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["\u{2318} Click"]);
        assert_eq!(texts(&l, &opts(false, true)), vec!["Cmd + Click"]);
        let l = log("macos", vec![(500.0, mods(&["Alt"])), (800.0, click(100.0)), (1600.0, rel(400.0)), (1700.0, mods(&[]))]);
        let b = build_key_labels(&l, &opts(true, true));
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "\u{2325} Drag");
        assert_eq!(b[0].end_ms, 2000.0);
        // A plain click with no modifier shows nothing.
        let l = log("macos", vec![(800.0, click(100.0)), (880.0, rel(100.0))]);
        assert!(texts(&l, &opts(true, true)).is_empty());
        // Right click with Ctrl on Windows.
        let l = log("windows", vec![(500.0, mods(&["Ctrl"])), (800.0, EventKind::Click { x: 1.0, y: 1.0, button: MouseButton::Right })]);
        assert_eq!(texts(&l, &opts(true, true)), vec!["Ctrl + Right click"]);
    }

    #[test]
    fn key_names_from_every_source_agree() {
        for (a, b) in [("Escape", "Esc"), ("Return", "Enter"), ("UpArrow", "Up"), ("PageDown", "PageDown"), ("Function", "Fn"), ("MediaPlayPause", "PlayPause")] {
            assert_eq!(canonical_key(a), b, "{a}");
        }
    }
}
