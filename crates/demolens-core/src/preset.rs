//! Product presets: a named brand style (background, padding, corners,
//! shadow, cursor and key overlay) that can be reapplied to any project.

use crate::project::{CursorStyle, KeyOverlay, Project, Style};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub style: Style,
    pub cursor: CursorStyle,
    pub keys: KeyOverlay,
}

impl Preset {
    pub fn from_project(name: &str, p: &Project) -> Self {
        Self { name: name.into(), style: p.style.clone(), cursor: p.cursor.clone(), keys: p.keys.clone() }
    }

    pub fn apply(&self, p: &mut Project) {
        p.style = self.style.clone();
        p.cursor = self.cursor.clone();
        p.keys = self.keys.clone();
    }
}

/// Presets live in the user config dir, one JSON file per product.
pub fn presets_dir() -> PathBuf {
    if let Ok(d) = std::env::var("DEMOLENS_CONFIG_DIR") {
        return PathBuf::from(d).join("presets");
    }
    let base = if cfg!(windows) {
        std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME").map(|h| PathBuf::from(h).join("Library/Application Support")).unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|_| PathBuf::from("."))
    };
    base.join("demolens").join("presets")
}

fn file_name(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{safe}.json")
}

pub fn save_preset(p: &Preset) -> Result<PathBuf> {
    let dir = presets_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(&p.name));
    std::fs::write(&path, serde_json::to_string_pretty(p)?)?;
    Ok(path)
}

/// Built in looks, so a first export looks good without designing a style.
pub fn builtin_presets() -> Vec<Preset> {
    use crate::project::Background::{Gradient, Solid};
    let mk = |name: &str, bg, padding, corner_radius, shadow, cursor_size, ripple: [u8; 3]| {
        let mut p = Preset { name: name.into(), style: Style::default(), cursor: CursorStyle::default(), keys: KeyOverlay::default() };
        p.style.background = bg;
        p.style.padding = padding;
        p.style.corner_radius = corner_radius;
        p.style.shadow = shadow;
        p.cursor.size = cursor_size;
        p.cursor.ripple_color = ripple;
        p
    };
    vec![
        mk("Sunset", Gradient { from: [99, 102, 241], to: [236, 72, 153], angle_deg: 35.0 }, 0.07, 0.018, 0.035, 1.5, [255, 255, 255]),
        mk("Midnight", Gradient { from: [15, 23, 42], to: [49, 46, 129], angle_deg: 120.0 }, 0.08, 0.02, 0.05, 1.6, [129, 140, 248]),
        mk("Ocean", Gradient { from: [14, 165, 233], to: [16, 185, 129], angle_deg: 60.0 }, 0.07, 0.018, 0.035, 1.5, [255, 255, 255]),
        mk("Clean light", Solid { color: [241, 245, 249] }, 0.06, 0.014, 0.025, 1.4, [79, 70, 229]),
        mk("Minimal", Solid { color: [17, 17, 20] }, 0.0, 0.0, 0.0, 1.3, [255, 255, 255]),
        mk("Social card", Gradient { from: [251, 191, 36], to: [239, 68, 68], angle_deg: 45.0 }, 0.1, 0.03, 0.05, 1.8, [255, 255, 255]),
    ]
}

pub fn load_preset(name: &str) -> Result<Preset> {
    let path = presets_dir().join(file_name(name));
    if !path.exists() {
        if let Some(b) = builtin_presets().into_iter().find(|p| p.name == name) {
            return Ok(b);
        }
    }
    let s = std::fs::read_to_string(&path).with_context(|| format!("no preset named {name}"))?;
    Ok(serde_json::from_str(&s)?)
}

pub fn list_presets() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(presets_dir()) {
        for e in rd.flatten() {
            if let Ok(s) = std::fs::read_to_string(e.path()) {
                if let Ok(p) = serde_json::from_str::<Preset>(&s) {
                    out.push(p.name);
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Background;

    #[test]
    fn preset_roundtrip_and_apply() {
        let dir = std::env::temp_dir().join(format!("demolens-test-{}", std::process::id()));
        std::env::set_var("DEMOLENS_CONFIG_DIR", &dir);
        let mut a = Project::new("a.mp4".into(), None, 10, 10, 1000.0);
        a.style.background = Background::Solid { color: [1, 2, 3] };
        a.cursor.size = 2.5;
        save_preset(&Preset::from_project("Acme App", &a)).unwrap();
        assert!(list_presets().contains(&"Acme App".to_string()));
        let mut b = Project::new("b.mp4".into(), None, 10, 10, 1000.0);
        load_preset("Acme App").unwrap().apply(&mut b);
        assert_eq!(b.style, a.style);
        assert_eq!(b.cursor.size, 2.5);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builtins_load_by_name() {
        let names: Vec<_> = builtin_presets().into_iter().map(|p| p.name).collect();
        assert!(names.len() >= 5);
        let mut p = Project::new("a.mp4".into(), None, 10, 10, 1000.0);
        load_preset("Midnight").unwrap().apply(&mut p);
        assert_eq!(p.cursor.ripple_color, [129, 140, 248]);
    }
}
