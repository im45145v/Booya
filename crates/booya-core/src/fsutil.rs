//! Writing files so that a failure never leaves a half written one behind.
//!
//! A plain `std::fs::write` truncates the old file first. If the disk fills
//! up, the app is killed or the machine sleeps halfway, the project (or the
//! settings, or last week's export) is gone. Everything that replaces a file
//! the user cares about goes through here instead: the new bytes go to a
//! temporary file next to it, and only a finished file is renamed over the
//! old one.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A temporary name next to `path`, in the same folder (so the rename is a
/// plain rename on the same disk) and keeping the extension, because ffmpeg
/// picks the container from it.
pub fn sibling_tmp(path: &Path) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let name = match path.extension() {
        Some(ext) => format!(".{stem}.booya-part-{}-{n}.{}", std::process::id(), ext.to_string_lossy()),
        None => format!(".{stem}.booya-part-{}-{n}", std::process::id()),
    };
    path.with_file_name(name)
}

/// Replace `path` with `bytes`, or leave the old file exactly as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = sibling_tmp(path);
    let r = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        // On disk before the rename, or a crash can still leave an empty file.
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    // The reason ("No space left on device") is part of the message, because
    // the app shows errors with `{}`, which drops anyhow's context chain.
    r.map_err(|err| anyhow::anyhow!("could not save {}: {err}", path.display()))
}

/// An output file that only appears when it is finished. Write to
/// [`Staged::path`], then call [`Staged::commit`]. Dropped without a commit
/// (an error, a cancel, a panic), the partial file is deleted and whatever
/// was at the destination before is untouched.
pub struct Staged {
    tmp: PathBuf,
    out: PathBuf,
    done: bool,
}

impl Staged {
    pub fn new(out: &Path) -> Self {
        Self { tmp: sibling_tmp(out), out: out.to_path_buf(), done: false }
    }

    /// Where to write the file meanwhile.
    pub fn path(&self) -> &Path {
        &self.tmp
    }

    pub fn commit(mut self) -> Result<()> {
        std::fs::rename(&self.tmp, &self.out).map_err(|err| anyhow::anyhow!("could not move the finished file to {}: {err}", self.out.display()))?;
        self.done = true;
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.done {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// True when `a` and `b` name the same file, also when one of them does not
/// exist yet, is relative, or differs only in letter case on a file system
/// that ignores case (the usual one on Windows and macOS).
pub fn same_file(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> Option<PathBuf> {
        if let Ok(c) = p.canonicalize() {
            return Some(c);
        }
        let name = p.file_name()?;
        let dir = match p.parent().filter(|d| !d.as_os_str().is_empty()) {
            Some(d) => d.canonicalize().ok()?,
            None => std::env::current_dir().ok()?.canonicalize().ok()?,
        };
        Some(dir.join(name))
    }
    match (norm(a), norm(b)) {
        (Some(x), Some(y)) => {
            if cfg!(any(windows, target_os = "macos")) {
                x.to_string_lossy().to_lowercase() == y.to_string_lossy().to_lowercase()
            } else {
                x == y
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("booya-fsutil-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn leftovers(d: &Path) -> Vec<String> {
        std::fs::read_dir(d).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains("booya-part")).collect()
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_nothing_behind() {
        let d = dir("replace");
        let p = d.join("project.json");
        std::fs::write(&p, "old").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert!(leftovers(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let d = dir("fail");
        // The destination is a folder, so the rename fails after the bytes
        // were written: the old content (here, the folder) must survive and
        // the temporary file must go.
        let p = d.join("busy.json");
        std::fs::create_dir_all(p.join("inside")).unwrap();
        assert!(write_atomic(&p, b"new").is_err());
        assert!(p.join("inside").is_dir());
        assert!(leftovers(&d).is_empty(), "{:?}", leftovers(&d));
        // A folder that does not exist fails cleanly too.
        assert!(write_atomic(&d.join("missing").join("x.json"), b"x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_staged_output_appears_only_when_committed() {
        let d = dir("staged");
        let out = d.join("demo export.mp4");
        std::fs::write(&out, "last week's export").unwrap();
        {
            let s = Staged::new(&out);
            assert_eq!(s.path().extension().unwrap(), "mp4", "ffmpeg needs the extension");
            assert_eq!(s.path().parent(), out.parent());
            std::fs::write(s.path(), "half an export").unwrap();
            // dropped: an error or a cancel
        }
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "last week's export");
        assert!(leftovers(&d).is_empty());
        let s = Staged::new(&out);
        std::fs::write(s.path(), "the new export").unwrap();
        s.commit().unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "the new export");
        assert!(leftovers(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_same_file_is_recognised_through_dots_and_missing_files() {
        let d = dir("same");
        let a = d.join("rec.mp4");
        std::fs::write(&a, "x").unwrap();
        assert!(same_file(&a, &d.join(".").join("rec.mp4")));
        assert!(!same_file(&a, &d.join("other.mp4")));
        // Neither exists yet: compared by folder and name.
        assert!(same_file(&d.join("new.mp4"), &d.join("new.mp4")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
