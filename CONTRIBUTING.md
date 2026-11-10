# Contributing to Booya

Thanks for helping. Booya is a small project, so a clear bug report is worth as much as a pull request.

## Reporting a bug

Open an [issue](https://github.com/im45145v/Booya/issues/new/choose) and pick **Bug report**. The form asks for:

- your OS and version, and the Booya version (Settings > About and updates)
- what you did, what you expected, and what happened
- for sound or sync problems, the `audio.log` from the recording's project folder (it has the `microphone clock:` line)
- for capture problems, the output of `booya doctor`

Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Building

You need Rust 1.88 or newer, plus ffmpeg and ffprobe on your `PATH` for development builds (release installers bundle their own copy). The per OS packages are listed under [Building](README.md#building) in the README.

```sh
cargo build --release                 # CLI: target/release/booya
cargo build --release -p booya-app    # desktop app
```

The editor UI in `ui/` is plain HTML, CSS and JS with no bundler, so you can change it and rebuild the app without Node.

## Before you open a pull request

Run what CI runs:

```sh
cargo test -p booya-core -p booya-ai
cargo clippy -p booya-core --all-targets -- -D warnings
cargo clippy --workspace --all-targets
for t in tests/js/*.test.js; do node "$t"; done
python3 scripts/test_make_latest_json.py
```

CI builds and tests on Ubuntu, macOS and Windows. One OS failing doesn't cancel the others, so look at each result. If a job fails while installing packages, it's usually a slow mirror. Re-run it before digging in.

If you change the website in `site/`, build it with `sh site/build.sh` and open `site/dist` with any static server. See [site/README.md](site/README.md).

## Pull requests

- Branch from `main`, keep each pull request to one change, and say in the description what you tested and on which OS.
- Capture, sync and touch code needs testing on real hardware. If you only ran it in CI, say so. That's fine, it just tells reviewers what still needs a real machine.
- Add or update a test when you fix a bug in `booya-core`, especially in `zoom.rs`, `events.rs` and the capture timing code.
- Screenshots or a short clip help a lot for UI changes.
- Nobody pushes to `main` directly. Releases are tagged by the maintainer after merging.

## Writing style

Docs, UI copy and release notes follow the brand voice in [brand/](brand/README.md):

- Short, plain sentences. Say what happened and what to do next.
- No exclamation marks, no emoji in the UI, and no em dashes.
- The product is always "Booya" with a capital B in text. The lowercase "booya" is only the logo.
- Colours and fonts come from the `--by-*` tokens. Signal orange is for the record dot, Stop, the click ripple and the focus frame, and orange text uses `--by-signal-text`.

## License

By contributing you agree that your work is released under the [MIT License](LICENSE).
