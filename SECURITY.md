# Security policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.2.x | Yes |
| 0.1.x | No. Please upgrade: see the advisory in the [v0.2.0 release notes](docs/release-notes/v0.2.0.md). |

## Reporting a vulnerability

Please report privately through [GitHub security advisories](https://github.com/im45145v/DemoLens/security/advisories/new). Do not open a public issue. You can expect a reply within 7 days. Please include the version, OS and steps to reproduce.

## What Booya does to stay safe

- No telemetry, accounts or update checks.
- Strict Content Security Policy; the webview can only write files you picked.
- Key capture is opt-in and never stores typed characters.
- Model downloads are SHA-256 verified.
- Cloud providers and uploads are off by default, need per task consent, and keep keys in the OS keychain.

The full audit is in [docs/SECURITY_AUDIT.md](docs/SECURITY_AUDIT.md).
