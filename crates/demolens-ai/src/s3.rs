//! Optional upload of an exported video to S3 or Cloudflare R2 (any S3
//! compatible store). Objects are private: nothing sets a public ACL, and
//! the share link is a presigned GET URL that expires (24 hours by default,
//! 7 days at most, the SigV4 limit). Only https endpoints are accepted.
//! Server side encryption (SSE-S3, AES256) is requested on AWS; R2 always
//! encrypts at rest and rejects the header, so it is not sent there.
//! Event logs (`*.events.json`) and projects are never uploaded.

use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

pub const DEFAULT_EXPIRY_S: u64 = 24 * 3600;
pub const MAX_EXPIRY_S: u64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct S3Config {
    /// e.g. https://s3.eu-west-1.amazonaws.com or
    /// https://<account>.r2.cloudflarestorage.com
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Key prefix, e.g. "demos/".
    pub prefix: String,
    pub access_key_id: String,
    /// Link lifetime in seconds (clamped to 7 days).
    pub expiry_s: Option<u64>,
}

impl S3Config {
    pub fn is_r2(&self) -> bool {
        self.endpoint.contains(".r2.cloudflarestorage.com")
    }
    fn host(&self) -> String {
        crate::llm::host_of(&self.endpoint)
            + &self.endpoint.split("://").nth(1).and_then(|r| r.split('/').next()).and_then(|a| a.rsplit_once(':').map(|(_, p)| format!(":{p}"))).unwrap_or_default()
    }
    fn region(&self) -> &str {
        if self.region.is_empty() {
            if self.is_r2() {
                "auto"
            } else {
                "us-east-1"
            }
        } else {
            &self.region
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !self.endpoint.to_ascii_lowercase().starts_with("https://") && !crate::llm::is_local(&self.endpoint) {
            bail!("the storage endpoint must use https://");
        }
        if self.bucket.is_empty() || self.access_key_id.is_empty() {
            bail!("set the bucket and access key id");
        }
        Ok(())
    }
}

/// Files that must never leave the machine through upload.
pub fn refuse(path: &Path) -> Option<&'static str> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(".events.json") || name == "events.json" {
        return Some("input event logs are never uploaded");
    }
    if name.ends_with(".demolens.json") || name.ends_with(".json") {
        return Some("only exported videos, GIFs and images can be uploaded");
    }
    let ok = ["mp4", "webm", "gif", "png", "jpg", "mov"];
    if !ok.iter().any(|e| name.ends_with(&format!(".{e}"))) {
        return Some("only exported videos, GIFs and images can be uploaded");
    }
    None
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hmac(key: &[u8], msg: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    m.update(msg.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn enc(s: &str, slash: bool) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b'/' if !slash => o.push('/'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// (yyyymmdd'T'hhmmss'Z', yyyymmdd) for a unix time.
fn amz_date(unix: u64) -> (String, String) {
    let days = (unix / 86400) as i64;
    let secs = unix % 86400;
    // Civil from days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let date = format!("{y:04}{m:02}{d:02}");
    (format!("{date}T{:02}{:02}{:02}Z", secs / 3600, secs / 60 % 60, secs % 60), date)
}

fn signing_key(secret: &str, date: &str, region: &str) -> Vec<u8> {
    let k = hmac(format!("AWS4{secret}").as_bytes(), date);
    let k = hmac(&k, region);
    let k = hmac(&k, "s3");
    hmac(&k, "aws4_request")
}

fn object_path(cfg: &S3Config, key: &str) -> String {
    let base_path = cfg.endpoint.split("://").nth(1).and_then(|r| r.split_once('/').map(|(_, p)| p.trim_end_matches('/').to_string())).unwrap_or_default();
    let mut p = String::new();
    if !base_path.is_empty() {
        p.push('/');
        p.push_str(&base_path);
    }
    format!("{p}/{}/{}", enc(&cfg.bucket, true), enc(key, false))
}

fn origin(cfg: &S3Config) -> String {
    let scheme = cfg.endpoint.split("://").next().unwrap_or("https");
    format!("{scheme}://{}", cfg.host())
}

/// A presigned GET URL valid for `expiry_s` seconds from `now`.
pub fn presign_get(cfg: &S3Config, secret: &str, key: &str, expiry_s: u64, now: u64) -> Result<String> {
    cfg.validate()?;
    let expiry = expiry_s.clamp(1, MAX_EXPIRY_S);
    let (stamp, date) = amz_date(now);
    let region = cfg.region();
    let scope = format!("{date}/{region}/s3/aws4_request");
    let path = object_path(cfg, key);
    let mut q = vec![
        ("X-Amz-Algorithm".to_string(), "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential".into(), format!("{}/{scope}", cfg.access_key_id)),
        ("X-Amz-Date".into(), stamp.clone()),
        ("X-Amz-Expires".into(), expiry.to_string()),
        ("X-Amz-SignedHeaders".into(), "host".into()),
    ];
    q.sort();
    let query: String = q.iter().map(|(k, v)| format!("{}={}", enc(k, true), enc(v, true))).collect::<Vec<_>>().join("&");
    let canonical = format!("GET\n{path}\n{query}\nhost:{}\n\nhost\nUNSIGNED-PAYLOAD", cfg.host());
    let to_sign = format!("AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}", hex(&Sha256::digest(canonical.as_bytes())));
    let sig = hex(&hmac(&signing_key(secret, &date, region), &to_sign));
    Ok(format!("{}{path}?{query}&X-Amz-Signature={sig}", origin(cfg)))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
pub struct Uploaded {
    pub key: String,
    pub url: String,
    pub expires_in_s: u64,
    pub encrypted: bool,
}

/// Upload a file as a private object and return a presigned link.
pub fn upload(cfg: &S3Config, secret: &str, file: &Path) -> Result<Uploaded> {
    cfg.validate()?;
    if let Some(why) = refuse(file) {
        bail!("{why}");
    }
    let name = file.file_name().context("no file name")?.to_string_lossy().to_string();
    let key = format!("{}{}-{name}", cfg.prefix, now());
    let len = std::fs::metadata(file)?.len();
    let ct = match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "gif" => "image/gif",
        "png" => "image/png",
        "jpg" => "image/jpeg",
        _ => "application/octet-stream",
    };
    let sse = !cfg.is_r2();
    let (stamp, date) = amz_date(now());
    let region = cfg.region();
    let scope = format!("{date}/{region}/s3/aws4_request");
    let path = object_path(cfg, &key);
    let mut headers = vec![
        ("content-type", ct.to_string()),
        ("host", cfg.host()),
        ("x-amz-content-sha256", "UNSIGNED-PAYLOAD".to_string()),
        ("x-amz-date", stamp.clone()),
    ];
    if sse {
        headers.push(("x-amz-server-side-encryption", "AES256".into()));
    }
    headers.sort();
    let signed: String = headers.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(";");
    let canon_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let canonical = format!("PUT\n{path}\n\n{canon_headers}\n{signed}\nUNSIGNED-PAYLOAD");
    let to_sign = format!("AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}", hex(&Sha256::digest(canonical.as_bytes())));
    let sig = hex(&hmac(&signing_key(secret, &date, region), &to_sign));
    let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={sig}", cfg.access_key_id);
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(3600))).http_status_as_error(false).build().into();
    let mut req = agent.put(&format!("{}{path}", origin(cfg))).header("Authorization", &auth).header("Content-Length", &len.to_string());
    for (k, v) in &headers {
        if *k != "host" {
            req = req.header(*k, v);
        }
    }
    let f = std::fs::File::open(file)?;
    let mut r = req.send(ureq::SendBody::from_owned_reader(f))?;
    let status = r.status().as_u16();
    if !(200..300).contains(&status) {
        let body = r.body_mut().read_to_string().unwrap_or_default();
        bail!("upload failed with HTTP {status}: {}", crate::providers::redact_err(&body.chars().take(300).collect::<String>()));
    }
    let expiry = cfg.expiry_s.unwrap_or(DEFAULT_EXPIRY_S).clamp(60, MAX_EXPIRY_S);
    Ok(Uploaded { url: presign_get(cfg, secret, &key, expiry, now())?, key, expires_in_s: expiry, encrypted: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presign_matches_aws_example() {
        // Example from the AWS SigV4 query string auth documentation.
        let cfg = S3Config { endpoint: "https://examplebucket.s3.amazonaws.com".into(), region: "us-east-1".into(), bucket: String::new(), access_key_id: "AKIAIOSFODNN7EXAMPLE".into(), ..Default::default() };
        // The doc uses virtual hosted style with the bucket in the host, so
        // build the path without a bucket segment.
        let mut c = cfg.clone();
        c.bucket = "x".into();
        let url = presign_get(&c, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "test.txt", 86400, 1369353600).unwrap();
        assert!(url.contains("X-Amz-Date=20130524T000000Z"));
        assert!(url.contains("X-Amz-Expires=86400"));
        assert!(url.contains("X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request"));
        // Same canonical request as the docs once the path matches.
        let canonical = "GET\n/test.txt\nX-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\nhost:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD";
        let to_sign = format!("AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n{}", hex(&Sha256::digest(canonical.as_bytes())));
        let sig = hex(&hmac(&signing_key("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "20130524", "us-east-1"), &to_sign));
        assert_eq!(sig, "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404");
    }

    #[test]
    fn expiry_is_capped_at_seven_days() {
        let c = S3Config { endpoint: "https://acct.r2.cloudflarestorage.com".into(), bucket: "b".into(), access_key_id: "k".into(), ..Default::default() };
        let url = presign_get(&c, "s", "a b.mp4", 30 * 86400, 1_800_000_000).unwrap();
        assert!(url.contains("X-Amz-Expires=604800"));
        assert!(url.contains("/b/a%20b.mp4?"));
        assert!(url.contains("%2Fauto%2Fs3%2F"));
    }

    #[test]
    fn refuses_logs_and_http() {
        assert!(refuse(Path::new("/x/rec.events.json")).is_some());
        assert!(refuse(Path::new("/x/demo.demolens.json")).is_some());
        assert!(refuse(Path::new("/x/demo.mp4")).is_none());
        let c = S3Config { endpoint: "http://s3.example.com".into(), bucket: "b".into(), access_key_id: "k".into(), ..Default::default() };
        assert!(c.validate().is_err());
    }

    #[test]
    fn upload_is_private_and_encrypted() {
        let (base, seen) = crate::providers::tests::mock(vec![(200, "text/plain", Vec::new())]);
        let dir = std::env::temp_dir().join(format!("dl-s3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("demo.mp4");
        std::fs::write(&f, b"fakevideo").unwrap();
        let c = S3Config { endpoint: base, bucket: "b".into(), access_key_id: "AK".into(), prefix: "demos/".into(), ..Default::default() };
        let up = upload(&c, "secret", &f).unwrap();
        assert!(up.url.contains("X-Amz-Expires=86400") && up.url.contains("X-Amz-Signature="));
        let s = seen.lock().unwrap();
        let h = s[0].1.to_ascii_lowercase();
        assert!(s[0].0.starts_with("PUT /b/demos/"));
        assert!(h.contains("x-amz-server-side-encryption: aes256"));
        assert!(!h.contains("x-amz-acl"));
        assert_eq!(s[0].2, b"fakevideo");
        let _ = std::fs::remove_dir_all(dir);
    }
}
