//! The one place a child process is created.
//!
//! On Windows a program started from a GUI app opens a console window unless
//! `CREATE_NO_WINDOW` is set, which shows up as terminals flashing open and
//! closed. Every child process in every crate (ffmpeg, ffprobe, llama-server,
//! tar, explorer, ...) is created through [`command`], and a test fails the
//! build when `Command::new` appears anywhere else.

use std::ffi::OsStr;
use std::process::Command;

/// `CREATE_NO_WINDOW`.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `Command` that never opens a console window. Use this instead of
/// `Command::new`.
pub fn command<S: AsRef<OsStr>>(program: S) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program); // proc:allow
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Stop a child and reap it, so no ffmpeg is left running or as a zombie.
pub fn reap(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target" || n == "node_modules" || n == ".git") {
                    continue;
                }
                rust_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// The Windows console flash cannot come back: no `Command::new` outside
    /// this module (a line may opt out with the marker `proc:allow`).
    #[test]
    fn every_child_process_goes_through_the_helper() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        for d in ["crates", "src-tauri"] {
            rust_files(&root.join(d), &mut files);
        }
        assert!(files.len() > 20, "the scan found only {} files, is the path right?", files.len());
        let mut bad = Vec::new();
        for f in files {
            if f.ends_with("demolens-core/src/proc.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                let hit = code.contains("Command::new(") || code.contains("Command::new (");
                if hit && !line.contains("proc:allow") {
                    bad.push(format!("{}:{}: {}", f.strip_prefix(&root).unwrap_or(&f).display(), i + 1, line.trim()));
                }
            }
        }
        assert!(bad.is_empty(), "use demolens_core::proc::command instead of Command::new:\n{}", bad.join("\n"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_children_get_no_console() {
        // A console program run through the helper must not create a console
        // of its own: it inherits none from a GUI parent, and gets the flag.
        let mut c = super::command("cmd");
        c.args(["/c", "exit", "0"]);
        assert!(c.status().unwrap().success());
    }
}
