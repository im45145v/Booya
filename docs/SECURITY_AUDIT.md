# Booya security and compute audit

Scope: tags v0.1.0, v0.1.1, v0.1.2, v0.1.3 and `main` before v0.2.0. Method: code review of every crate, the Tauri shell and the UI, `cargo audit` against the RustSec database, and the privacy review in the project tracker (GDPR and India DPDP Act 2023). All findings below are fixed in v0.2.0 unless marked otherwise.

Severity: **High** means a realistic way for private data to leak or for an attacker to write files. **Medium** means a real weakness that needs another bug or user mistake. **Low** is hardening or accuracy.

## Summary

| # | Finding | Severity | Affected | Status |
|---|---------|----------|----------|--------|
| 1 | Typed characters stored in the event log | High | v0.1.0 to v0.1.3 (Linux, Windows) | Fixed |
| 2 | Keyboard hook kept running after a recording stopped | High | v0.1.0 to v0.1.3 (Linux, Windows) | Fixed |
| 3 | Webview could write any file the user can write | High | v0.1.0 to v0.1.3 | Fixed |
| 4 | No Content Security Policy | Medium | v0.1.0 to v0.1.3 | Fixed |
| 5 | "Local LLM" check matched by prefix (`localhost.evil.com` counted as local) | Medium | v0.1.0 to v0.1.3 | Fixed |
| 6 | Remote LLM allowed over plain http, with no prompt | Medium | v0.1.0 to v0.1.3 | Fixed |
| 7 | Downloaded models and the llama.cpp runtime not hash checked | Medium | v0.1.0 to v0.1.3 | Fixed |
| 8 | Auto blur missed Indian IDs, UPI, IBAN, IPs, bearer tokens and password fields | Medium | v0.1.0 to v0.1.3 | Fixed (still best effort) |
| 9 | No visible recording indicator or pause | Medium | v0.1.0 to v0.1.3 | Fixed |
| 10 | Recordings and event logs readable by other local users (0755 / 0644) | Low | v0.1.0 to v0.1.3 (Linux, macOS) | Fixed (0700 / 0600) |
| 11 | No retention or one click delete | Low | v0.1.0 to v0.1.3 | Fixed |
| 12 | Dialog plugin exposed to the webview | Low | v0.1.0 to v0.1.3 | Fixed |
| 13 | GitHub Actions not pinned, release token writable in build jobs | Low | v0.1.0 to v0.1.3 | Fixed |
| 14 | README said "no internet connection" although models download | Low | v0.1.0 to v0.1.3 | Fixed |
| 15 | macOS usage strings did not match what the app does | Low | v0.1.2, v0.1.3 | Fixed |
| 16 | Transitive crates flagged unmaintained or unsound by RustSec | Info | all | Tracked, no known vulnerability |

No finding sends data to the maintainer. There has never been telemetry.

## Details

### 1. Typed characters stored in the event log (High)
`rdev` key events were written to `recording.events.jsonl` with the key label, so everything typed during a recording, including passwords, ended up in a plain text file next to the video. **Fix:** key capture is now off by default and no hook is installed when it is off. When turned on, keys are classified before they leave the hook process: shortcuts with Ctrl, Alt (Option), Cmd or Win held (stored with their key, for example Cmd+W, Shift alone does not count) and named keys (Enter, Tab, arrows) keep their name, every printable key becomes the class `printable`. The log format is v2 and the overlay shows "Typing..." for printable keys. Old v1 logs still load, and their characters are masked on load.

### 2. Keyboard hook outlived the recording (High)
`rdev::listen` cannot be stopped, so the global hook stayed active in the app until it quit. **Fix:** the hook runs in a short lived child process (`booya-app --booya-keyhook`) that is killed on stop and on pause. On macOS there is no hook: when key logging is on, a listen-only event tap (or, as fallback, a poller) reads keys through CoreGraphics (Input Monitoring permission) and applies the same rule: a letter is stored only when it is part of a modifier shortcut.

### 3. Arbitrary file write from the webview (High)
`save_project`, `export_video`, `export_captions` and `write_text` accepted any path from JavaScript. Any script running in the webview could write or overwrite files anywhere the user can. **Fix:** file dialogs run in Rust; only paths the user picked (or the app's own recordings folder) are writable, and each command only accepts its expected extensions. `reveal` is limited the same way.

### 4. No CSP (Medium)
`"csp": null`. **Fix:** a strict CSP (`script-src 'self'`, `connect-src` IPC only, `object-src`, `frame-src`, `base-uri` and `form-action` set to `'none'`) and `freezePrototype`.

### 5 and 6. Remote LLM check (Medium)
`is_local` used `starts_with`, so `http://localhost.evil.com` passed, and `0.0.0.0` counted as local. **Fix:** the host is parsed and must be exactly `127.0.0.1`, `localhost` or `[::1]`. A remote `BOOYA_LLM_URL` needs `BOOYA_ALLOW_REMOTE_LLM=1`, must use https, and the app asks before sending.

### 7. Unverified downloads (Medium)
Models and the llama.cpp server binary were used as downloaded. **Fix:** SHA-256 pinned in the catalog, checked after download (the file is deleted on mismatch), `booya models verify` re-checks installed files.

### 8. Detector gaps (Medium)
**Fix:** new detectors for Aadhaar (Verhoeff checksum), PAN, UPI IDs, IBAN (mod 97), IPv4 addresses, bearer tokens and `password=` strings, and fields next to a "password" label. A review warning is shown before export: "Auto blur may miss text. Watch the export before sharing." OCR is still best effort.

### 9. Recording indicator (Medium)
**Fix:** an always on top REC badge with a timer and a Pause button, plus Ctrl/Cmd+Shift+P. Pausing stops input capture and cuts the paused span. The badge is excluded from the capture on macOS and Windows; on Linux (X11) it can appear in the video.

### 10 to 15 (Low)
Private folders (0700) and logs (0600); retention setting and delete buttons in Settings > Privacy; dialog plugin removed from capabilities; all actions pinned to commit SHAs, `contents: read` by default and `write` only for the publish job, `cargo audit` in CI; README wording fixed; camera string now explains the webcam overlay.

### 16. RustSec warnings (Info)
`cargo audit` reports no vulnerabilities. It warns about unmaintained `proc-macro-error`, `ttf-parser`, `zbus_names`, `zvariant`, `zvariant_derive` and an unsoundness in `glib::VariantStrIter` (RUSTSEC-2024-0429), all pulled in by the GTK and Tauri stack. Booya does not call the affected glib API. These clear when Tauri moves to newer GTK bindings.

There are no npm dependencies, so `npm audit` does not apply.

## Compute findings

| Finding | Affected | Fix |
|---------|----------|-----|
| Preview started a new ffmpeg per frame during playback | v0.1.0 to v0.1.3 | A persistent decoder streams frames while playing; scrubbing still seeks |
| Export always re-encoded, even with no edits | all | Untouched recordings are remuxed (stream copy) |
| Whisper ran over long silences | all | Energy based VAD; only speech is transcribed, timings mapped back |
| Re-transcribing the same audio | all | Cache keyed by SHA-256 of audio, model and prompt (0600, cleared by Delete all data) |
| OCR ran on every sampled frame | all | Frames that did not change (48x27 thumbnail diff) reuse the last result |

## New surface in v0.2.0
Cloud AI providers and S3/R2 upload are new and **off by default**. Keys live in the OS keychain and never reach the webview (only a masked form). Each provider is opted into per task, with the date recorded; every call shows a send preview (data, size, provider, endpoint, terms). Uploads are private objects with SSE where supported and presigned links (24 hours by default, 7 days at most); event logs and project files are refused. Error messages are scrubbed of keys. All of this is covered by mocked HTTP tests; it has not been tested against every live provider.
