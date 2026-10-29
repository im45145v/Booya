//! Project folders (`*.booya`).
//!
//! A bundle is an ordinary folder you can copy, zip or sync:
//!
//! ```text
//! My demo.booya/
//!   project.json      the edit (paths inside are relative to this folder)
//!   transcript.json   the captions as plain text, readable without Booya
//!   media/            the recording (and the webcam file)
//!   events/           the mouse and key log
//!   assets/           background photos
//! ```
//!
//! Opening one finds its media by relative path, so the folder works from
//! wherever it is moved to. A file that has gone missing is reported, and
//! `relocate` points the project at where it is now.

use crate::project::{Background, Project};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

pub const MANIFEST: &str = "project.json";
pub const TRANSCRIPT: &str = "transcript.json";

/// A file the project refers to that is not where it should be.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Missing {
    /// "source", "events", "webcam" or "background".
    pub field: String,
    /// Where the project expects it.
    pub path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct Opened {
    pub project: Project,
    pub missing: Vec<Missing>,
}

pub fn is_bundle(dir: &Path) -> bool {
    dir.is_dir() && dir.join(MANIFEST).is_file()
}

/// `dir` for a folder, or the folder holding a `project.json` that was chosen.
pub fn bundle_dir_of(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        return is_bundle(path).then(|| path.to_path_buf());
    }
    if path.file_name().is_some_and(|n| n == MANIFEST) {
        return path.parent().map(Path::to_path_buf).filter(|d| is_bundle(d));
    }
    None
}

fn clean(name: &std::ffi::OsStr) -> String {
    let n: String = name.to_string_lossy().chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    if n.is_empty() || n == "." || n == ".." {
        "file".into()
    } else {
        n
    }
}

/// Put `src` in `dir/sub` (a hard link when the disk allows it, else a copy)
/// and return where it is now. A file already inside `dir`, or one that no
/// longer exists, is left alone.
fn place(src: &Path, sub: &str, dir: &Path) -> Result<PathBuf> {
    if src.starts_with(dir) || !src.is_file() || src.canonicalize().is_ok_and(|c| c.starts_with(dir)) {
        return Ok(src.to_path_buf());
    }
    let folder = dir.join(sub);
    std::fs::create_dir_all(&folder)?;
    let name = clean(src.file_name().unwrap_or_default());
    let want_len = std::fs::metadata(src)?.len();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    for n in 0.. {
        let target = if n == 0 { folder.join(&name) } else { folder.join(format!("{stem}-{n}{ext}")) };
        match std::fs::metadata(&target) {
            // The same file from an earlier save.
            Ok(m) if m.len() == want_len && same_bytes(src, &target) => return Ok(target),
            Ok(_) => continue,
            Err(_) => {
                if std::fs::hard_link(src, &target).is_err() {
                    let part = target.with_extension("part");
                    std::fs::copy(src, &part).with_context(|| format!("copy {}", src.display()))?;
                    std::fs::rename(&part, &target)?;
                }
                return Ok(target);
            }
        }
    }
    unreachable!()
}

/// Same size is not enough to call two files one (two takes can be equal in
/// length), so the first and last 64 KiB are compared too.
fn same_bytes(a: &Path, b: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let (Ok(mut fa), Ok(mut fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let Ok(len) = fa.metadata().map(|m| m.len()) else { return false };
    for off in [0, len.saturating_sub(65536)] {
        let (mut x, mut y) = (vec![0u8; 65536], vec![0u8; 65536]);
        if fa.seek(SeekFrom::Start(off)).is_err() || fb.seek(SeekFrom::Start(off)).is_err() {
            return false;
        }
        let (nx, ny) = (fa.read(&mut x).unwrap_or(0), fb.read(&mut y).unwrap_or(1));
        if nx != ny || x[..nx] != y[..ny] {
            return false;
        }
    }
    true
}

fn relative(p: &Path, dir: &Path) -> PathBuf {
    match p.strip_prefix(dir) {
        // Always forward slashes, so a bundle made on Windows opens on a Mac and back.
        Ok(r) => PathBuf::from(
            r.components()
                .filter_map(|c| match c {
                    Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/"),
        ),
        Err(_) => p.to_path_buf(),
    }
}

fn absolute(p: &mut PathBuf, dir: &Path) {
    if p.is_relative() {
        *p = dir.join(&*p);
    }
}

/// Write `project` into `dir` (created if needed), copying in the media it
/// uses. Returns the project as it is now, with paths inside `dir`.
pub fn save_bundle(project: &Project, dir: &Path) -> Result<Project> {
    if dir.is_file() {
        bail!("{} is a file, not a folder", dir.display());
    }
    std::fs::create_dir_all(dir)?;
    let dir = &dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut p = project.clone();
    p.source = place(&p.source, "media", dir)?;
    if let Some(e) = p.events.as_mut() {
        *e = place(e, "events", dir)?;
    }
    if let Some(w) = p.webcam.as_mut() {
        w.path = place(&w.path, "media", dir)?;
    }
    if let Background::Image { path } = &mut p.style.background {
        *path = place(path, "assets", dir)?;
    }
    // The manifest has relative paths; the project handed back has real ones.
    let mut disk = p.clone();
    disk.source = relative(&disk.source, dir);
    if let Some(e) = disk.events.as_mut() {
        *e = relative(e, dir);
    }
    if let Some(w) = disk.webcam.as_mut() {
        w.path = relative(&w.path, dir);
    }
    if let Background::Image { path } = &mut disk.style.background {
        *path = relative(path, dir);
    }
    write_atomic(&dir.join(MANIFEST), serde_json::to_string_pretty(&disk)?.as_bytes())?;
    write_atomic(&dir.join(TRANSCRIPT), serde_json::to_string_pretty(&transcript(&p))?.as_bytes())?;
    Ok(p)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The captions as plain text with times in seconds of the source.
fn transcript(p: &Project) -> serde_json::Value {
    serde_json::json!({
        "note": "Written by Booya from the captions in project.json. Edit captions in Booya, not here.",
        "captions": p.captions.iter().map(|c| serde_json::json!({
            "start_s": (c.start_ms / 10.0).round() / 100.0,
            "end_s": (c.end_ms / 10.0).round() / 100.0,
            "text": c.text,
        })).collect::<Vec<_>>(),
        "text": p.captions.iter().map(|c| c.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "),
    })
}

fn check(p: &Project) -> Vec<Missing> {
    let mut m = Vec::new();
    let mut want = |field: &str, path: &Path| {
        if !path.exists() {
            m.push(Missing { field: field.into(), path: path.to_path_buf() });
        }
    };
    want("source", &p.source);
    if let Some(e) = &p.events {
        want("events", e);
    }
    if let Some(w) = &p.webcam {
        want("webcam", &w.path);
    }
    if let Background::Image { path } = &p.style.background {
        want("background", path);
    }
    m
}

/// Which files a project lists that cannot be found right now.
pub fn missing(p: &Project) -> Vec<Missing> {
    check(p)
}

/// Open a project given as a bundle folder, its `project.json`, or an older
/// loose project file. Reports the media that is missing instead of failing.
pub fn open(path: &Path) -> Result<Opened> {
    let (file, dir) = match bundle_dir_of(path) {
        Some(d) => (d.join(MANIFEST), d),
        None if path.is_file() => (path.to_path_buf(), path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))),
        None if path.is_dir() => bail!("{} is not a Booya project (no {MANIFEST} inside)", path.display()),
        None => bail!("{} does not exist", path.display()),
    };
    let mut project = Project::load(&file)?;
    absolute(&mut project.source, &dir);
    if let Some(e) = project.events.as_mut() {
        absolute(e, &dir);
    }
    if let Some(w) = project.webcam.as_mut() {
        absolute(&mut w.path, &dir);
    }
    if let Background::Image { path } = &mut project.style.background {
        absolute(path, &dir);
    }
    project.validate()?;
    let missing = check(&project);
    Ok(Opened { project, missing })
}

/// Point one missing file at where it is now.
pub fn relocate(project: &mut Project, field: &str, to: &Path) -> Result<()> {
    if !to.is_file() {
        bail!("{} is not a file", to.display());
    }
    match field {
        "source" => project.source = to.to_path_buf(),
        "events" => project.events = Some(to.to_path_buf()),
        "webcam" => match project.webcam.as_mut() {
            Some(w) => w.path = to.to_path_buf(),
            None => bail!("this project has no webcam"),
        },
        "background" => project.style.background = Background::Image { path: to.to_path_buf() },
        other => bail!("unknown file kind {other}"),
    }
    Ok(())
}

/// Look for the missing files next to a file the user found (the recording
/// often moves with its siblings). Returns how many it fixed.
pub fn relocate_siblings(project: &mut Project, found: &Path) -> usize {
    let Some(folder) = found.parent() else { return 0 };
    let mut n = 0;
    let mut try_fix = |p: &mut PathBuf| {
        if !p.exists() {
            if let Some(name) = p.file_name() {
                let c = folder.join(name);
                if c.is_file() {
                    *p = c;
                    n += 1;
                }
            }
        }
    };
    try_fix(&mut project.source);
    if let Some(e) = project.events.as_mut() {
        try_fix(e);
    }
    if let Some(w) = project.webcam.as_mut() {
        try_fix(&mut w.path);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dl-bundle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample(d: &Path) -> Project {
        let v = d.join("rec.mp4");
        let e = d.join("rec.events.jsonl");
        std::fs::write(&v, vec![7u8; 100_000]).unwrap();
        std::fs::write(&e, "{}\n").unwrap();
        let mut p = Project::new(v, Some(e), 1920, 1080, 5000.0);
        p.captions.push(crate::project::Caption { start_ms: 0.0, end_ms: 1500.0, text: "Hello there".into(), words: vec![] });
        p
    }

    #[test]
    fn a_saved_bundle_is_self_contained_and_reopens_after_a_move() {
        let d = tmp("roundtrip");
        let p = sample(&d);
        let b = d.join("Demo.booya");
        let saved = save_bundle(&p, &b).unwrap();
        assert!(saved.source.starts_with(b.canonicalize().unwrap()));
        let disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(b.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(disk["source"], "media/rec.mp4");
        assert_eq!(disk["events"], "events/rec.events.jsonl");
        let t = std::fs::read_to_string(b.join(TRANSCRIPT)).unwrap();
        assert!(t.contains("Hello there"));
        // The original can go; the copy in the bundle is what counts.
        std::fs::remove_file(&p.source).unwrap();
        let moved = d.join("elsewhere.booya");
        std::fs::rename(&b, &moved).unwrap();
        let o = open(&moved).unwrap();
        assert!(o.missing.is_empty(), "{:?}", o.missing);
        assert_eq!(o.project.captions[0].text, "Hello there");
        assert!(o.project.source.starts_with(&moved));
        // Choosing project.json inside the folder is the same thing.
        assert!(open(&moved.join(MANIFEST)).unwrap().missing.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn saving_twice_does_not_copy_twice_and_two_files_of_one_name_both_survive() {
        let d = tmp("twice");
        let mut p = sample(&d);
        let b = d.join("Demo.booya");
        let a1 = save_bundle(&p, &b).unwrap();
        let a2 = save_bundle(&a1, &b).unwrap();
        assert_eq!(a1.source, a2.source);
        assert_eq!(std::fs::read_dir(b.join("media")).unwrap().count(), 1);
        // A different file with the same name does not overwrite the first.
        let other = d.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("rec.mp4"), vec![9u8; 100_000]).unwrap();
        p.source = other.join("rec.mp4");
        let a3 = save_bundle(&p, &b).unwrap();
        assert_ne!(a3.source, a1.source);
        assert_eq!(std::fs::read_dir(b.join("media")).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_media_is_reported_and_can_be_pointed_at_its_new_home() {
        let d = tmp("missing");
        let p = sample(&d);
        let b = d.join("Demo.booya");
        save_bundle(&p, &b).unwrap();
        std::fs::remove_file(b.join("media/rec.mp4")).unwrap();
        let mut o = open(&b).unwrap();
        assert_eq!(o.missing.len(), 1);
        assert_eq!(o.missing[0].field, "source");
        relocate(&mut o.project, "source", &p.source).unwrap();
        assert!(missing(&o.project).is_empty());
        assert!(relocate(&mut o.project, "source", &d.join("nope.mp4")).is_err());
        // Sibling search: the log moved along with the recording.
        let mut q = o.project.clone();
        q.source = d.join("gone/rec.mp4");
        q.events = Some(d.join("gone/rec.events.jsonl"));
        assert_eq!(relocate_siblings(&mut q, &p.source), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn older_loose_project_files_still_open() {
        let d = tmp("loose");
        let p = sample(&d);
        p.save(&d.join("old.json")).unwrap();
        let o = open(&d.join("old.json")).unwrap();
        assert!(o.missing.is_empty());
        assert!(open(&d).is_err(), "a plain folder is not a project");
        let _ = std::fs::remove_dir_all(&d);
    }
}
