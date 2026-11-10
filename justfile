# Booya tasks. Run `just` to see this list, `just <task>` to run one.
# Install just with `bash scripts/dev-setup.sh`, `brew install just`,
# `winget install Casey.Just` or `cargo install just --locked`.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# List the tasks
default:
    @just --list --unsorted

# Install or check what you need to build Booya
[unix]
setup *args:
    bash scripts/dev-setup.sh {{args}}

# Install or check what you need to build Booya
[windows]
setup *args:
    powershell -ExecutionPolicy Bypass -File scripts/dev-setup.ps1 {{args}}

# Run the desktop app (debug build)
app *args:
    cargo run -p booya-app -- {{args}}

# Run the desktop app with optimisations, closer to what users get
app-release *args:
    cargo run --release -p booya-app -- {{args}}

# Run the booya command line tool, for example: just cli doctor
cli *args:
    cargo run -q -p booya-cli -- {{args}}

# Make the built in sample recording and project in samples/
sample:
    cargo run -q -p booya-cli -- sample samples

# Check capture, permissions and ffmpeg on this machine
doctor:
    cargo run -q -p booya-cli -- doctor

# Type check everything, fastest feedback
check:
    cargo check --workspace --all-targets

# All tests: Rust, editor JS and the updater script
test: test-rust test-js test-py

# Rust unit tests for the core and AI crates
test-rust *args:
    cargo test -p booya-core -p booya-ai {{args}}

# Only the core library tests, the quickest Rust loop. Add a filter: just test-core zoom
test-core *filter:
    cargo test -p booya-core --lib {{filter}}

# Editor JS tests (Node)
test-js:
    node scripts/js-tests.mjs

# Python tests for the release scripts
test-py:
    python3 scripts/test_make_latest_json.py

# Clippy on the whole workspace, warnings are errors (same as CI)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format the Rust code
fmt:
    cargo fmt --all

# Measure sound against picture on files, as CI does
syncprobe:
    cargo run -q --release -p booya-cli -- syncprobe

# Time recording and export on this machine
bench seconds="8":
    cargo run -q --release -p booya-cli -- bench --seconds {{seconds}}

# What the CI build job runs, in the shared `ci` profile
ci:
    cargo test --profile ci -p booya-core -p booya-ai
    cargo build --profile ci --workspace
    cargo clippy --profile ci --workspace --all-targets -- -D warnings
    node scripts/js-tests.mjs
    python3 scripts/test_make_latest_json.py

# Build installers for this OS (needs: cargo install tauri-cli --version "^2")
bundle:
    cd src-tauri && cargo tauri build

# Download the static ffmpeg that release builds bundle
[unix]
fetch-ffmpeg:
    bash scripts/fetch-ffmpeg.sh

# Build the website into site/dist and serve it on http://localhost:8080
[unix]
site:
    sh site/build.sh
    python3 -m http.server 8080 --directory site/dist

# Delete build output
clean:
    cargo clean
