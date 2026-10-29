//! Global shortcut model: the actions, their defaults, and the rules for what
//! a binding may be. Pure logic, so it is tested here; the desktop app only
//! turns a validated combo into an OS registration.
//!
//! A combo is a string like `CmdOrCtrl+Shift+KeyR`: modifiers first, then a
//! physical key name (the same names a browser gives in `event.code`, so a
//! binding means the same key on every keyboard layout). `CmdOrCtrl` is Cmd on
//! macOS and Ctrl elsewhere. An empty string means "not bound".

use std::collections::BTreeMap;

/// What a shortcut can do.
#[derive(Debug, Clone, Copy)]
pub struct ActionInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// What the shortcut does, for the settings page and the tooltips.
    pub hint: &'static str,
    /// Default combo, empty when the action starts unbound.
    pub default: &'static str,
}

pub const ACTIONS: &[ActionInfo] = &[
    ActionInfo { id: "record_toggle", label: "Start or stop recording", hint: "Starts with your current source and options, stops and opens the editor", default: "CmdOrCtrl+Shift+KeyR" },
    ActionInfo { id: "pause", label: "Pause or resume", hint: "Pauses capture; the paused part is cut out", default: "CmdOrCtrl+Shift+KeyP" },
    ActionInfo { id: "record_area", label: "Record an area", hint: "Pick an area on screen, then record it", default: "CmdOrCtrl+Alt+Digit1" },
    ActionInfo { id: "record_window", label: "Record a window", hint: "Pick a window, then record it", default: "CmdOrCtrl+Alt+Digit2" },
    ActionInfo { id: "record_screen", label: "Record the full screen", hint: "Records the main display", default: "CmdOrCtrl+Alt+Digit3" },
    ActionInfo { id: "restart", label: "Restart the take", hint: "Throws the current take away and starts a new one", default: "CmdOrCtrl+Alt+KeyR" },
    ActionInfo { id: "discard", label: "Discard the take", hint: "Stops and throws the recording away", default: "" },
    ActionInfo { id: "marker", label: "Drop a marker", hint: "Marks this moment; the editor zooms to the cursor here", default: "CmdOrCtrl+Alt+KeyM" },
    ActionInfo { id: "toggle_mic", label: "Toggle the microphone", hint: "For the next recording", default: "" },
    ActionInfo { id: "toggle_webcam", label: "Toggle the webcam", hint: "For the next recording", default: "" },
    ActionInfo { id: "toggle_keys", label: "Toggle the keystroke display", hint: "Whether key presses are captured in the next recording", default: "" },
];

pub fn info(id: &str) -> Option<&'static ActionInfo> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// A parsed combo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    pub cmd_or_ctrl: bool,
    pub ctrl: bool,
    pub sup: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

/// Is this a key name the app can register?
pub fn valid_key(k: &str) -> bool {
    if let Some(c) = k.strip_prefix("Key") {
        return c.len() == 1 && c.as_bytes()[0].is_ascii_uppercase();
    }
    if let Some(d) = k.strip_prefix("Digit") {
        return d.len() == 1 && d.as_bytes()[0].is_ascii_digit();
    }
    if let Some(n) = k.strip_prefix('F') {
        if let Ok(n) = n.parse::<u32>() {
            return (1..=24).contains(&n);
        }
    }
    matches!(
        k,
        "Space" | "Enter" | "Backspace" | "Tab" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Backquote" | "Minus" | "Equal" | "BracketLeft"
            | "BracketRight" | "Backslash" | "Semicolon" | "Quote" | "Comma" | "Period" | "Slash" | "Home" | "End" | "PageUp" | "PageDown" | "Insert" | "Delete"
    )
}

fn is_function_key(k: &str) -> bool {
    k.strip_prefix('F').is_some_and(|n| n.parse::<u32>().is_ok())
}

impl Combo {
    pub fn parse(s: &str) -> Result<Combo, String> {
        let mut c = Combo { cmd_or_ctrl: false, ctrl: false, sup: false, alt: false, shift: false, key: String::new() };
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err("That is not a key combination.".into());
        }
        for (i, p) in parts.iter().enumerate() {
            if i + 1 == parts.len() {
                if !valid_key(p) {
                    return Err(format!("{p} is not a key that can be used in a shortcut."));
                }
                c.key = (*p).to_string();
                continue;
            }
            let flag = match p.to_ascii_lowercase().as_str() {
                "cmdorctrl" | "commandorcontrol" => &mut c.cmd_or_ctrl,
                "ctrl" | "control" => &mut c.ctrl,
                "super" | "meta" | "win" | "cmd" | "command" => &mut c.sup,
                "alt" | "option" => &mut c.alt,
                "shift" => &mut c.shift,
                other => return Err(format!("{other} is not a modifier key.")),
            };
            if *flag {
                return Err("A modifier appears twice.".into());
            }
            *flag = true;
        }
        Ok(c)
    }

    /// Canonical text, so equal combos compare equal as strings.
    pub fn normalize(&self) -> String {
        let mut v: Vec<&str> = Vec::new();
        if self.cmd_or_ctrl {
            v.push("CmdOrCtrl");
        }
        if self.ctrl {
            v.push("Ctrl");
        }
        if self.sup {
            v.push("Super");
        }
        if self.alt {
            v.push("Alt");
        }
        if self.shift {
            v.push("Shift");
        }
        v.push(&self.key);
        v.join("+")
    }

    fn has_command_modifier(&self) -> bool {
        self.cmd_or_ctrl || self.ctrl || self.sup || self.alt
    }

    /// Shortcuts the system or the editor already use. A global registration
    /// would take them away from every app, so they are refused.
    fn reserved(&self) -> Option<&'static str> {
        let only_cmd = self.cmd_or_ctrl && !self.ctrl && !self.sup && !self.alt;
        let k = self.key.as_str();
        if only_cmd && !self.shift && matches!(k, "KeyC" | "KeyV" | "KeyX" | "KeyA" | "KeyZ" | "KeyY" | "KeyQ" | "KeyW" | "KeyH" | "KeyM" | "Tab" | "Space") {
            return Some("The system and every app use that for copy, paste, undo, quit and window control.");
        }
        if only_cmd && !self.shift && matches!(k, "KeyS" | "KeyO" | "KeyE" | "KeyR" | "Comma" | "Slash") {
            return Some("The editor already uses that for save, open, export, record, settings or help.");
        }
        if only_cmd && self.shift && matches!(k, "KeyZ" | "KeyS") {
            return Some("The editor already uses that for redo and Save As.");
        }
        if self.alt && !self.cmd_or_ctrl && !self.ctrl && !self.sup && matches!(k, "F4" | "Tab") {
            return Some("The system uses that to close or switch windows.");
        }
        if self.cmd_or_ctrl && self.alt && k == "Delete" {
            return Some("The system uses that for the task manager.");
        }
        None
    }

    /// Reject combos that would hijack typing or the system.
    pub fn check(&self) -> Result<(), String> {
        if !self.has_command_modifier() && !is_function_key(&self.key) {
            return Err("Add Ctrl, Cmd or Alt, so the shortcut does not take over normal typing.".into());
        }
        if let Some(why) = self.reserved() {
            return Err(why.into());
        }
        Ok(())
    }
}

/// User overrides, by action id. An empty combo means "unbound".
pub type Bindings = BTreeMap<String, String>;

/// The combo in effect for an action: the override, else the default.
pub fn effective(overrides: &Bindings, id: &str) -> String {
    match overrides.get(id) {
        Some(c) => c.clone(),
        None => info(id).map(|a| a.default.to_string()).unwrap_or_default(),
    }
}

/// Every action with its combo in effect, in display order.
pub fn all_effective(overrides: &Bindings) -> Vec<(&'static str, String)> {
    ACTIONS.iter().map(|a| (a.id, effective(overrides, a.id))).collect()
}

/// Which other action already uses this combo.
pub fn conflict_with(overrides: &Bindings, id: &str, combo: &str) -> Option<&'static str> {
    if combo.is_empty() {
        return None;
    }
    ACTIONS.iter().find(|a| a.id != id && effective(overrides, a.id) == combo).map(|a| a.label)
}

/// Validate and normalise a new binding for `id`. `Ok("")` means unbind.
pub fn check_binding(overrides: &Bindings, id: &str, combo: &str) -> Result<String, String> {
    if info(id).is_none() {
        return Err(format!("Unknown shortcut action {id}."));
    }
    if combo.trim().is_empty() {
        return Ok(String::new());
    }
    let c = Combo::parse(combo)?;
    c.check()?;
    let norm = c.normalize();
    if let Some(other) = conflict_with(overrides, id, &norm) {
        return Err(format!("{} already uses {}.", other, label(&norm, cfg!(target_os = "macos"))));
    }
    Ok(norm)
}

/// Store a binding, dropping the override when it equals the default.
pub fn set_binding(overrides: &mut Bindings, id: &str, combo: &str) -> Result<(), String> {
    let norm = check_binding(overrides, id, combo)?;
    let default = info(id).map(|a| a.default).unwrap_or("");
    if norm == default {
        overrides.remove(id);
    } else {
        overrides.insert(id.to_string(), norm);
    }
    Ok(())
}

/// Clean a loaded settings map: unknown actions, bad combos and duplicates
/// (the later action in display order loses) are dropped, so a hand edited
/// file cannot break startup.
pub fn sanitize(raw: &Bindings) -> Bindings {
    // First keep only combos that parse and pass the rules.
    let mut cand = Bindings::new();
    for a in ACTIONS {
        let Some(v) = raw.get(a.id) else { continue };
        if v.trim().is_empty() {
            cand.insert(a.id.to_string(), String::new());
        } else if let Ok(n) = Combo::parse(v).and_then(|c| c.check().map(|_| c.normalize())) {
            cand.insert(a.id.to_string(), n);
        }
    }
    // Then drop duplicates: an action that keeps its default owns that default.
    let mut out = Bindings::new();
    for a in ACTIONS {
        let Some(norm) = cand.get(a.id) else { continue };
        if !norm.is_empty() {
            if out.values().any(|v| v == norm) {
                continue;
            }
            if ACTIONS.iter().any(|o| o.id != a.id && !cand.contains_key(o.id) && o.default == norm) {
                continue;
            }
        }
        out.insert(a.id.to_string(), norm.clone());
    }
    out.retain(|k, v| info(k).is_none_or(|a| a.default != v));
    out
}

fn key_label(k: &str) -> String {
    if let Some(c) = k.strip_prefix("Key") {
        return c.to_string();
    }
    if let Some(d) = k.strip_prefix("Digit") {
        return d.to_string();
    }
    match k {
        "ArrowUp" => "Up".into(),
        "ArrowDown" => "Down".into(),
        "ArrowLeft" => "Left".into(),
        "ArrowRight" => "Right".into(),
        "Backquote" => "`".into(),
        "Minus" => "-".into(),
        "Equal" => "=".into(),
        "BracketLeft" => "[".into(),
        "BracketRight" => "]".into(),
        "Backslash" => "\\".into(),
        "Semicolon" => ";".into(),
        "Quote" => "'".into(),
        "Comma" => ",".into(),
        "Period" => ".".into(),
        "Slash" => "/".into(),
        other => other.to_string(),
    }
}

/// Text for a menu or tooltip: `Ctrl+Shift+R` or, on a Mac, `Shift+Cmd+R`.
/// Mac order follows Apple's menus (Ctrl, Option, Shift, Cmd).
pub fn label(combo: &str, mac: bool) -> String {
    let Ok(c) = Combo::parse(combo) else { return String::new() };
    let mut v: Vec<String> = Vec::new();
    if mac {
        if c.ctrl {
            v.push("Ctrl".into());
        }
        if c.alt {
            v.push("Option".into());
        }
        if c.shift {
            v.push("Shift".into());
        }
        if c.cmd_or_ctrl || c.sup {
            v.push("Cmd".into());
        }
    } else {
        if c.cmd_or_ctrl || c.ctrl {
            v.push("Ctrl".into());
        }
        if c.sup {
            v.push("Win".into());
        }
        if c.alt {
            v.push("Alt".into());
        }
        if c.shift {
            v.push("Shift".into());
        }
    }
    v.push(key_label(&c.key));
    v.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_unique_and_not_reserved() {
        let mut seen = std::collections::HashSet::new();
        for a in ACTIONS {
            if a.default.is_empty() {
                continue;
            }
            let c = Combo::parse(a.default).unwrap_or_else(|e| panic!("{}: {e}", a.id));
            assert_eq!(c.normalize(), a.default, "{} default is not in canonical form", a.id);
            c.check().unwrap_or_else(|e| panic!("{}: {e}", a.id));
            assert!(seen.insert(a.default), "{} duplicates another default", a.id);
        }
        // The two shortcuts earlier versions shipped keep their keys.
        assert_eq!(info("record_toggle").unwrap().default, "CmdOrCtrl+Shift+KeyR");
        assert_eq!(info("pause").unwrap().default, "CmdOrCtrl+Shift+KeyP");
    }

    #[test]
    fn parse_normalises_order_and_aliases() {
        assert_eq!(Combo::parse("shift+control+KeyR").unwrap().normalize(), "Ctrl+Shift+KeyR");
        assert_eq!(Combo::parse("Option+Command+Digit1").unwrap().normalize(), "Super+Alt+Digit1");
        assert_eq!(Combo::parse("CmdOrCtrl+Shift+F9").unwrap().normalize(), "CmdOrCtrl+Shift+F9");
    }

    #[test]
    fn bad_input_is_refused_with_a_reason() {
        assert!(Combo::parse("").is_err());
        assert!(Combo::parse("CmdOrCtrl+").is_err());
        assert!(Combo::parse("CmdOrCtrl+CmdOrCtrl+KeyA").is_err());
        assert!(Combo::parse("Hyper+KeyA").is_err());
        assert!(Combo::parse("CmdOrCtrl+KeyAA").is_err());
        assert!(Combo::parse("CmdOrCtrl+Escape").is_err());
        assert!(Combo::parse("CmdOrCtrl+F25").is_err());
    }

    #[test]
    fn typing_keys_need_a_real_modifier() {
        assert!(Combo::parse("KeyA").unwrap().check().is_err());
        assert!(Combo::parse("Shift+KeyA").unwrap().check().is_err());
        assert!(Combo::parse("Shift+Digit1").unwrap().check().is_err());
        assert!(Combo::parse("F9").unwrap().check().is_ok());
        assert!(Combo::parse("Alt+KeyA").unwrap().check().is_ok());
        assert!(Combo::parse("CmdOrCtrl+Shift+KeyA").unwrap().check().is_ok());
    }

    #[test]
    fn system_and_editor_shortcuts_are_reserved() {
        for s in ["CmdOrCtrl+KeyC", "CmdOrCtrl+KeyV", "CmdOrCtrl+KeyQ", "CmdOrCtrl+KeyS", "CmdOrCtrl+Comma", "CmdOrCtrl+Shift+KeyZ", "Alt+F4", "CmdOrCtrl+Alt+Delete"] {
            assert!(Combo::parse(s).unwrap().check().is_err(), "{s} should be reserved");
        }
        // With another modifier it is a different shortcut.
        assert!(Combo::parse("CmdOrCtrl+Alt+KeyC").unwrap().check().is_ok());
        assert!(Combo::parse("CmdOrCtrl+Shift+KeyC").unwrap().check().is_ok());
    }

    #[test]
    fn a_conflict_names_the_other_action() {
        let mut o = Bindings::new();
        let err = set_binding(&mut o, "marker", "CmdOrCtrl+Shift+KeyR").unwrap_err();
        assert!(err.contains("Start or stop recording"), "{err}");
        assert!(o.is_empty(), "a refused binding must not be stored");
        // Rebinding the other action first frees the combo.
        set_binding(&mut o, "record_toggle", "CmdOrCtrl+Shift+KeyG").unwrap();
        set_binding(&mut o, "marker", "CmdOrCtrl+Shift+KeyR").unwrap();
        assert_eq!(effective(&o, "marker"), "CmdOrCtrl+Shift+KeyR");
    }

    #[test]
    fn rebinding_to_the_default_drops_the_override_and_empty_unbinds() {
        let mut o = Bindings::new();
        set_binding(&mut o, "pause", "CmdOrCtrl+Shift+KeyG").unwrap();
        assert_eq!(o.get("pause").unwrap(), "CmdOrCtrl+Shift+KeyG");
        set_binding(&mut o, "pause", "shift+cmdorctrl+KeyP").unwrap();
        assert!(!o.contains_key("pause"));
        set_binding(&mut o, "pause", "").unwrap();
        assert_eq!(effective(&o, "pause"), "");
        // An unbound action does not conflict with another unbound one.
        set_binding(&mut o, "restart", "").unwrap();
        assert!(conflict_with(&o, "marker", "").is_none());
    }

    #[test]
    fn unknown_actions_are_refused() {
        assert!(check_binding(&Bindings::new(), "launch_rockets", "CmdOrCtrl+Alt+KeyL").is_err());
    }

    #[test]
    fn sanitize_drops_garbage_and_duplicates_but_keeps_valid_choices() {
        let mut raw = Bindings::new();
        raw.insert("pause".into(), "CmdOrCtrl+Shift+KeyG".into());
        raw.insert("marker".into(), "nonsense".into());
        raw.insert("nope".into(), "CmdOrCtrl+Alt+KeyL".into());
        raw.insert("toggle_mic".into(), "CmdOrCtrl+Shift+KeyG".into()); // duplicates pause
        raw.insert("toggle_keys".into(), "CmdOrCtrl+KeyC".into()); // reserved
        raw.insert("discard".into(), "".into()); // equals the default (unbound)
        let out = sanitize(&raw);
        assert_eq!(out.get("pause").unwrap(), "CmdOrCtrl+Shift+KeyG");
        assert!(!out.contains_key("marker") && !out.contains_key("nope") && !out.contains_key("toggle_mic") && !out.contains_key("toggle_keys") && !out.contains_key("discard"));
    }

    #[test]
    fn labels_read_naturally_on_each_platform() {
        assert_eq!(label("CmdOrCtrl+Shift+KeyR", false), "Ctrl+Shift+R");
        assert_eq!(label("CmdOrCtrl+Shift+KeyR", true), "Shift+Cmd+R");
        assert_eq!(label("CmdOrCtrl+Alt+Digit1", true), "Option+Cmd+1");
        assert_eq!(label("CmdOrCtrl+Alt+Digit1", false), "Ctrl+Alt+1");
        assert_eq!(label("F9", false), "F9");
        assert_eq!(label("Ctrl+ArrowUp", true), "Ctrl+Up");
        assert_eq!(label("", false), "");
    }
}
