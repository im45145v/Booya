//! "Is each AI feature ready?" for the status panel, `demolens doctor` and the
//! first run checklist. Listing is cheap (file checks only); self tests run a
//! real, tiny job and are only done on request.

use crate::models::{self, Kind};
use crate::providers::Task;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Works now, on this machine.
    Ready,
    /// Needs a local model first (see `needs`).
    NeedsModel,
    /// Runs through a cloud provider the user opted into.
    Cloud,
    /// Only available through a cloud provider that is not set up.
    NeedsCloud,
    /// A model is installed but broken.
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct NeedModel {
    pub id: &'static str,
    pub title: &'static str,
    pub size: u64,
    pub size_human: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureStatus {
    pub id: &'static str,
    pub title: &'static str,
    pub state: State,
    /// One plain sentence for the user.
    pub detail: String,
    /// Models to download to make it work (empty when Ready).
    pub needs: Vec<NeedModel>,
    /// Models that would make it better but are not required.
    pub optional: Vec<NeedModel>,
    /// Name of the cloud provider when state is Cloud.
    pub provider: Option<String>,
    /// Whether a self test exists (needs a local model).
    pub can_test: bool,
}

fn need(id: &'static str) -> Option<NeedModel> {
    let m = models::find(id).ok()?;
    Some(NeedModel { id: m.id, title: m.title, size: m.size, size_human: models::human(m.size) })
}

/// The model ids "Download recommended models" fetches: captions, secret
/// finding. (The local writer model is optional and much bigger.)
pub const RECOMMENDED: &[&str] = &["whisper-base.en", "ocr-detection", "ocr-recognition"];

/// Recommended models that are not installed yet.
pub fn missing_recommended() -> Vec<NeedModel> {
    RECOMMENDED.iter().filter(|id| models::find(id).map(|m| !models::is_installed(&m)).unwrap_or(false)).filter_map(|id| need(id)).collect()
}

/// An installed single file model whose size is far off looks cut short.
fn damaged(m: &models::ModelInfo) -> bool {
    if m.archive {
        return false;
    }
    std::fs::metadata(models::path_of(m)).map(|f| f.len() + f.len() / 10 < m.size).unwrap_or(false)
}

/// `cloud` returns the provider name when a cloud provider is fully set up
/// (opted in, consented and key stored) for the task.
pub fn features(cloud: &dyn Fn(Task) -> Option<String>) -> Vec<FeatureStatus> {
    let mut v = Vec::new();

    // Captions
    let speech: Vec<_> = models::catalog().into_iter().filter(|m| m.kind == Kind::Speech && models::is_installed(m)).collect();
    let bad = speech.iter().find(|m| damaged(m));
    let mut f = FeatureStatus { id: "captions", title: "Captions (speech to text)", state: State::Ready, detail: String::new(), needs: vec![], optional: vec![], provider: None, can_test: false };
    if let Some(m) = bad {
        f.state = State::Error;
        f.detail = format!("{} looks incomplete. Delete it and download it again.", m.title);
        f.needs = need(m.id).into_iter().collect();
    } else if let Some(m) = crate::transcribe::default_model().and_then(|p| speech.iter().find(|m| models::path_of(m) == p).cloned()) {
        f.detail = format!("Local model: {}. Audio stays on this computer.", m.title);
        f.can_test = true;
    } else if let Some(p) = cloud(Task::Stt) {
        f.state = State::Cloud;
        f.provider = Some(p.clone());
        f.detail = format!("Audio is sent to {p} (you opted in). Download a model to keep it local.");
        f.optional = need("whisper-base.en").into_iter().collect();
    } else {
        f.state = State::NeedsModel;
        f.detail = "Needs a speech model. Without it, Transcribe cannot run.".into();
        f.needs = need("whisper-base.en").into_iter().collect();
    }
    v.push(f);

    // Secret finder
    let ocr_ids = ["ocr-detection", "ocr-recognition"];
    let missing: Vec<_> = ocr_ids.iter().filter(|id| models::find(id).map(|m| !models::is_installed(&m)).unwrap_or(true)).filter_map(|id| need(id)).collect();
    let mut f = FeatureStatus { id: "secrets", title: "Find secrets and blur them (OCR)", state: State::Ready, detail: String::new(), needs: vec![], optional: vec![], provider: None, can_test: false };
    if !missing.is_empty() {
        f.state = State::NeedsModel;
        f.detail = "Needs two small OCR models to read the screen. It always runs on this computer.".into();
        f.needs = missing;
    } else if ocr_ids.iter().filter_map(|id| models::find(id).ok()).any(|m| damaged(&m)) {
        f.state = State::Error;
        f.detail = "An OCR model file looks incomplete. Delete both and download them again.".into();
        f.needs = ocr_ids.iter().filter_map(|id| need(id)).collect();
    } else {
        f.detail = "OCR models installed. Scans run on this computer.".into();
        f.can_test = true;
    }
    v.push(f);

    // Auto cut
    let have_caps = v[0].state == State::Ready || v[0].state == State::Cloud;
    v.push(FeatureStatus {
        id: "autocut",
        title: "Cut silences and filler words",
        state: State::Ready,
        detail: if have_caps { "Silences and filler words (um, uh) can be cut.".into() } else { "Silence cuts work now. Filler word cuts also need captions (a speech model).".into() },
        needs: vec![],
        optional: if have_caps { vec![] } else { need("whisper-base.en").into_iter().collect() },
        provider: None,
        can_test: false,
    });

    // Docs
    let llm_local = models::first_installed(Kind::Llm).is_some() && models::catalog().iter().any(|m| m.kind == Kind::Runtime && models::is_installed(m));
    let mut f = FeatureStatus { id: "docs", title: "Guide, chapters and titles", state: State::Ready, detail: String::new(), needs: vec![], optional: vec![], provider: None, can_test: false };
    if llm_local {
        f.detail = "A local writer model is installed.".into();
        f.can_test = true;
    } else if let Some(p) = cloud(Task::Llm) {
        f.state = State::Cloud;
        f.provider = Some(p.clone());
        f.detail = format!("Text is sent to {p} (you opted in).");
    } else {
        f.detail = "Basic mode works now (steps from your clicks and captions). A local writer model makes the text better.".into();
        f.optional = ["qwen2.5-0.5b", "llama-runtime"].iter().filter(|id| models::find(id).map(|m| !models::is_installed(&m)).unwrap_or(false)).filter_map(|id| need(id)).collect();
    }
    v.push(f);

    // Voiceover
    let mut f = FeatureStatus { id: "voiceover", title: "AI voiceover (text to speech)", state: State::NeedsCloud, detail: String::new(), needs: vec![], optional: vec![], provider: None, can_test: false };
    if let Some(p) = cloud(Task::Tts) {
        f.state = State::Cloud;
        f.provider = Some(p.clone());
        f.detail = format!("Text is sent to {p} (you opted in).");
    } else {
        f.detail = "Cloud only. Add a provider in Settings > Cloud & API keys.".into();
    }
    v.push(f);
    v
}

#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub id: String,
    pub ok: bool,
    pub message: String,
}

/// Run the tiny self test for one feature that has a local model.
pub fn self_test(id: &str) -> TestResult {
    let r: anyhow::Result<String> = match id {
        "captions" => match crate::transcribe::default_model() {
            Some(m) => crate::transcribe::self_test(&m),
            None => Err(anyhow::anyhow!("no speech model installed")),
        },
        "secrets" => crate::redact::self_test(),
        "docs" => (|| {
            let llm = crate::llm::Llm::detect()?.ok_or_else(|| anyhow::anyhow!("no local writer model"))?;
            let t0 = std::time::Instant::now();
            let out = llm.chat("Reply with one word.", "Say OK.", 8)?;
            Ok(format!("{} answered in {:.1} s ({:?})", llm.label, t0.elapsed().as_secs_f64(), out.trim()))
        })(),
        other => Err(anyhow::anyhow!("{other} has no self test")),
    };
    match r {
        Ok(message) => TestResult { id: id.into(), ok: true, message },
        Err(e) => TestResult { id: id.into(), ok: false, message: format!("{e:#}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_models_features_say_what_to_download() {
        let dir = std::env::temp_dir().join(format!("dl-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The models folder is process global, so this only checks the shape.
        let f = features(&|_| None);
        assert_eq!(f.len(), 5);
        let caps = &f[0];
        if caps.state == State::NeedsModel {
            assert_eq!(caps.needs[0].id, "whisper-base.en");
            assert!(caps.needs[0].size > 0);
        }
        assert_eq!(f[4].state, State::NeedsCloud);
        let with_cloud = features(&|t| (t == Task::Tts).then(|| "ElevenLabs".to_string()));
        assert_eq!(with_cloud[4].state, State::Cloud);
        assert_eq!(with_cloud[4].provider.as_deref(), Some("ElevenLabs"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
