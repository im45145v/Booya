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

pub fn load_preset(name: &str) -> Result<Preset> {
    let path = presets_dir().join(file_name(name));
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
}
