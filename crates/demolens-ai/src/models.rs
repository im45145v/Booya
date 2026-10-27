//! Model manager. Models live in the user data dir and are optional: every
//! feature checks for its model and explains what to download if missing.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Speech,
    Ocr,
    Llm,
    Runtime,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub kind: Kind,
    pub title: &'static str,
    /// Approximate download size in bytes.
    pub size: u64,
    pub license: &'static str,
    #[serde(skip)]
    pub url: &'static str,
    /// File name on disk. Archives are extracted into a folder of this name.
    pub file: &'static str,
    #[serde(skip)]
    pub archive: bool,
}

const LLAMA_TAG: &str = "b11913";

macro_rules! llama_url {
    ($asset:literal) => {
        concat!("https://github.com/ggml-org/llama.cpp/releases/download/b11913/llama-b11913-bin-", $asset)
    };
}

fn llama_runtime() -> Option<ModelInfo> {
    let (url, size) = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (llama_url!("ubuntu-x64.tar.gz"), 22_474_748)
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (llama_url!("ubuntu-arm64.tar.gz"), 18_210_224)
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        (llama_url!("macos-arm64.tar.gz"), 16_295_309)
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        (llama_url!("macos-x64.tar.gz"), 12_035_739)
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        (llama_url!("win-cpu-x64.zip"), 24_218_562)
    } else {
        return None;
    };
    let _ = LLAMA_TAG;
    Some(ModelInfo {
        id: "llama-runtime",
        kind: Kind::Runtime,
        title: "llama.cpp server (CPU) to run local LLMs without Ollama",
        size,
        license: "MIT",
        url,
        file: "llama-runtime",
        archive: true,
    })
}

pub fn catalog() -> Vec<ModelInfo> {
    let mut v = vec![
        ModelInfo { id: "whisper-tiny.en", kind: Kind::Speech, title: "Whisper tiny (English), fastest", size: 77_704_715, license: "MIT", url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin", file: "ggml-tiny.en.bin", archive: false },
        ModelInfo { id: "whisper-base.en", kind: Kind::Speech, title: "Whisper base (English), recommended", size: 147_964_211, license: "MIT", url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin", file: "ggml-base.en.bin", archive: false },
        ModelInfo { id: "whisper-small", kind: Kind::Speech, title: "Whisper small (99 languages)", size: 487_601_967, license: "MIT", url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin", file: "ggml-small.bin", archive: false },
        ModelInfo { id: "ocr-detection", kind: Kind::Ocr, title: "ocrs text detection", size: 2_510_284, license: "CC-BY-SA-4.0", url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten", file: "text-detection.rten", archive: false },
        ModelInfo { id: "ocr-recognition", kind: Kind::Ocr, title: "ocrs text recognition", size: 9_716_568, license: "CC-BY-SA-4.0", url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten", file: "text-recognition.rten", archive: false },
        ModelInfo { id: "qwen2.5-0.5b", kind: Kind::Llm, title: "Qwen2.5 0.5B Instruct Q4_K_M, tiny and quick", size: 491_400_032, license: "Apache-2.0", url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf", file: "qwen2.5-0.5b-instruct-q4_k_m.gguf", archive: false },
        ModelInfo { id: "qwen2.5-1.5b", kind: Kind::Llm, title: "Qwen2.5 1.5B Instruct Q4_K_M, better writing", size: 1_117_320_736, license: "Apache-2.0", url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf", file: "qwen2.5-1.5b-instruct-q4_k_m.gguf", archive: false },
    ];
    v.extend(llama_runtime());
    v
}

pub fn find(id: &str) -> Result<ModelInfo> {
    catalog().into_iter().find(|m| m.id == id).with_context(|| format!("unknown model {id}"))
}

pub fn models_dir() -> PathBuf {
    if let Ok(d) = std::env::var("DEMOLENS_MODELS_DIR") {
        return PathBuf::from(d);
    }
    demolens_core::preset::presets_dir().parent().map(|p| p.join("models")).unwrap_or_else(|| PathBuf::from("models"))
}

pub fn path_of(m: &ModelInfo) -> PathBuf {
    models_dir().join(m.file)
}

pub fn is_installed(m: &ModelInfo) -> bool {
    path_of(m).exists()
}

/// Path of an installed model, or a friendly error telling the user what to
/// download.
pub fn require(id: &str) -> Result<PathBuf> {
    let m = find(id)?;
    let p = path_of(&m);
    if !p.exists() {
        bail!("model '{id}' is not installed. Download it with: demolens models download {id} ({})", human(m.size));
    }
    Ok(p)
}

/// First installed model of a kind, preferring the order of the catalog.
pub fn first_installed(kind: Kind) -> Option<ModelInfo> {
    catalog().into_iter().find(|m| m.kind == kind && is_installed(m))
}

pub fn human(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.0} MB")
    }
}

#[derive(Debug, Serialize)]
pub struct ModelStatus {
    #[serde(flatten)]
    pub info: ModelInfo,
    pub installed: bool,
    pub size_human: String,
}

pub fn list() -> Vec<ModelStatus> {
    catalog()
        .into_iter()
        .map(|m| ModelStatus { installed: is_installed(&m), size_human: human(m.size), info: m })
        .collect()
}

/// Download a model. `progress(done, total)` is called as bytes arrive.
pub fn download(id: &str, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    let m = find(id)?;
    let dir = models_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = path_of(&m);
    if dest.exists() {
        return Ok(dest);
    }
    let tmp = dir.join(format!("{}.part", m.file));
    let resp = ureq::get(m.url).call().with_context(|| format!("download {}", m.url))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(m.size);
    let mut reader = resp.into_body().into_with_config().limit(u64::MAX).reader();
    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    f.flush()?;
    drop(f);
    if m.archive {
        let out = dir.join(format!("{}.extract", m.file));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out)?;
        // bsdtar ships with Windows 10+ and macOS and reads zip and tar.gz.
        let st = std::process::Command::new("tar").arg("-xf").arg(&tmp).arg("-C").arg(&out).status().context("tar not found")?;
        if !st.success() {
            bail!("could not extract {}", tmp.display());
        }
        std::fs::remove_file(&tmp)?;
        std::fs::rename(&out, &dest)?;
    } else {
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(dest)
}

pub fn delete(id: &str) -> Result<()> {
    let m = find(id)?;
    let p = path_of(&m);
    if p.is_dir() {
        std::fs::remove_dir_all(&p)?;
    } else if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

/// Find an executable named `name` anywhere under `root`.
pub fn find_binary(root: &Path, name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|f| f == exe.as_str()) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_consistent() {
        let c = catalog();
        let mut ids: Vec<_> = c.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), c.len());
        assert!(c.iter().all(|m| m.url.starts_with("https://") && m.size > 0));
        assert_eq!(human(77_704_715), "78 MB");
        assert_eq!(human(1_117_320_736), "1.1 GB");
    }

    #[test]
    fn missing_model_error_is_helpful() {
        std::env::set_var("DEMOLENS_MODELS_DIR", std::env::temp_dir().join("demolens-no-models"));
        let e = require("whisper-tiny.en").unwrap_err().to_string();
        assert!(e.contains("demolens models download whisper-tiny.en"));
    }
}
