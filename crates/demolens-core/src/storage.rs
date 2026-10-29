//! Disk usage of the recordings folder, and finding recordings that are empty
//! or broken so the app can offer to clean them up. Nothing here deletes
//! anything on its own: [`delete_recording`] is only called after the user
//! confirmed in the UI, and only accepts folders directly inside the root.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Ok,
    /// No file with content: left over from a start that failed.
    Empty,
    /// Has files but the video cannot be used.
    Broken,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingEntry {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    /// Seconds since the Unix epoch.
    pub modified: u64,
    pub health: Health,
    /// Plain language reason when not Ok.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageReport {
    pub root: PathBuf,
    pub total_bytes: u64,
    pub recordings: Vec<RecordingEntry>,
    pub empty: usize,
    pub broken: usize,
    pub reclaimable_bytes: u64,
    /// Size of the editing proxy cache.
    pub proxy_bytes: u64,
}

fn dir_bytes(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// The video file of a recording folder, if any (mp4 preferred).
pub fn video_of(dir: &Path) -> Option<PathBuf> {
    ["recording.mp4", "recording.mkv"].iter().map(|n| dir.join(n)).find(|p| p.metadata().map(|m| m.len() > 0).unwrap_or(false))
}

/// Classify one recording folder.
pub fn classify(dir: &Path) -> (Health, String) {
    let bytes = dir_bytes(dir);
    let Some(video) = video_of(dir) else {
        // Only a few KB of metadata at most.
        return if bytes < 16 * 1024 {
            (Health::Empty, "Nothing was recorded (the start failed).".into())
        } else {
            (Health::Broken, "There is no video file in this folder.".into())
        };
    };
    let info = match crate::ffmpeg::probe(&video) {
        Ok(i) => i,
        Err(e) => return (Health::Broken, format!("The video cannot be read: {}", e.to_string().lines().next().unwrap_or(""))),
    };
    if info.width == 0 || info.height == 0 {
        return (Health::Broken, "The video has no picture.".into());
    }
    // A duration that is far longer than the event log means the container is
    // damaged (a recording that was never closed properly).
    let events = dir.join("recording.events.jsonl");
    if let Ok(log) = crate::events::EventLog::load(&events) {
        let last = log.events.iter().map(|e| e.t).fold(0.0_f64, f64::max);
        if last > 1000.0 && info.duration_ms > last * 3.0 + 120_000.0 {
            return (
                Health::Broken,
                format!("The video says {:.0} s but the event log ends at {:.0} s: the file was not closed properly.", info.duration_ms / 1000.0, last / 1000.0),
            );
        }
    }
    (Health::Ok, String::new())
}

/// Scan `root` (the recordings folder). `cache_dir` is the app cache with the proxies.
pub fn scan(root: &Path, cache_dir: Option<&Path>) -> StorageReport {
    let mut recordings = Vec::new();
    for ent in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let p = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        // is_dir() follows symlinks; a symlink is never listed or deleted.
        let is_real_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_real_dir || !name.starts_with("recording-") {
            continue;
        }
        let bytes = dir_bytes(&p);
        let modified = ent.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        let (health, reason) = classify(&p);
        recordings.push(RecordingEntry { path: p, name, bytes, modified, health, reason });
    }
    recordings.sort_by_key(|r| std::cmp::Reverse(r.modified));
    let total_bytes = recordings.iter().map(|r| r.bytes).sum();
    let empty = recordings.iter().filter(|r| r.health == Health::Empty).count();
    let broken = recordings.iter().filter(|r| r.health == Health::Broken).count();
    let reclaimable_bytes = recordings.iter().filter(|r| r.health != Health::Ok).map(|r| r.bytes).sum();
    let proxy_bytes = cache_dir.map(|c| dir_bytes(&c.join("proxies"))).unwrap_or(0);
    StorageReport { root: root.to_path_buf(), total_bytes, recordings, empty, broken, reclaimable_bytes, proxy_bytes }
}

/// Delete one recording folder. Refuses anything that is not a real folder
/// named `recording-*` directly inside `root` (no symlinks, no `..`).
pub fn delete_recording(root: &Path, dir: &Path) -> Result<u64> {
    let root_c = root.canonicalize().context("recordings folder")?;
    let meta = std::fs::symlink_metadata(dir).context("recording folder")?;
    if !meta.is_dir() {
        bail!("not a recording folder");
    }
    let c = dir.canonicalize()?;
    let ok_name = c.file_name().is_some_and(|n| n.to_string_lossy().starts_with("recording-"));
    if c.parent() != Some(root_c.as_path()) || !ok_name {
        bail!("Only Booya recording folders can be deleted here");
    }
    let bytes = dir_bytes(&c);
    std::fs::remove_dir_all(&c)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dl-storage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn empty_and_broken_folders_are_found_and_good_ones_kept() {
        let root = tmp("scan");
        std::fs::create_dir_all(root.join("recording-1")).unwrap();
        std::fs::create_dir_all(root.join("recording-2")).unwrap();
        std::fs::write(root.join("recording-2").join("recording.mkv"), vec![7u8; 4096]).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let r = scan(&root, None);
        assert_eq!(r.recordings.len(), 2);
        assert_eq!(r.empty, 1);
        assert_eq!(r.broken, 1);
        assert!(r.recordings.iter().any(|x| x.name == "recording-1" && x.health == Health::Empty));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_healthy_recording_is_ok() {
        if !crate::ffmpeg::ffmpeg_found() {
            return;
        }
        let root = tmp("ok");
        let d = root.join("recording-3");
        std::fs::create_dir_all(&d).unwrap();
        let ok = crate::proc::command(crate::ffmpeg::ffmpeg_bin())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=320x180:r=10:d=2", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(d.join("recording.mp4"))
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert_eq!(classify(&d).0, Health::Ok);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_refuses_anything_outside_the_root() {
        let root = tmp("del");
        let outside = tmp("del-outside");
        std::fs::create_dir_all(root.join("recording-9")).unwrap();
        std::fs::create_dir_all(root.join("keepme")).unwrap();
        assert!(delete_recording(&root, &outside).is_err());
        assert!(delete_recording(&root, &root.join("keepme")).is_err());
        assert!(delete_recording(&root, &root.join("recording-9").join("..").join("keepme")).is_err());
        assert!(outside.exists() && root.join("keepme").exists());
        assert!(delete_recording(&root, &root.join("recording-9")).is_ok());
        assert!(!root.join("recording-9").exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
