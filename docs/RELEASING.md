# Releasing Booya

Releases are built by `.github/workflows/release.yml` when a tag like `v0.8.0` is pushed. Only the maintainer tags. Everything before the tag happens in pull requests.

## 1. Get main ready

- [ ] CI is green on main, on Ubuntu, macOS and Windows.
- [ ] The `capture-sync` job passed, which means the sync probe stayed within the budgets in `crates/booya-core/src/syncprobe.rs`.
- [ ] Someone has recorded on a real Mac and a real Windows machine with the mic and camera on, and done the clap test (clap at the start, middle and end, check the sound, picture and captions line up).
- [ ] Anything that touches touch input was tried on a real touch screen, or the release notes say it wasn't.

## 2. Bump the version (in a pull request)

The version lives in two places and they must match:

| File | Field |
|---|---|
| `Cargo.toml` | `[workspace.package] version` (every crate inherits it) |
| `src-tauri/tauri.conf.json` | `"version"` (installer names and the updater compare against this) |

Then run `cargo check` so `Cargo.lock` picks up the new version, and commit the lock file too.

## 3. Write the release notes (same pull request)

Create `docs/release-notes/vX.Y.Z.md`. The release workflow uses this file as the GitHub Release body and as the updater's notes, so the tag name and the file name must match exactly. Look at the previous file for the shape:

- What's new, in plain sentences, from the user's side.
- Fixes, linked to issues.
- Anything people need to do (permissions to grant again on macOS, a setting that moved).
- What wasn't tested on real hardware.

House style: short sentences, no exclamation marks, no em dashes, and the name is always Booya.

Merge the pull request.

## 4. Tag

On an up to date main:

```sh
git switch main && git pull
git tag -s vX.Y.Z -m "Booya vX.Y.Z"
git push origin vX.Y.Z
```

`-s` signs the tag. Never move or delete a pushed tag. If something is wrong, fix it and release the next patch version.

## 5. Watch the build

Actions, then Release. Each OS builds the installers with the bundled ffmpeg and checks it:

- **Linux** (Ubuntu 22.04 for older glibc): `.AppImage`, `.deb`, `.rpm`.
- **macOS** (Apple Silicon): `.dmg` and the updater `.app.tar.gz`. The workflow checks the app signature, checks every helper is signed the same way, and makes sure a recording doesn't hang.
- **Windows**: the setup `.exe` and the `.msi`.
- The `booya` CLI for each OS.

Every OS also checks that the bundled ffmpeg runs. Then the publish job uploads everything to a GitHub Release with the notes from step 3.

## 6. Updater signatures and latest.json

The in-app updater only installs files signed with the updater key (see [UPDATES.md](UPDATES.md)).

- If the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are set, the workflow signs the update files and writes `latest.json` itself.
- If they aren't, the release still publishes, without signatures. On the machine that holds the key, run:

  ```sh
  GITHUB_TOKEN=... python3 scripts/sign-release.py vX.Y.Z
  ```

  It downloads the update files, signs them with the Tauri signer (needs node), and uploads the `.sig` files and `latest.json` to the release. The key path comes from `BOOYA_UPDATER_KEY` and the password file from `BOOYA_UPDATER_KEY_PASSWORD_FILE`. Neither is printed.

Check: `https://github.com/im45145v/Booya/releases/latest/download/latest.json` should show the new version, and an older install should offer the update on its next start.

## 7. After the release

- Install from the release page on each OS and open the app once. On macOS, expect to grant Screen Recording again, because each build is a new ad hoc identity.
- Close the issues the release fixed, with a link to the release.
- If something is badly broken, publish a patch release. Don't delete the release, because the updater may already have pointed people at it.

## Building installers locally

```sh
cargo install tauri-cli --version "^2"
bash scripts/fetch-ffmpeg.sh            # optional, to bundle ffmpeg like CI does
cd src-tauri && cargo tauri build
```

Installers land in `target/release/bundle/`. Local builds aren't signed for the updater.
