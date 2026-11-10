# How Booya is put together

This is the map for working on Booya's code. For what each feature does from a user's side, see the [README](../README.md). The measurements behind the sync work are in [SYNC.md](SYNC.md), and the auto zoom rules are in [EDITING.md](EDITING.md#auto-zoom-when-it-zooms).

## The big picture

```
            record                          edit                               export
 ┌─────────────────────────┐    ┌───────────────────────────────┐    ┌──────────────────────┐
 │ capture::start          │    │ My demo.booya/project.json    │    │ export::export       │
 │  screen  -> ffmpeg      │    │  clips, zooms, captions,      │    │  decode  (ffmpeg)    │
 │  mic     -> sidecar     │──▶ │  style, layouts, blur ...     │──▶ │  render  (Renderer)  │
 │  camera  -> sidecar     │    │                               │    │  encode  (ffmpeg)    │
 │  input   -> events.jsonl│    │ Renderer::render(src, t, cam) │    └──────────────────────┘
 │ RecordingSession::stop  │    │  -> PNG for the editor preview│
 │ finalize: align, mux    │    └───────────────────────────────┘
 └─────────────────────────┘
```

Three rules hold the design together:

1. **One render path.** The editor preview and the exporter both call `Renderer::render` (one `Renderer` per project) in `booya-core/src/render.rs`. The web UI never draws video pixels; it edits the project and shows frames that Rust rendered. If the preview looks right, the export is right.
2. **One clock per recording.** Screen, microphone, system audio, camera and the input log are separate captures, but each one records wall clock times against the same start. `finalize` lines them up at the end. Nothing assumes a fixed frame rate or that two processes started at the same instant.
3. **Everything is a file you can inspect.** A recording is the video, its sidecars and `recording.events.jsonl`. A project is a `*.booya` folder (see `bundle.rs`) holding `project.json`, the media, the event log and a plain text transcript. The CLI can do everything the editor does, so every stage can be scripted and tested without a window.

## Crates

| Crate | What it is | Depends on |
|---|---|---|
| `crates/booya-core` | The engine: capture, input log, auto zoom, compositor, export, project model, ffmpeg wrappers | ffmpeg on disk, no GUI |
| `crates/booya-ai` | Local models: whisper.cpp captions, OCR auto blur, the docs writer, optional cloud providers and S3 upload | `booya-core` |
| `crates/booya-cli` | The `booya` command line tool. Thin, it parses arguments and calls the two crates above | both |
| `src-tauri` (`booya-app`) | The desktop app. Tauri 2 commands over the core, tray, global shortcuts, windows, updater | all of the above |
| `ui/` | The editor front end. Plain HTML, CSS and JS, no bundler. Talks to Rust through Tauri `invoke` | `src-tauri` |

### booya-core, module by module

| Area | Modules | Notes |
|---|---|---|
| Capture | `capture/mod.rs`, `linux.rs`, `macos.rs`, `macos_sck.rs`, `windows.rs`, `winapi.rs` | `capture::start` builds the per OS screen capture (x11grab, ScreenCaptureKit piped to ffmpeg, gdigrab) and returns a `RecordingSession`. `stop` ends everything and hands back a `Recording` |
| Audio capture | `capture/mic.rs`, `mics.rs`, `wasapi.rs`, `sysaudio.rs`, `audiosink.rs` | The microphone always records as its own sidecar with a packet timing log (`recording.mic.timing`). Windows uses WASAPI in process. System audio is another sidecar |
| Picker | `capture/sources.rs`, `windowfit.rs` | Displays and windows for the visual picker, and where the editor window goes |
| Input | `input.rs`, `touch.rs`, `keycombo.rs`, `keylabels.rs`, `events.rs` | Mouse is polled on every OS. Keys come from OS hooks and only keep shortcuts and typing timing. Touch and pen come from the digitizer (Windows Raw Input, X11 XInput2) and are logged as clicks with `MouseButton::Touch` |
| After recording | `finalize.rs`, `splice.rs`, `storage.rs`, `fsutil.rs` | MKV to MP4 without re-encoding, microphone start and clock correction, system audio mix, retakes, cleaning up broken recordings, atomic file writes |
| Editing model | `project.rs`, `bundle.rs`, `preset.rs`, `shortcuts.rs` | `Project` is everything the editor changes, saved as `project.json` inside a `*.booya` folder |
| Auto zoom | `zoom.rs`, `activity.rs` | Event log to zoom segments, then the camera at any time (see below) |
| Drawing | `render.rs`, `frames.rs`, `text.rs` | The compositor. CPU, parallel over rows with rayon. Text uses an embedded font (DejaVu Sans Bold) so output matches on every OS |
| Media | `export.rs`, `ffmpeg.rs`, `proxy.rs`, `playback.rs`, `audio.rs`, `captions.rs` | Decode, render, encode. 720p editing proxies. Native sound playback for the editor. Noise removal, silence detection, caption lines and SRT/VTT |
| Tools | `sample.rs`, `syncprobe.rs`, `permissions.rs`, `proc.rs` | The built in sample, the sync measurement, what the OS allows, and the one place child processes are spawned (so Windows never flashes a console) |

## Recording, step by step

1. **Start.** `capture::start(&RecordOptions)` notes the start time, launches the screen grabber, opens the microphone (and system audio, camera) as separate recorders, and starts the input logger. On macOS, ScreenCaptureKit frames are piped into ffmpeg as raw video.
2. **During.** Each recorder writes its own file. The microphone writes a timing line per packet. The camera keeps each frame's wall clock time. The input logger writes `recording.events.jsonl` with times in milliseconds.
3. **Stop.** `RecordingSession::stop` stops the recorders in a safe order and collects their paths.
4. **Finish.** `finalize.rs` works out where the voice really started from the packet log (`mic_clock`), measures the microphone's real clock rate and corrects it when it is off by more than 0.05% (`clock_rate_filter`), lines the audio up with the first video frame (`audio_align_filter`), mixes in system audio (`merge_system_audio`), and remuxes to MP4 without re-encoding the video (`remux_to_mp4_info`).
5. **Check.** `booya verify recording.mp4` prints where each track starts (`sync_report`), and `booya syncprobe` measures sound against picture on a test signal. The CI `capture-sync` job runs it with `--capture` on Linux and fails when the budgets in `syncprobe.rs` are exceeded.

Why it's built this way, with numbers, is in [SYNC.md](SYNC.md).

## Auto zoom

`zoom.rs` has two halves:

- **Planning** (`generate_zoom_segments`) turns the event log into `ZoomSegment`s you can edit on the timeline. A segment starts for a cluster of clicks in a small area (3 within 4.5 s on Normal), a burst of typing, or a marker. Navigation (tab and window switches, the Dock, menus, drags) is skipped, scrolling ends a zoom, and nothing zooms in the first second. Touch taps count as clicks. The thresholds for Subtle, Normal and Lively are in the code and in [EDITING.md](EDITING.md#auto-zoom-when-it-zooms).
- **Evaluating** (`camera_at`, and `camera_at_mapped` when trims or speed changes move time) gives the camera's center and scale at any time: eased in and out on a spring curve that never overshoots, panning to the cursor inside a dead zone (the Steadiness setting), and clamped by `clamp_center` so the view never leaves the frame.

Both halves are pure functions of the event log and the settings, so they're tested without any capture. When a zoom feels wrong, the fix is almost always a new test case built from the event log that caused it.

## The editor

- `src-tauri/src/main.rs` registers the Tauri commands. Most are thin: load or save a project, render a preview frame at a time, run export, start or stop recording.
- `ui/main.js` is the app shell. Feature logic that can be tested without a window lives in small modules (`snap.js`, `shortcuts.js`, `captions-edit.js`, `timeline.js`, `avsync.js`) with tests in `tests/js/`. Other windows have their own files: `picker.js`, `indicator.js`, `permissions.js`, `projects.js`, `updater.js`.
- The preview asks Rust for each frame through the `preview` command in `src-tauri/src/main.rs`. It decodes the 720p proxy with `PreviewDecoder`, renders with the same `Renderer` and returns a PNG. Sound plays through the native player in `playback.rs`.
- Undo and redo work on project snapshots kept in `ui/main.js` (`checkpoint`, `undo`, `redo`).

## Export

`export::export` decodes the source with ffmpeg, renders each output frame with `Renderer::render`, and pipes raw frames to an ffmpeg encoder. The encoder list is in `ffmpeg.rs`: H.264 through VideoToolbox, NVENC and others where they're available, falling back to libx264 if a hardware encoder fails, and VP9 for WebM. Audio is cut and retimed to match trims, cuts and speed changes.

## Testing

| What | Where | Run |
|---|---|---|
| Core and AI unit and property tests | `#[cfg(test)]` in each module | `just test-rust` or `cargo test -p booya-core -p booya-ai` |
| Editor logic | `tests/js/*.test.js` | `just test-js` |
| Updater manifest | `scripts/test_make_latest_json.py` | `just test-py` |
| Real capture on Linux | CI job `capture-sync` | CI only |
| Captions on real speech | CI job `subtitles`, `scripts/subtitles_e2e.py` | CI only |
| The editor in a browser against a stand in backend (`tests/e2e/shim.js`) | `tests/e2e/`, CI job `editor-e2e` | CI only |
| Sound against picture | `booya syncprobe` | `just syncprobe` |

What CI can't test: real capture on macOS and Windows, microphones, cameras and touch screens. Changes there need a run on a real machine, and the PR should say which ones were tried.

## Where to start

- A zoom feels wrong: `zoom.rs`, and add a case to its tests with the event log that caused it.
- Sound or camera out of step: `finalize.rs` and `capture/mic.rs`, then `booya verify` on the recording. Read [SYNC.md](SYNC.md) first.
- Something looks wrong in the export but not the preview, or the other way round: it shouldn't happen, since both use `render.rs`. Check the proxy (`proxy.rs`) and the frame time passed in.
- A new editor control: the field goes in `project.rs` (with a serde default so old projects still load), drawing goes in `render.rs`, the control goes in `ui/`.
