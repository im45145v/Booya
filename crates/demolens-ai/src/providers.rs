//! Optional cloud AI providers. Local models stay the default: nothing in
//! this module runs unless the user picks a provider for a task, stores a
//! key for it and accepts a notice that the data leaves this computer. The
//! consent is recorded per provider and task with the date.
//!
//! API keys are never written here. The desktop app keeps them in the OS
//! keychain (keyring) and passes them in per call; the CLI reads
//! `DEMOLENS_<PROVIDER>_API_KEY` from the environment.

use crate::llm::{is_local, Llm};
use anyhow::{bail, Context, Result};
use demolens_core::project::CaptionWord;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gemini,
    Groq,
    OpenRouter,
    Deepgram,
    AssemblyAi,
    ElevenLabs,
    /// Any OpenAI compatible endpoint (base URL from the config).
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Task {
    /// Speech to text (captions).
    Stt,
    /// Text generation (guides, chapters).
    Llm,
    /// Text to speech (voiceover).
    Tts,
}

impl Task {
    pub fn id(self) -> &'static str {
        match self {
            Task::Stt => "stt",
            Task::Llm => "llm",
            Task::Tts => "tts",
        }
    }
    pub fn parse(s: &str) -> Result<Task> {
        Ok(match s {
            "stt" => Task::Stt,
            "llm" => Task::Llm,
            "tts" => Task::Tts,
            _ => bail!("unknown task {s} (stt, llm or tts)"),
        })
    }
}

/// Static facts about a provider, shown in settings and the send preview.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub id: Provider,
    pub name: &'static str,
    pub tasks: &'static [Task],
    pub base: &'static str,
    pub terms: &'static str,
    pub privacy: &'static str,
    pub key_url: &'static str,
    pub default_models: &'static [(Task, &'static str)],
}

pub const ALL: &[Provider] = &[
    Provider::OpenAi,
    Provider::Anthropic,
    Provider::Gemini,
    Provider::Groq,
    Provider::OpenRouter,
    Provider::Deepgram,
    Provider::AssemblyAi,
    Provider::ElevenLabs,
    Provider::Custom,
];

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::Groq => "groq",
            Provider::OpenRouter => "openrouter",
            Provider::Deepgram => "deepgram",
            Provider::AssemblyAi => "assemblyai",
            Provider::ElevenLabs => "elevenlabs",
            Provider::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Result<Provider> {
        ALL.iter().copied().find(|p| p.id() == s.to_ascii_lowercase()).with_context(|| format!("unknown provider {s}"))
    }

    pub fn info(self) -> Info {
        use Task::*;
        let (name, tasks, base, terms, privacy, key_url, default_models): (_, &'static [Task], _, _, _, _, &'static [(Task, &'static str)]) = match self {
            Provider::OpenAi => ("OpenAI", &[Stt, Llm, Tts], "https://api.openai.com/v1", "https://openai.com/policies/row-terms-of-use/", "https://openai.com/policies/row-privacy-policy/", "https://platform.openai.com/api-keys", &[(Stt, "whisper-1"), (Llm, "gpt-4o-mini"), (Tts, "gpt-4o-mini-tts")]),
            Provider::Anthropic => ("Anthropic", &[Llm], "https://api.anthropic.com/v1", "https://www.anthropic.com/legal/commercial-terms", "https://www.anthropic.com/legal/privacy", "https://console.anthropic.com/settings/keys", &[(Llm, "claude-3-5-haiku-latest")]),
            Provider::Gemini => ("Google Gemini", &[Llm], "https://generativelanguage.googleapis.com/v1beta/openai", "https://ai.google.dev/gemini-api/terms", "https://policies.google.com/privacy", "https://aistudio.google.com/apikey", &[(Llm, "gemini-2.0-flash")]),
            Provider::Groq => ("Groq", &[Stt, Llm], "https://api.groq.com/openai/v1", "https://groq.com/terms-of-use/", "https://groq.com/privacy-policy/", "https://console.groq.com/keys", &[(Stt, "whisper-large-v3-turbo"), (Llm, "llama-3.1-8b-instant")]),
            Provider::OpenRouter => ("OpenRouter", &[Llm], "https://openrouter.ai/api/v1", "https://openrouter.ai/terms", "https://openrouter.ai/privacy", "https://openrouter.ai/settings/keys", &[(Llm, "openai/gpt-4o-mini")]),
            Provider::Deepgram => ("Deepgram", &[Stt], "https://api.deepgram.com/v1", "https://deepgram.com/terms", "https://deepgram.com/privacy", "https://console.deepgram.com/", &[(Stt, "nova-2")]),
            Provider::AssemblyAi => ("AssemblyAI", &[Stt], "https://api.assemblyai.com/v2", "https://www.assemblyai.com/legal/terms-of-service", "https://www.assemblyai.com/legal/privacy-policy", "https://www.assemblyai.com/app/account", &[(Stt, "best")]),
            Provider::ElevenLabs => ("ElevenLabs", &[Stt, Tts], "https://api.elevenlabs.io/v1", "https://elevenlabs.io/terms-of-use", "https://elevenlabs.io/privacy-policy", "https://elevenlabs.io/app/settings/api-keys", &[(Stt, "scribe_v1"), (Tts, "eleven_multilingual_v2")]),
            Provider::Custom => ("OpenAI compatible endpoint", &[Stt, Llm, Tts], "", "", "", "", &[(Stt, "whisper-1"), (Llm, "local"), (Tts, "tts-1")]),
        };
        Info { id: self, name, tasks, base, terms, privacy, key_url, default_models }
    }

    pub fn supports(self, t: Task) -> bool {
        self.info().tasks.contains(&t)
    }

    pub fn env_key(self) -> String {
        format!("DEMOLENS_{}_API_KEY", self.id().to_ascii_uppercase())
    }
}

/// Settings stored in `~/.config/demolens/cloud.json` (0600). No keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudConfig {
    /// Provider chosen per task. Absent means local.
    pub choice: BTreeMap<String, Provider>,
    /// "provider:task" -> date the user accepted the data leaves notice.
    pub consents: BTreeMap<String, String>,
    /// "provider:task" -> model override.
    pub models: BTreeMap<String, String>,
    /// Base URL for the custom OpenAI compatible provider.
    pub custom_base: Option<String>,
    /// ElevenLabs or OpenAI voice for TTS.
    pub voice: Option<String>,
    /// Test only: override every provider's base URL.
    #[serde(skip)]
    pub base_override: Option<String>,
}

pub fn config_path() -> PathBuf {
    demolens_core::preset::presets_dir().parent().map(|p| p.join("cloud.json")).unwrap_or_else(|| "cloud.json".into())
}

impl CloudConfig {
    pub fn load() -> CloudConfig {
        std::fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        use std::io::Write;
        o.open(&p)?.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    fn key_of(p: Provider, t: Task) -> String {
        format!("{}:{}", p.id(), t.id())
    }

    pub fn consent_date(&self, p: Provider, t: Task) -> Option<&String> {
        self.consents.get(&Self::key_of(p, t))
    }

    /// Record that the user accepted sending `t` data to `p` on `date`.
    pub fn consent(&mut self, p: Provider, t: Task, date: &str) -> Result<()> {
        if !p.supports(t) {
            bail!("{} does not offer {}", p.info().name, t.id());
        }
        self.consents.insert(Self::key_of(p, t), date.to_string());
        self.choice.insert(t.id().into(), p);
        Ok(())
    }

    /// Withdraw consent and go back to local for that task.
    pub fn revoke(&mut self, p: Provider, t: Task) {
        self.consents.remove(&Self::key_of(p, t));
        if self.choice.get(t.id()) == Some(&p) {
            self.choice.remove(t.id());
        }
    }

    /// The provider to use for a task, only if it was opted into.
    pub fn active(&self, t: Task) -> Option<Provider> {
        self.choice.get(t.id()).copied().filter(|p| self.consent_date(*p, t).is_some())
    }

    pub fn model(&self, p: Provider, t: Task) -> String {
        self.models
            .get(&Self::key_of(p, t))
            .cloned()
            .or_else(|| p.info().default_models.iter().find(|(tt, _)| *tt == t).map(|(_, m)| m.to_string()))
            .unwrap_or_default()
    }

    pub fn base(&self, p: Provider) -> Result<String> {
        if let Some(b) = &self.base_override {
            return Ok(b.trim_end_matches('/').to_string());
        }
        let b = if p == Provider::Custom { self.custom_base.clone().context("set the base URL for the custom provider first")? } else { p.info().base.to_string() };
        if !is_local(&b) && !b.to_ascii_lowercase().starts_with("https://") {
            bail!("{b} must use https:// (only localhost may use http)");
        }
        Ok(b.trim_end_matches('/').to_string())
    }
}

/// What will be sent, shown to the user before every cloud call.
#[derive(Debug, Clone, Serialize)]
pub struct SendPreview {
    pub provider: String,
    pub task: Task,
    pub endpoint: String,
    pub data: String,
    pub bytes: u64,
    pub model: String,
    pub terms: String,
    pub privacy: String,
    pub consented_on: Option<String>,
}

pub fn preview(cfg: &CloudConfig, p: Provider, t: Task, data: &str, bytes: u64) -> Result<SendPreview> {
    let base = cfg.base(p)?;
    let path = endpoint_path(p, t, &cfg.model(p, t), cfg.voice.as_deref());
    let info = p.info();
    Ok(SendPreview {
        provider: info.name.into(),
        task: t,
        endpoint: format!("{base}{path}"),
        data: data.into(),
        bytes,
        model: cfg.model(p, t),
        terms: info.terms.into(),
        privacy: info.privacy.into(),
        consented_on: cfg.consent_date(p, t).cloned(),
    })
}

fn endpoint_path(p: Provider, t: Task, model: &str, voice: Option<&str>) -> String {
    match (p, t) {
        (Provider::Deepgram, Task::Stt) => format!("/listen?model={model}&smart_format=true&filler_words=true&punctuate=true"),
        (Provider::AssemblyAi, Task::Stt) => "/upload then /transcript".into(),
        (Provider::ElevenLabs, Task::Stt) => "/speech-to-text".into(),
        (Provider::ElevenLabs, Task::Tts) => format!("/text-to-speech/{}?output_format=mp3_44100_128", voice.unwrap_or(ELEVEN_VOICE)),
        (_, Task::Stt) => "/audio/transcriptions".into(),
        (_, Task::Llm) => "/chat/completions".into(),
        (_, Task::Tts) => "/audio/speech".into(),
    }
}

const ELEVEN_VOICE: &str = "JBFqnCBsd6RMkjVDRZzb";

fn require_consent(cfg: &CloudConfig, p: Provider, t: Task) -> Result<()> {
    if !p.supports(t) {
        bail!("{} does not offer {}", p.info().name, t.id());
    }
    if cfg.consent_date(p, t).is_none() {
        bail!("{} is not enabled for {}. Turn it on in Settings > Cloud AI first.", p.info().name, t.id());
    }
    Ok(())
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(secs))).http_status_as_error(false).build().into()
}

/// Remove anything that looks like a key from an error message.
pub fn redact_err(s: &str) -> String {
    let re = regex::Regex::new(r"(?i)(bearer\s+|token\s+|key[=:]\s*|sk-|xi-api-key:\s*)[A-Za-z0-9_\-\.]{6,}").unwrap();
    re.replace_all(s, "${1}[redacted]").to_string()
}

fn check(status: u16, body: &str, what: &str) -> Result<()> {
    if !(200..300).contains(&status) {
        let snippet: String = redact_err(body).chars().take(300).collect();
        bail!("{what} failed with HTTP {status}: {snippet}");
    }
    Ok(())
}

fn multipart(fields: &[(&str, &str)], file: &[u8], file_field: &str) -> (String, Vec<u8>) {
    let boundary = format!("demolens{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let mut b = Vec::new();
    for (k, v) in fields {
        b.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    b.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    b.extend_from_slice(file);
    b.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), b)
}

/// 16 kHz mono 16-bit WAV of `samples`.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend(b"RIFF");
    v.extend((36 + data_len).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * 2).to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend(data_len.to_le_bytes());
    for s in samples {
        v.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    v
}

fn word(text: &str, start_s: f64, end_s: f64) -> CaptionWord {
    CaptionWord { start_ms: start_s * 1000.0, end_ms: end_s * 1000.0, text: text.trim().to_string() }
}

/// Speech to text in the cloud. `wav` is 16 kHz mono WAV audio.
pub fn transcribe(cfg: &CloudConfig, p: Provider, key: &str, wav: &[u8], language: Option<&str>) -> Result<Vec<CaptionWord>> {
    require_consent(cfg, p, Task::Stt)?;
    let base = cfg.base(p)?;
    let model = cfg.model(p, Task::Stt);
    let a = agent(900);
    let words: Vec<CaptionWord> = match p {
        Provider::Deepgram => {
            let mut url = format!("{base}{}", endpoint_path(p, Task::Stt, &model, None));
            if let Some(l) = language {
                url.push_str(&format!("&language={l}"));
            }
            let mut r = a.post(&url).header("Authorization", &format!("Token {key}")).header("Content-Type", "audio/wav").send(wav)?;
            let body = r.body_mut().read_to_string()?;
            check(r.status().as_u16(), &body, "Deepgram")?;
            let v: Value = serde_json::from_str(&body)?;
            v["results"]["channels"][0]["alternatives"][0]["words"]
                .as_array()
                .map(|ws| ws.iter().map(|w| word(w["punctuated_word"].as_str().or(w["word"].as_str()).unwrap_or(""), w["start"].as_f64().unwrap_or(0.0), w["end"].as_f64().unwrap_or(0.0))).collect())
                .unwrap_or_default()
        }
        Provider::AssemblyAi => {
            let mut r = a.post(&format!("{base}/upload")).header("Authorization", key).header("Content-Type", "application/octet-stream").send(wav)?;
            let body = r.body_mut().read_to_string()?;
            check(r.status().as_u16(), &body, "AssemblyAI upload")?;
            let up: Value = serde_json::from_str(&body)?;
            let audio_url = up["upload_url"].as_str().context("no upload_url")?;
            let mut req = json!({ "audio_url": audio_url, "disfluencies": true, "punctuate": true });
            if model != "best" {
                req["speech_model"] = json!(model);
            }
            if let Some(l) = language {
                req["language_code"] = json!(l);
            }
            let mut r = a.post(&format!("{base}/transcript")).header("Authorization", key).send_json(&req)?;
            let body = r.body_mut().read_to_string()?;
            check(r.status().as_u16(), &body, "AssemblyAI transcript")?;
            let id = serde_json::from_str::<Value>(&body)?["id"].as_str().context("no transcript id")?.to_string();
            let t0 = std::time::Instant::now();
            let done = loop {
                let mut r = a.get(&format!("{base}/transcript/{id}")).header("Authorization", key).call()?;
                let body = r.body_mut().read_to_string()?;
                check(r.status().as_u16(), &body, "AssemblyAI poll")?;
                let v: Value = serde_json::from_str(&body)?;
                match v["status"].as_str() {
                    Some("completed") => break v,
                    Some("error") => bail!("AssemblyAI: {}", v["error"].as_str().unwrap_or("error")),
                    _ if t0.elapsed() > Duration::from_secs(900) => bail!("AssemblyAI took too long"),
                    _ => std::thread::sleep(Duration::from_millis(if cfg.base_override.is_some() { 10 } else { 2000 })),
                }
            };
            // Ask AssemblyAI to delete the transcript and audio now that we have it.
            let _ = a.delete(&format!("{base}/transcript/{id}")).header("Authorization", key).call();
            done["words"]
                .as_array()
                .map(|ws| ws.iter().map(|w| word(w["text"].as_str().unwrap_or(""), w["start"].as_f64().unwrap_or(0.0) / 1000.0, w["end"].as_f64().unwrap_or(0.0) / 1000.0)).collect())
                .unwrap_or_default()
        }
        Provider::ElevenLabs => {
            let mut fields = vec![("model_id", model.as_str()), ("tag_audio_events", "false")];
            if let Some(l) = language {
                fields.push(("language_code", l));
            }
            let (ct, body) = multipart(&fields, wav, "file");
            let mut r = a.post(&format!("{base}/speech-to-text")).header("xi-api-key", key).header("Content-Type", &ct).send(&body[..])?;
            let body = r.body_mut().read_to_string()?;
            check(r.status().as_u16(), &body, "ElevenLabs")?;
            let v: Value = serde_json::from_str(&body)?;
            v["words"]
                .as_array()
                .map(|ws| ws.iter().filter(|w| w["type"].as_str().unwrap_or("word") == "word").map(|w| word(w["text"].as_str().unwrap_or(""), w["start"].as_f64().unwrap_or(0.0), w["end"].as_f64().unwrap_or(0.0))).collect())
                .unwrap_or_default()
        }
        _ => {
            let mut fields = vec![("model", model.as_str()), ("response_format", "verbose_json"), ("timestamp_granularities[]", "word"), ("prompt", crate::transcribe::FILLER_PROMPT)];
            if let Some(l) = language {
                fields.push(("language", l));
            }
            let (ct, body) = multipart(&fields, wav, "file");
            let mut r = a.post(&format!("{base}/audio/transcriptions")).header("Authorization", &format!("Bearer {key}")).header("Content-Type", &ct).send(&body[..])?;
            let body = r.body_mut().read_to_string()?;
            check(r.status().as_u16(), &body, p.info().name)?;
            let v: Value = serde_json::from_str(&body)?;
            v["words"]
                .as_array()
                .map(|ws| ws.iter().map(|w| word(w["word"].as_str().unwrap_or(""), w["start"].as_f64().unwrap_or(0.0), w["end"].as_f64().unwrap_or(0.0))).collect())
                .unwrap_or_default()
        }
    };
    Ok(crate::transcribe::merge_fragments(words.into_iter().filter(|w| !w.text.is_empty()).collect()))
}

/// An LLM handle for an opted in cloud provider. All supported providers
/// expose an OpenAI compatible chat endpoint.
pub fn llm(cfg: &CloudConfig, p: Provider, key: &str) -> Result<Llm> {
    require_consent(cfg, p, Task::Llm)?;
    Llm::cloud(&cfg.base(p)?, &cfg.model(p, Task::Llm), key, &format!("{} (cloud)", p.info().name))
}

/// Text to speech in the cloud. Returns MP3 bytes.
pub fn speak(cfg: &CloudConfig, p: Provider, key: &str, text: &str) -> Result<Vec<u8>> {
    require_consent(cfg, p, Task::Tts)?;
    let base = cfg.base(p)?;
    let model = cfg.model(p, Task::Tts);
    let a = agent(300);
    let mut r = match p {
        Provider::ElevenLabs => a
            .post(&format!("{base}{}", endpoint_path(p, Task::Tts, &model, cfg.voice.as_deref())))
            .header("xi-api-key", key)
            .send_json(&json!({ "text": text, "model_id": model }))?,
        _ => a
            .post(&format!("{base}/audio/speech"))
            .header("Authorization", &format!("Bearer {key}"))
            .send_json(&json!({ "model": model, "input": text, "voice": cfg.voice.clone().unwrap_or_else(|| "alloy".into()), "response_format": "mp3" }))?,
    };
    let status = r.status().as_u16();
    let bytes = r.body_mut().with_config().limit(200 * 1024 * 1024).read_to_vec()?;
    if !(200..300).contains(&status) {
        check(status, &String::from_utf8_lossy(&bytes), p.info().name)?;
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A tiny HTTP server that answers each request with the next canned
    /// response and records (request line, headers, body).
    pub fn mock(responses: Vec<(u16, &'static str, Vec<u8>)>) -> (String, Arc<Mutex<Vec<(String, String, Vec<u8>)>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for (status, ct, body) in responses {
                let Ok((mut c, _)) = l.accept() else { return };
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut headers = String::new();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    headers.push_str(&h);
                }
                let mut b = vec![0; len];
                r.read_exact(&mut b).unwrap();
                s2.lock().unwrap().push((line.trim().to_string(), headers, b));
                let _ = write!(c, "HTTP/1.1 {status} OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = c.write_all(&body);
            }
        });
        (base, seen)
    }

    fn cfg(base: &str, p: Provider, t: Task) -> CloudConfig {
        let mut c = CloudConfig { base_override: Some(base.into()), ..Default::default() };
        c.consent(p, t, "2026-10-27").unwrap();
        c
    }

    #[test]
    fn nothing_runs_without_consent() {
        let c = CloudConfig::default();
        assert!(transcribe(&c, Provider::OpenAi, "k", b"x", None).unwrap_err().to_string().contains("not enabled"));
        assert!(c.active(Task::Stt).is_none());
        assert!(llm(&c, Provider::Anthropic, "k").is_err());
        let mut c = c;
        assert!(c.consent(Provider::Anthropic, Task::Stt, "2026-10-27").is_err());
        c.consent(Provider::Groq, Task::Llm, "2026-10-27").unwrap();
        assert_eq!(c.active(Task::Llm), Some(Provider::Groq));
        c.revoke(Provider::Groq, Task::Llm);
        assert_eq!(c.active(Task::Llm), None);
    }

    #[test]
    fn http_is_refused_for_remote_custom_endpoints() {
        let c = CloudConfig { custom_base: Some("http://example.com/v1".into()), ..Default::default() };
        assert!(c.base(Provider::Custom).is_err());
        let c = CloudConfig { custom_base: Some("http://127.0.0.1:1234/v1".into()), ..Default::default() };
        assert!(c.base(Provider::Custom).is_ok());
    }

    #[test]
    fn openai_compatible_stt() {
        let (base, seen) = mock(vec![(200, "application/json", br#"{"text":"hi there","words":[{"word":"Hi","start":0.5,"end":0.8},{"word":"there.","start":0.9,"end":1.2}]}"#.to_vec())]);
        let w = transcribe(&cfg(&base, Provider::Groq, Task::Stt), Provider::Groq, "gsk_secret", &wav_bytes(&[0.0; 160], 16000), Some("en")).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].start_ms, 500.0);
        let s = seen.lock().unwrap();
        assert!(s[0].0.starts_with("POST /audio/transcriptions"));
        assert!(s[0].1.contains("authorization: Bearer gsk_secret") || s[0].1.to_ascii_lowercase().contains("authorization: bearer gsk_secret"));
        assert!(String::from_utf8_lossy(&s[0].2).contains("whisper-large-v3-turbo"));
    }

    #[test]
    fn deepgram_stt() {
        let (base, seen) = mock(vec![(200, "application/json", br#"{"results":{"channels":[{"alternatives":[{"words":[{"word":"um","punctuated_word":"Um,","start":0.1,"end":0.3}]}]}]}}"#.to_vec())]);
        let w = transcribe(&cfg(&base, Provider::Deepgram, Task::Stt), Provider::Deepgram, "dg", b"RIFF", None).unwrap();
        assert_eq!(w[0].text, "Um,");
        let s = seen.lock().unwrap();
        assert!(s[0].0.contains("/listen?model=nova-2") && s[0].0.contains("filler_words=true"));
        assert!(s[0].1.to_ascii_lowercase().contains("authorization: token dg"));
    }

    #[test]
    fn assemblyai_stt_uploads_polls_and_deletes() {
        let (base, seen) = mock(vec![
            (200, "application/json", br#"{"upload_url":"https://cdn/x"}"#.to_vec()),
            (200, "application/json", br#"{"id":"t1","status":"queued"}"#.to_vec()),
            (200, "application/json", br#"{"id":"t1","status":"processing"}"#.to_vec()),
            (200, "application/json", br#"{"id":"t1","status":"completed","words":[{"text":"Hello","start":100,"end":400}]}"#.to_vec()),
            (200, "application/json", b"{}".to_vec()),
        ]);
        let w = transcribe(&cfg(&base, Provider::AssemblyAi, Task::Stt), Provider::AssemblyAi, "aai", b"RIFF", None).unwrap();
        assert_eq!((w[0].text.as_str(), w[0].start_ms, w[0].end_ms), ("Hello", 100.0, 400.0));
        std::thread::sleep(Duration::from_millis(100));
        let s = seen.lock().unwrap();
        assert!(s[1].0.starts_with("POST /transcript"));
        assert!(String::from_utf8_lossy(&s[1].2).replace([' ', '\n'], "").contains("\"disfluencies\":true"));
        assert!(s.last().unwrap().0.starts_with("DELETE /transcript/t1"));
    }

    #[test]
    fn elevenlabs_stt_and_tts() {
        let (base, seen) = mock(vec![
            (200, "application/json", br#"{"words":[{"text":"Hi","start":0.0,"end":0.2,"type":"word"},{"text":" ","start":0.2,"end":0.3,"type":"spacing"}]}"#.to_vec()),
            (200, "audio/mpeg", b"ID3fake".to_vec()),
        ]);
        let mut c = cfg(&base, Provider::ElevenLabs, Task::Stt);
        c.consent(Provider::ElevenLabs, Task::Tts, "2026-10-27").unwrap();
        let w = transcribe(&c, Provider::ElevenLabs, "xi", b"RIFF", None).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(speak(&c, Provider::ElevenLabs, "xi", "Hello").unwrap(), b"ID3fake");
        let s = seen.lock().unwrap();
        assert!(s[0].1.to_ascii_lowercase().contains("xi-api-key: xi"));
        assert!(s[1].0.contains("/text-to-speech/"));
    }

    #[test]
    fn cloud_llm_sends_bearer_and_parses() {
        let (base, seen) = mock(vec![(200, "application/json", br#"{"choices":[{"message":{"content":"A guide"}}]}"#.to_vec())]);
        let l = llm(&cfg(&base, Provider::Anthropic, Task::Llm), Provider::Anthropic, "sk-ant-x").unwrap();
        assert_eq!(l.chat("s", "u", 50).unwrap(), "A guide");
        let s = seen.lock().unwrap();
        assert!(s[0].0.starts_with("POST /chat/completions"));
        assert!(String::from_utf8_lossy(&s[0].2).contains("claude"));
    }

    #[test]
    fn errors_never_echo_keys() {
        let (base, _) = mock(vec![(401, "application/json", br#"{"error":"bad key sk-abcdef123456 Bearer abcdefghijkl"}"#.to_vec())]);
        let mut c = cfg(&base, Provider::OpenAi, Task::Tts);
        c.voice = None;
        let err = speak(&c, Provider::OpenAi, "sk-abcdef123456", "x").unwrap_err().to_string();
        assert!(err.contains("401") && !err.contains("abcdef123456") && !err.contains("abcdefghijkl"), "{err}");
    }

    #[test]
    fn preview_lists_endpoint_size_and_terms() {
        let mut c = CloudConfig::default();
        c.consent(Provider::OpenAi, Task::Stt, "2026-10-27").unwrap();
        let p = preview(&c, Provider::OpenAi, Task::Stt, "audio track (16 kHz mono WAV)", 1234).unwrap();
        assert_eq!(p.endpoint, "https://api.openai.com/v1/audio/transcriptions");
        assert!(p.terms.starts_with("https://") && p.privacy.starts_with("https://"));
        assert_eq!(p.consented_on.as_deref(), Some("2026-10-27"));
    }
}
