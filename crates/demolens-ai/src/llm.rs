//! Local LLM access through an OpenAI compatible chat endpoint on
//! localhost. In order of preference:
//! 1. `DEMOLENS_LLM_URL` (any OpenAI compatible server, e.g. LM Studio)
//! 2. Ollama on 127.0.0.1:11434
//! 3. a llama.cpp server already running on 127.0.0.1:8080
//! 4. the managed llama.cpp runtime plus a GGUF model from the model manager,
//!    started on a free port for the duration of the job.
//! Nothing is ever sent off the machine.

use crate::models::{self, Kind};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Llm {
    pub base: String,
    pub model: String,
    pub label: String,
    child: Option<Child>,
}

impl Drop for Llm {
    fn drop(&mut self) {
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into()
}

fn get_json(url: &str, timeout_ms: u64) -> Option<Value> {
    let a: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_millis(timeout_ms))).build().into();
    a.get(url).call().ok()?.body_mut().read_json().ok()
}

fn is_local(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or(url);
    ["127.0.0.1", "localhost", "[::1]", "0.0.0.0"].iter().any(|h| rest.starts_with(h))
}

impl Llm {
    /// Find a local LLM. Returns Ok(None) when nothing is available so the
    /// caller can fall back to the rule based writer.
    pub fn detect() -> Result<Option<Llm>> {
        if let Ok(url) = std::env::var("DEMOLENS_LLM_URL") {
            if !is_local(&url) && std::env::var("DEMOLENS_ALLOW_REMOTE_LLM").is_err() {
                bail!("DEMOLENS_LLM_URL must point at this machine (set DEMOLENS_ALLOW_REMOTE_LLM=1 to override)");
            }
            let model = std::env::var("DEMOLENS_LLM_MODEL").unwrap_or_else(|_| "local".into());
            return Ok(Some(Llm { base: url.trim_end_matches('/').into(), model, label: "custom endpoint".into(), child: None }));
        }
        if let Some(tags) = get_json("http://127.0.0.1:11434/api/tags", 800) {
            if let Some(name) = tags["models"].as_array().and_then(|m| m.first()).and_then(|m| m["name"].as_str()) {
                let model = std::env::var("DEMOLENS_LLM_MODEL").unwrap_or_else(|_| name.to_string());
                return Ok(Some(Llm { base: "http://127.0.0.1:11434/v1".into(), label: format!("Ollama ({model})"), model, child: None }));
            }
        }
        if get_json("http://127.0.0.1:8080/v1/models", 800).is_some() {
            return Ok(Some(Llm { base: "http://127.0.0.1:8080/v1".into(), model: "local".into(), label: "llama.cpp server".into(), child: None }));
        }
        Self::start_managed()
    }

    fn start_managed() -> Result<Option<Llm>> {
        let Some(rt) = models::catalog().into_iter().find(|m| m.kind == Kind::Runtime && models::is_installed(m)) else { return Ok(None) };
        let Some(gguf) = models::first_installed(Kind::Llm) else { return Ok(None) };
        let server = models::find_binary(&models::path_of(&rt), "llama-server").context("llama-server not found in the runtime")?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let mut cmd = Command::new(&server);
        cmd.args(["-m"]).arg(models::path_of(&gguf)).args(["--host", "127.0.0.1", "--port", &port.to_string(), "-c", "4096", "--log-disable"]);
        if let Some(dir) = server.parent() {
            // Shared libraries ship next to the binary.
            let key = if cfg!(target_os = "macos") { "DYLD_LIBRARY_PATH" } else { "LD_LIBRARY_PATH" };
            cmd.env(key, dir);
        }
        let child = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().context("failed to start llama-server")?;
        let llm = Llm { base: format!("http://127.0.0.1:{port}/v1"), model: gguf.id.into(), label: format!("managed llama.cpp ({})", gguf.id), child: Some(child) };
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(120) {
            if let Some(h) = get_json(&format!("http://127.0.0.1:{port}/health"), 1000) {
                if h["status"] == "ok" {
                    return Ok(Some(llm));
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        bail!("llama-server did not become ready")
    }

    pub fn chat(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "temperature": 0.3,
            "max_tokens": max_tokens,
            "stream": false
        });
        let v: Value = agent()
            .post(&format!("{}/chat/completions", self.base))
            .send_json(&body)
            .context("local LLM request failed")?
            .body_mut()
            .read_json()?;
        let text = v["choices"][0]["message"]["content"].as_str().context("unexpected LLM response")?;
        Ok(clean(text))
    }
}

/// Strip thinking blocks, code fences and stray quotes from model output.
pub fn clean(s: &str) -> String {
    let mut s = s.to_string();
    while let (Some(a), Some(b)) = (s.find("<think>"), s.find("</think>")) {
        if b < a {
            break;
        }
        s.replace_range(a..b + "</think>".len(), "");
    }
    let s = s.trim().trim_start_matches("```markdown").trim_start_matches("```").trim_end_matches("```").trim();
    s.replace(" \u{2014} ", ", ").replace('\u{2014}', ", ").replace('\u{2013}', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleans_model_output() {
        assert_eq!(clean("<think>hmm</think>\n```markdown\nHi \u{2014} there\n```"), "Hi, there");
        assert!(is_local("http://127.0.0.1:11434/v1"));
        assert!(!is_local("https://api.example.com/v1"));
    }
}
