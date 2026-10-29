//! Turns a binding from `booya_core::shortcuts` into an OS shortcut.

use booya_core::shortcuts::Combo;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

/// Every key name `booya_core::shortcuts::valid_key` accepts.
fn code_from_name(name: &str) -> Option<Code> {
    Some(match name {
        "KeyA" => Code::KeyA,
        "KeyB" => Code::KeyB,
        "KeyC" => Code::KeyC,
        "KeyD" => Code::KeyD,
        "KeyE" => Code::KeyE,
        "KeyF" => Code::KeyF,
        "KeyG" => Code::KeyG,
        "KeyH" => Code::KeyH,
        "KeyI" => Code::KeyI,
        "KeyJ" => Code::KeyJ,
        "KeyK" => Code::KeyK,
        "KeyL" => Code::KeyL,
        "KeyM" => Code::KeyM,
        "KeyN" => Code::KeyN,
        "KeyO" => Code::KeyO,
        "KeyP" => Code::KeyP,
        "KeyQ" => Code::KeyQ,
        "KeyR" => Code::KeyR,
        "KeyS" => Code::KeyS,
        "KeyT" => Code::KeyT,
        "KeyU" => Code::KeyU,
        "KeyV" => Code::KeyV,
        "KeyW" => Code::KeyW,
        "KeyX" => Code::KeyX,
        "KeyY" => Code::KeyY,
        "KeyZ" => Code::KeyZ,
        "Digit0" => Code::Digit0,
        "Digit1" => Code::Digit1,
        "Digit2" => Code::Digit2,
        "Digit3" => Code::Digit3,
        "Digit4" => Code::Digit4,
        "Digit5" => Code::Digit5,
        "Digit6" => Code::Digit6,
        "Digit7" => Code::Digit7,
        "Digit8" => Code::Digit8,
        "Digit9" => Code::Digit9,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "F13" => Code::F13,
        "F14" => Code::F14,
        "F15" => Code::F15,
        "F16" => Code::F16,
        "F17" => Code::F17,
        "F18" => Code::F18,
        "F19" => Code::F19,
        "F20" => Code::F20,
        "F21" => Code::F21,
        "F22" => Code::F22,
        "F23" => Code::F23,
        "F24" => Code::F24,
        "Space" => Code::Space,
        "Enter" => Code::Enter,
        "Backspace" => Code::Backspace,
        "Tab" => Code::Tab,
        "ArrowUp" => Code::ArrowUp,
        "ArrowDown" => Code::ArrowDown,
        "ArrowLeft" => Code::ArrowLeft,
        "ArrowRight" => Code::ArrowRight,
        "Backquote" => Code::Backquote,
        "Minus" => Code::Minus,
        "Equal" => Code::Equal,
        "BracketLeft" => Code::BracketLeft,
        "BracketRight" => Code::BracketRight,
        "Backslash" => Code::Backslash,
        "Semicolon" => Code::Semicolon,
        "Quote" => Code::Quote,
        "Comma" => Code::Comma,
        "Period" => Code::Period,
        "Slash" => Code::Slash,
        "Home" => Code::Home,
        "End" => Code::End,
        "PageUp" => Code::PageUp,
        "PageDown" => Code::PageDown,
        "Insert" => Code::Insert,
        "Delete" => Code::Delete,
        _ => return None,
    })
}

/// The OS shortcut for a combo string, or None if it is empty or invalid.
pub fn to_shortcut(combo: &str) -> Option<Shortcut> {
    let c = Combo::parse(combo).ok()?;
    let code = code_from_name(&c.key)?;
    let mut m = Modifiers::empty();
    if c.cmd_or_ctrl {
        m |= if cfg!(target_os = "macos") { Modifiers::SUPER } else { Modifiers::CONTROL };
    }
    if c.ctrl {
        m |= Modifiers::CONTROL;
    }
    if c.sup {
        m |= Modifiers::SUPER;
    }
    if c.alt {
        m |= Modifiers::ALT;
    }
    if c.shift {
        m |= Modifiers::SHIFT;
    }
    Some(Shortcut::new(if m.is_empty() { None } else { Some(m) }, code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_valid_key_name_has_a_code() {
        let mut names: Vec<String> = ('A'..='Z').map(|c| format!("Key{c}")).collect();
        names.extend((0..=9).map(|d| format!("Digit{d}")));
        names.extend((1..=24).map(|n| format!("F{n}")));
        for n in names {
            assert!(booya_core::shortcuts::valid_key(&n));
            assert!(code_from_name(&n).is_some(), "{n}");
        }
        assert!(to_shortcut("CmdOrCtrl+Shift+KeyR").is_some());
        assert!(to_shortcut("").is_none());
    }
}
