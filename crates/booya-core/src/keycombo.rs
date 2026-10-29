//! Turns raw keyboard facts (a key went down, a key went up, the held
//! modifiers changed) into the events the log stores.
//!
//! One rule decides what is kept: when Ctrl, Alt (Option), Win or Cmd (Super)
//! is held, the key that goes with it is stored by name, because that is a
//! shortcut (`Cmd+W`, `Ctrl+T`, `Alt+Tab`). Shift alone does not count. Plain
//! typing keeps only the fact that something was typed, never the letters.
//!
//! The builder is pure, so every backend (the macOS event tap, the Windows and
//! Linux hook helper, the macOS polling fallback) shares and tests the same
//! behaviour:
//!
//! - the modifiers are read at the moment the key goes down, so releasing Cmd
//!   before the letter does not lose the combo;
//! - a key held down repeats at the OS rate; only the first press is stored;
//! - a modifier pressed and released alone is only a `Mods` change, which the
//!   overlay shows by itself.

use crate::events::EventKind;
use crate::keylabels::ordered_mods;
use std::collections::HashMap;

/// Modifiers that make a key press a shortcut. Shift and Fn alone do not.
pub fn is_shortcut_modifier(m: &str) -> bool {
    matches!(m.to_ascii_lowercase().as_str(), "ctrl" | "control" | "alt" | "option" | "altgr" | "meta" | "cmd" | "command" | "win" | "super")
}

/// Does this set of held modifiers turn a key press into a shortcut?
pub fn makes_shortcut(mods: &[String]) -> bool {
    mods.iter().any(|m| is_shortcut_modifier(m))
}

fn canon(mods: &[String]) -> Vec<String> {
    ordered_mods(mods).into_iter().map(String::from).collect()
}

#[derive(Debug, Default)]
pub struct ComboBuilder {
    held: Vec<String>,
    /// Keys that are down, with the time we last heard of each.
    down: HashMap<String, f64>,
}

/// A key that has been quiet this long and arrives again is a new press: its
/// key-up was lost (a system shortcut such as Win+L can swallow it).
pub const STUCK_MS: f64 = 1500.0;

impl ComboBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The modifiers held right now, in display order.
    pub fn held(&self) -> &[String] {
        &self.held
    }

    /// The whole set of held modifiers is known (macOS flags). Returns a
    /// `Mods` event only when it changed.
    pub fn set_mods(&mut self, held: &[String]) -> Option<EventKind> {
        let held = canon(held);
        if held == self.held {
            return None;
        }
        self.held = held.clone();
        Some(EventKind::Mods { held })
    }

    /// One modifier key went down. Holding it repeats the press; only the
    /// change counts.
    pub fn modifier_down(&mut self, m: &str) -> Option<EventKind> {
        let mut next = self.held.clone();
        next.push(m.to_string());
        self.set_mods(&next)
    }

    pub fn modifier_up(&mut self, m: &str) -> Option<EventKind> {
        let canon_m = canon(&[m.to_string()]);
        let next: Vec<String> = self.held.iter().filter(|h| !canon_m.contains(h)).cloned().collect();
        self.set_mods(&next)
    }

    /// A non-modifier key went down at `now_ms`. `id` tells the key apart from
    /// others (its scan code or enum name), `key` is the name to store.
    /// `mods` are the modifiers the source saw with this very press (macOS
    /// event flags); `None` uses the tracked set. `repeat` is the source's own
    /// auto-repeat flag. Returns the `Key` event, or `None` for an
    /// auto-repeat of a key that is still down.
    pub fn key_down(&mut self, id: &str, key: &str, repeat: bool, mods: Option<&[String]>, now_ms: f64) -> Option<EventKind> {
        match self.down.get(id).copied() {
            // Still down: the OS repeating it, flagged or not.
            Some(last) if now_ms - last < STUCK_MS => {
                self.down.insert(id.to_string(), now_ms);
                return None;
            }
            // A repeat of a key whose press we never saw.
            None if repeat => {
                self.down.insert(id.to_string(), now_ms);
                return None;
            }
            _ => {}
        }
        self.down.insert(id.to_string(), now_ms);
        let modifiers = match mods {
            Some(m) => canon(m),
            None => self.held.clone(),
        };
        let key = crate::input::classify(key, &modifiers);
        Some(EventKind::Key { key, modifiers })
    }

    pub fn key_up(&mut self, id: &str) {
        self.down.remove(id);
    }

    /// Focus or the hook was lost: forget what is down, so a missed key-up
    /// cannot swallow the next press.
    pub fn reset(&mut self) {
        self.down.clear();
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn key(e: Option<EventKind>) -> (String, Vec<String>) {
        match e {
            Some(EventKind::Key { key, modifiers }) => (key, modifiers),
            other => panic!("not a key event: {other:?}"),
        }
    }

    #[test]
    fn a_letter_with_cmd_is_stored_by_name() {
        let mut b = ComboBuilder::new();
        assert_eq!(b.modifier_down("Meta"), Some(EventKind::Mods { held: s(&["Meta"]) }));
        assert_eq!(key(b.key_down("w", "w", false, None, 0.0)), ("W".into(), s(&["Meta"])));
    }

    #[test]
    fn every_kind_of_key_is_kept_with_a_modifier() {
        for (raw, shown) in [("KeyT", "T"), ("t", "T"), ("4", "4"), ("F5", "F5"), ("UpArrow", "Up"), ("Tab", "Tab"), ("Return", "Enter"), ("-", "-"), ("Escape", "Esc")] {
            for m in ["Ctrl", "Alt", "Meta"] {
                let mut b = ComboBuilder::new();
                b.modifier_down(m);
                assert_eq!(key(b.key_down(raw, raw, false, None, 0.0)).0, shown, "{m} + {raw}");
            }
        }
    }

    #[test]
    fn shift_alone_and_plain_typing_never_store_letters() {
        let mut b = ComboBuilder::new();
        assert_eq!(key(b.key_down("q", "q", false, None, 0.0)).0, crate::events::PRINTABLE);
        b.key_up("q");
        b.modifier_down("Shift");
        let (k, m) = key(b.key_down("Q", "Q", false, None, 0.0));
        assert_eq!((k.as_str(), m), (crate::events::PRINTABLE, s(&["Shift"])));
        // Fn on its own is not a shortcut either.
        b.reset();
        b.modifier_down("Fn");
        assert_eq!(key(b.key_down("e", "e", false, None, 0.0)).0, crate::events::PRINTABLE);
        // But a named key is always kept.
        assert_eq!(key(b.key_down("Enter", "Enter", false, None, 0.0)).0, "Enter");
    }

    #[test]
    fn shift_plus_cmd_is_still_a_shortcut_and_keeps_both() {
        let mut b = ComboBuilder::new();
        b.modifier_down("Shift");
        b.modifier_down("Meta");
        assert_eq!(key(b.key_down("4", "4", false, None, 0.0)), ("4".into(), s(&["Shift", "Meta"])));
    }

    #[test]
    fn the_modifier_released_first_does_not_lose_the_combo() {
        let mut b = ComboBuilder::new();
        b.modifier_down("Meta");
        let press = b.key_down("w", "w", false, None, 0.0);
        // Cmd comes up before W does.
        assert_eq!(b.modifier_up("Meta"), Some(EventKind::Mods { held: vec![] }));
        b.key_up("w");
        assert_eq!(key(press), ("W".into(), s(&["Meta"])));
        // A second press after Cmd is gone is plain typing again.
        assert_eq!(key(b.key_down("w", "w", false, None, 0.0)).0, crate::events::PRINTABLE);
    }

    #[test]
    fn a_held_key_is_one_press_and_pressing_again_is_another() {
        let mut b = ComboBuilder::new();
        b.modifier_down("Ctrl");
        assert!(b.key_down("z", "z", false, None, 0.0).is_some());
        for i in 1..=40 {
            assert!(b.key_down("z", "z", true, None, i as f64 * 33.0).is_none(), "auto-repeat is not a new press");
        }
        b.key_up("z");
        assert!(b.key_down("z", "z", false, None, 1400.0).is_some());
        b.key_up("z");
        assert!(b.key_down("z", "z", false, None, 1500.0).is_some());
    }

    #[test]
    fn a_repeat_that_arrives_without_the_flag_is_still_one_press() {
        // Hooks that cannot tell a repeat (key pressed again with no release).
        let mut b = ComboBuilder::new();
        b.modifier_down("Meta");
        assert!(b.key_down("z", "z", false, None, 0.0).is_some());
        assert!(b.key_down("z", "z", false, None, 40.0).is_none());
        assert!(b.key_down("z", "z", false, None, 500.0).is_none(), "the initial repeat delay is still a repeat");
    }

    #[test]
    fn a_lost_key_up_cannot_swallow_the_next_press() {
        let mut b = ComboBuilder::new();
        b.modifier_down("Meta");
        assert!(b.key_down("l", "l", false, None, 0.0).is_some());
        // The system took Win+L and never told us the key came up.
        assert!(b.key_down("l", "l", false, None, STUCK_MS + 10.0).is_some());
    }

    #[test]
    fn a_repeat_of_a_press_we_never_saw_is_ignored() {
        let mut b = ComboBuilder::new();
        b.modifier_down("Meta");
        assert!(b.key_down("z", "z", true, None, 0.0).is_none());
    }

    #[test]
    fn modifiers_alone_only_change_the_held_set() {
        let mut b = ComboBuilder::new();
        assert_eq!(b.modifier_down("Alt"), Some(EventKind::Mods { held: s(&["Alt"]) }));
        assert_eq!(b.modifier_down("Alt"), None, "holding repeats the press");
        assert_eq!(b.modifier_up("Alt"), Some(EventKind::Mods { held: vec![] }));
        assert_eq!(b.modifier_up("Alt"), None);
    }

    #[test]
    fn flags_from_the_event_win_over_what_was_tracked() {
        let mut b = ComboBuilder::new();
        // The press arrived before the modifier change was seen.
        assert_eq!(key(b.key_down("t", "t", false, Some(&s(&["Ctrl"])), 0.0)), ("T".into(), s(&["Ctrl"])));
    }

    #[test]
    fn what_makes_a_shortcut() {
        assert!(makes_shortcut(&s(&["Meta"])) && makes_shortcut(&s(&["Shift", "Ctrl"])) && makes_shortcut(&s(&["Alt"])));
        assert!(!makes_shortcut(&s(&["Shift"])) && !makes_shortcut(&s(&["Fn"])) && !makes_shortcut(&[]));
    }
}
