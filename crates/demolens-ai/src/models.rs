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
    /// Second place to fetch the same file from when `url` fails. The pinned
    /// SHA-256 applies to both, so a mirror cannot change the file.
    #[serde(skip)]
    pub mirror: &'static str,
    /// SHA-256 of the downloaded file (or archive). Downloads that do not
    /// match are deleted, so a compromised mirror cannot ship a different
    /// model or, worse, a different llama-server binary.
    #[serde(skip)]
    pub sha256: &'static str,
}

const LLAMA_TAG: &str = "b11913";

#[cfg(test)]
const WHISPER: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/";
macro_rules! whisper_url {
    ($f:literal) => {
        concat!("https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/", $f)
    };
}

macro_rules! llama_url {
    ($asset:literal) => {
        concat!("https://github.com/ggml-org/llama.cpp/releases/download/b11913/llama-b11913-bin-", $asset)
    };
}

fn llama_runtime() -> Option<ModelInfo> {
    let (url, size, sha256) = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (llama_url!("ubuntu-x64.tar.gz"), 22_474_748, "c670ad826f59478db859c9d4a0d931ab9eb653f22b7e0adebd70816c4092aab5")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (llama_url!("ubuntu-arm64.tar.gz"), 18_210_224, "bb4f3daf4633fcac5eff5ea055c3ca9df3f4cc074d1d848f1c5abcdf4e435d89")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        (llama_url!("macos-arm64.tar.gz"), 16_295_309, "d5b7b56c275624be373d2ab064b68dee2a3571e852657482cce75cbb5cb143b2")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        (llama_url!("macos-x64.tar.gz"), 12_035_739, "b19a9c08544114b02e8e3dd688ed83f6983339e56d85aa455dd8a06df51c7c30")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        (llama_url!("win-cpu-x64.zip"), 24_218_562, "55b31ccaf398e90457cfe28087297f5f3fdf1e3bfa7a28e1108f13d0af8e4621")
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
        mirror: "",
        sha256,
    })
}

pub fn catalog() -> Vec<ModelInfo> {
    let mut v = vec![
        ModelInfo { id: "whisper-tiny.en", kind: Kind::Speech, title: "Whisper tiny (English), fastest", size: 77_704_715, license: "MIT", url: whisper_url!("ggml-tiny.en.bin"), file: "ggml-tiny.en.bin", archive: false, mirror: "", sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f" },
        ModelInfo { id: "whisper-base.en", kind: Kind::Speech, title: "Whisper base (English), recommended", size: 147_964_211, license: "MIT", url: whisper_url!("ggml-base.en.bin"), file: "ggml-base.en.bin", archive: false, mirror: "", sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002" },
        ModelInfo { id: "whisper-small.en", kind: Kind::Speech, title: "Whisper small (English), better captions", size: 487_614_201, license: "MIT", url: whisper_url!("ggml-small.en.bin"), file: "ggml-small.en.bin", archive: false, mirror: "", sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d" },
        ModelInfo { id: "whisper-small", kind: Kind::Speech, title: "Whisper small (99 languages)", size: 487_601_967, license: "MIT", url: whisper_url!("ggml-small.bin"), file: "ggml-small.bin", archive: false, mirror: "", sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b" },
        ModelInfo { id: "whisper-large-v3-turbo-q5", kind: Kind::Speech, title: "Whisper large v3 turbo Q5 (99 languages), best captions", size: 574_041_195, license: "MIT", url: whisper_url!("ggml-large-v3-turbo-q5_0.bin"), file: "ggml-large-v3-turbo-q5_0.bin", archive: false, mirror: "", sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2" },
        ModelInfo { id: "ocr-detection", kind: Kind::Ocr, title: "ocrs text detection", size: 2_510_284, license: "CC-BY-SA-4.0", url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten", file: "text-detection.rten", archive: false, mirror: "https://github.com/deeplabua/deepocr/releases/download/models-v1/text-detection.rten", sha256: "f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca" },
        ModelInfo { id: "ocr-recognition", kind: Kind::Ocr, title: "ocrs text recognition", size: 9_716_568, license: "CC-BY-SA-4.0", url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten", file: "text-recognition.rten", archive: false, mirror: "https://github.com/deeplabua/deepocr/releases/download/models-v1/text-recognition.rten", sha256: "e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e" },
        ModelInfo { id: "qwen2.5-0.5b", kind: Kind::Llm, title: "Qwen2.5 0.5B Instruct Q4_K_M, tiny and quick", size: 491_400_032, license: "Apache-2.0", url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/9217f5db79a29953eb74d5343926648285ec7e67/qwen2.5-0.5b-instruct-q4_k_m.gguf", file: "qwen2.5-0.5b-instruct-q4_k_m.gguf", archive: false, mirror: "", sha256: "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db" },
        ModelInfo { id: "qwen2.5-1.5b", kind: Kind::Llm, title: "Qwen2.5 1.5B Instruct Q4_K_M, better writing", size: 1_117_320_736, license: "Apache-2.0", url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/qwen2.5-1.5b-instruct-q4_k_m.gguf", file: "qwen2.5-1.5b-instruct-q4_k_m.gguf", archive: false, mirror: "", sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e" },
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
    path_of(m).exists() && (!m.archive || verified_marker(m).exists())
}

/// Archives are extracted, so their hash cannot be checked after the fact.
/// A marker written after a verified download tells them apart from a
/// runtime installed by v0.1.x, which was never verified.
fn verified_marker(m: &ModelInfo) -> PathBuf {
    models_dir().join(format!("{}.verified", m.file))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(p: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Check an installed single-file model against its pinned hash. Files from
/// v0.1.x were never verified; this is how to check them.
pub fn verify(id: &str) -> Result<bool> {
    let m = find(id)?;
    let p = path_of(&m);
    if m.archive {
        return Ok(verified_marker(&m).exists());
    }
    Ok(sha256_file(&p)? == m.sha256)
}

/// Hard cap on bytes accepted for a download: a little over the known size,
/// so a hostile or broken server cannot fill the disk.
pub fn size_cap(m: &ModelInfo) -> u64 {
    m.size + m.size / 20 + 1_000_000
}

/// Path of an installed model, or a friendly error telling the user what to
/// download.
pub fn require(id: &str) -> Result<PathBuf> {
    let m = find(id)?;
    let p = path_of(&m);
    if !is_installed(&m) {
        if m.archive && p.exists() {
            bail!("the llama.cpp runtime was installed by an older Booya without a checksum check. Delete and download it again: demolens models delete {id} && demolens models download {id}");
        }
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

/// Download a model over HTTPS, enforcing the size cap and the pinned
/// SHA-256 before anything is moved into place or extracted.
/// `progress(done, total)` is called as bytes arrive.
pub fn download(id: &str, progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    let m = find(id)?;
    let mut progress = progress;
    match download_from(&m, m.url, &mut progress) {
        Ok(p) => Ok(p),
        Err(first) if !m.mirror.is_empty() => {
            // The primary host can go away or refuse downloads (the OCR
            // bucket answers 403 today). Try the pinned mirror.
            download_from(&m, m.mirror, &mut progress).map_err(|second| anyhow::anyhow!("{first:#}; mirror also failed: {second:#}"))
        }
        Err(e) => Err(e),
    }
}

fn download_from(m: &ModelInfo, url: &str, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let dir = models_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = path_of(m);
    if is_installed(m) {
        return Ok(dest);
    }
    if !url.starts_with("https://") && !cfg!(test) {
        bail!("refusing to download over plain HTTP: {url}");
    }
    let cap = size_cap(m);
    let tmp = dir.join(format!("{}.part", m.file));
    let resp = ureq::get(url).call().with_context(|| format!("download {url}"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(m.size);
    if total > cap {
        bail!("{} is {} bytes, more than the expected {}; not downloading", m.id, total, m.size);
    }
    let mut reader = resp.into_body().into_with_config().limit(cap).reader();
    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let res: Result<()> = (|| {
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n])?;
            h.update(&buf[..n]);
            done += n as u64;
            progress(done, total);
        }
        f.flush()?;
        Ok(())
    })();
    drop(f);
    let got = hex(&h.finalize());
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.context(format!("download of {} failed (limit {} bytes)", m.id, cap)));
    }
    if got != m.sha256 {
        let _ = std::fs::remove_file(&tmp);
        bail!("checksum mismatch for {}: expected {}, got {}. The file was deleted.", m.id, m.sha256, got);
    }
    if m.archive {
        let _ = std::fs::remove_dir_all(&dest);
        let out = dir.join(format!("{}.extract", m.file));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out)?;
        // bsdtar ships with Windows 10+ and macOS and reads zip and tar.gz.
        // Both bsdtar and GNU tar refuse absolute and ".." member paths.
        let st = demolens_core::proc::command("tar").arg("-xf").arg(&tmp).arg("-C").arg(&out).status().context("tar not found")?;
        if !st.success() {
            bail!("could not extract {}", tmp.display());
        }
        std::fs::remove_file(&tmp)?;
        std::fs::rename(&out, &dest)?;
        std::fs::write(verified_marker(m), &got)?;
    } else {
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(dest)
}

pub fn delete(id: &str) -> Result<()> {
    let m = find(id)?;
    let p = path_of(&m);
    let _ = std::fs::remove_file(verified_marker(&m));
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
        assert!(c.iter().all(|m| m.url.starts_with("https://") && m.size > 0 && m.sha256.len() == 64));
        // Hugging Face URLs must be pinned to a commit, not the moving "main".
        assert!(c.iter().all(|m| !m.url.contains("/resolve/main/")));
        assert!(whisper_url!("x").starts_with(WHISPER));
        assert_eq!(human(77_704_715), "78 MB");
        assert_eq!(human(1_117_320_736), "1.1 GB");
    }

    /// Serve `body` once on a local port, like a mirror would.
    fn serve(body: Vec<u8>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut req = [0u8; 4096];
                let _ = c.read(&mut req);
                let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = c.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}/m.bin")
    }

    fn fake(file: &'static str, body: &[u8], sha256: &'static str) -> ModelInfo {
        ModelInfo { id: "fake", kind: Kind::Ocr, title: "", size: body.len() as u64, license: "", url: "", file, archive: false, mirror: "", sha256 }
    }

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn download_verifies_hash_and_size() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("demolens-dl-{}", std::process::id()));
        std::env::set_var("DEMOLENS_MODELS_DIR", &dir);
        let body = b"hello model".to_vec();
        let real = {
            use sha2::{Digest, Sha256};
            hex(&Sha256::digest(&body))
        };
        let leaked: &'static str = Box::leak(real.into_boxed_str());
        let ok = fake("ok.bin", &body, leaked);
        let p = download_from(&ok, &serve(body.clone()), |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), body);

        let bad = fake("bad.bin", &body, "00");
        let e = download_from(&bad, &serve(body.clone()), |_, _| {}).unwrap_err().to_string();
        assert!(e.contains("checksum mismatch"), "{e}");
        assert!(!dir.join("bad.bin").exists() && !dir.join("bad.bin.part").exists());

        // A server sending far more than expected is cut off.
        let mut small = fake("big.bin", b"x", leaked);
        small.size = 1;
        let e = download_from(&small, &serve(vec![0u8; 3_000_000]), |_, _| {}).unwrap_err().to_string();
        assert!(e.contains("more than the expected"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_model_error_is_helpful() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DEMOLENS_MODELS_DIR", std::env::temp_dir().join("demolens-no-models"));
        let e = require("whisper-tiny.en").unwrap_err().to_string();
        assert!(e.contains("demolens models download whisper-tiny.en"));
    }
}
