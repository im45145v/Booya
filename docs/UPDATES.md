# Updates

## How it works

DemoLens checks GitHub for a file called `latest.json` on the newest release (when it starts, if "Check for updates when DemoLens starts" is on, or when you press **Settings > About and updates > Check for updates**). It sends nothing but the request. If a newer version exists, a banner offers **Install and restart**.

The download is a signed update file. It is checked against a public key built into the app before it replaces anything. Without a valid signature it is refused.

- macOS: `.app.tar.gz`. Windows: the setup exe. Linux: the AppImage.
- A copy installed from a `.deb` or `.rpm` cannot replace itself. The banner opens the download page instead. Update through your package manager.
- If you have unsaved changes, DemoLens asks first. It will not update while recording.

## macOS permissions come back after an update

DemoLens is signed ad hoc, not with an Apple Developer ID, so macOS treats each new build as a new app. After an update it may ask for Screen Recording, Microphone and Input Monitoring again. Allow them again, then reopen DemoLens. This is in the update banner and in Settings. It goes away only with a paid Apple Developer ID, which has been left out on purpose for now.

## Signing keys (for whoever publishes releases)

- The public key is in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
- The private key signs every release's update files. It must never be committed.
- The release workflow signs when the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` exist. Without them it still builds and publishes, but without update signatures and without `latest.json`.
- Fallback: `scripts/sign-release.py vX.Y.Z` downloads the published update files, signs them with the key on the machine that holds it, and uploads `.sig` files and `latest.json` to the release.
- `scripts/make-latest-json.py` builds the manifest. `scripts/test_make_latest_json.py` tests it on CI.
- Lose the private key and existing installs can never update again; people would have to download a new build by hand. Keep a backup.
