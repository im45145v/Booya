# Booya product review

Two reviews from October 2026, written after actually using the app. I ran the desktop app on a 1600x1000 X11 display, opened it with no project, tried to record, opened the sample, ran every Local AI action, exported, and read the README top to bottom as a new user would. Screenshots of what I saw are in `docs/media/before-after/before-*.png`.

---

## Part 1: Product manager review

### Who this is for

| User | Job to be done | Today they use |
|------|----------------|----------------|
| Indie hacker / founder | "Ship a launch video for my landing page and X post tonight, looking as good as a Screen Studio video." | Screen Studio (Mac only, paid), Cap, Loom |
| Developer advocate | "Record a feature walkthrough, then get captions, chapters and a written guide out of it without editing for an hour." | Tella, Descript, Loom |
| Engineer filing or showing a bug / PR | "Record 30 seconds, make sure no API key or customer email is visible, drop it in the PR." | OBS, Peek, Kap, built in recorders |
| Linux and Windows users | "I want the Screen Studio look and I am not on a Mac." | OBS plus a video editor |

The sharpest job is the third and fourth row combined with the AI: **a demo that is safe to share and looks good, made on any OS, without uploading anything.** Nobody else does that.

### Onboarding and first run

What a new user sees (`before-first-run.png`):

- A big black rectangle with a **broken image icon** drawn over the text "Record or open a video to start."
- Every button is enabled. Clicking Transcribe, Split or Export does nothing and says nothing.
- The record bar asks for a "window id or x,y,w,h". Nobody knows an X11 window id.
- No sample project, no hint about what the app does, nothing explaining that AI models are optional downloads.

The first time I pressed Record (`before-record-error.png`) it failed with raw ffmpeg output: `Error opening input: No such process`. The cause was simply that there was no microphone. The whole recording was thrown away instead of recording without audio.

Verdict: a new user has a strong chance of quitting within 60 seconds.

### Workflow friction

1. **The AI is five buttons that must be pressed in the right order.** Auto cut of filler words silently needs Transcribe first. Getting the headline result (zoomed, cut, captioned, secrets blurred) takes five clicks and reading status lines.
2. **The inspector is one long flat scroll** of about 40 controls with equal weight. Export, the thing you do last and most, is at the very bottom, below the fold.
3. **The transport row mixes clip editing and zoom editing** (Split, Delete clip, Speed, Add zoom, Delete zoom, Zoom scale, Auto zoom) in one undifferentiated line.
4. **Timeline:** no time ruler, no zoom, no scrubbing while dragging, zoom segments can be resized but not moved, nothing is labelled, and no lane names.
5. **Manual blur requires typing pixel coordinates** (`x,y,w,h (source px)`).
6. **No keyboard shortcuts** for the basics: Space to play, S to split, Delete, arrow keys to step. Only Ctrl+Z/S/O exist and are undiscoverable.
7. **Export** has no sensible destination presets (YouTube, X, LinkedIn, Reels), and the progress bar is tiny at the bottom of the panel.
8. **Presets** only work after you design and save your own. There are no built in looks to start from.

### Missing must haves (compared with what users expect in 2026)

- A first-run screen with a one click sample.
- One click "make it good" button.
- Downloadable installers. Today the only way in is `cargo build`, which needs Rust, a C toolchain, CMake and clang. That alone loses most of the audience.
- Window and area picking without typing ids or coordinates.
- Webcam bubble (every competitor has it).
- Light theme and basic accessibility: visible focus rings, readable contrast, labelled controls.

### Prioritised backlog

Impact and effort are 1 (low) to 3 (high).

| # | Item | Impact | Effort | Notes |
|---|------|--------|--------|-------|
| 1 | First-run welcome with "Try the sample", Record and Open, and disabled controls until a project is loaded | 3 | 1 | Fixes the 60 second drop off |
| 2 | One click **Make it demo-ready**: auto zoom, transcribe, cut fillers and pauses, captions, blur secrets, with a clear per step report and a single undo | 3 | 1 | The headline feature, built from parts that exist |
| 3 | Recording falls back to no audio when the mic or system audio fails, with human error messages | 3 | 1 | Today a missing mic loses the recording |
| 4 | Release workflow: installers for Linux (.deb, AppImage), macOS (.dmg) and Windows (.msi / setup .exe) on every tag | 3 | 2 | Biggest distribution unlock |
| 5 | Built in style templates (for example Midnight, Sunset, Clean light, Ocean, Minimal) next to user presets | 2 | 1 | Good looking output on first export |
| 6 | Export presets for YouTube, X, LinkedIn, Reels/Shorts and GIF, plus a visible progress state and "Show in folder" | 2 | 1 | |
| 7 | Editor layout: toolbar, preview, timeline, inspector with tabs, Export always visible | 3 | 2 | Stage 3 |
| 8 | Timeline: ruler, zoom, scrubbing, move and resize zoom segments, labelled lanes | 2 | 2 | Stage 3 |
| 9 | Keyboard shortcuts plus a cheat sheet (press ?) | 2 | 1 | Stage 3 |
| 10 | Draw blur boxes on the preview with the mouse | 2 | 2 | |
| 11 | Visual window and area picker | 2 | 2 | |
| 12 | Webcam overlay | 2 | 3 | |
| 13 | Wayland capture through the PipeWire portal | 2 | 3 | Default session on Ubuntu and Fedora |
| 14 | Bundle ffmpeg with the installers | 2 | 2 | Removes the last install step |
| 15 | Signed and notarised macOS build, signed Windows build | 2 | 2 | Needs certificates and money |

---

## Part 2: Outside product strategy consultant

### The market

| Product | Model | Platforms | Strength | Weakness |
|---------|-------|-----------|----------|----------|
| Screen Studio | Paid, closed | macOS only | The look everyone copies: auto zoom, cursor smoothing | Mac only, no AI beyond basics, cloud sharing is extra |
| Screen Charm | Paid, closed | macOS | Cheaper Screen Studio | Mac only |
| Cap | Open source (AGPL), Rust + Tauri | macOS, Windows | Instant share links, Studio mode, strong brand and community | Cloud first, the free tier pushes you to their hosting, no Linux release |
| OpenScreen | Open source | Desktop | Free Screen Studio style editor | Smaller feature set, no AI |
| Loom | SaaS | All | Instant sharing, comments, analytics | Not about polish, upload required, privacy concerns |
| Tella | SaaS | Web | Beautiful layouts, AI captions | Browser and cloud only, subscription |

### Positioning

Cap already owns "open source Screen Studio" for Mac and Windows. Booya should not fight on that sentence. The defensible position is:

> **The private demo studio. Product demos with local models, on Linux, Windows and macOS. Nothing leaves your machine.**

Three things nobody else combines:

1. **Fully offline AI**: captions, filler and pause removal, secret blurring and written docs, with open models you download once. Tella and Loom need the cloud. Screen Studio and Cap do not do secret detection at all.
2. **Auto blur of API keys, emails and card numbers.** This is the hook. Every developer has leaked something in a screen recording or been scared of it. It is a one sentence pitch that makes people try the app.
3. **Linux as a first class citizen, plus a scriptable CLI.** Re-render a demo from a project file in CI, batch export every aspect ratio, generate docs from a recording. That is a developer tool, not just a video app.

### What would make people star and share it

- A top of README GIF that shows **an API key on screen getting blurred automatically** and filler words being cut. Lead with the hook, not the feature list.
- Download buttons. A GitHub Release with installers for each OS, linked from the first screen of the README.
- A "Why Booya" table against Screen Studio, Cap and Loom. People share comparisons.
- A Show HN / r/linux / r/selfhosted launch angled at "Screen Studio for Linux with offline AI".
- A tiny landing page (GitHub Pages) with the GIF, the three bullets above and the download links.
- Every exported video could carry an optional, off by default "Made with Booya" end card. Opt in only, or it costs trust.

### Distribution

- **Now:** GitHub Releases built by Actions on every `v*` tag, with installers for all three OSes.
- **Next:** Homebrew cask, winget manifest, Flathub (this matters for Linux discovery), AUR.
- **Later:** a GitHub Pages landing page and a short launch video made with Booya itself.

### Risks

1. **Wayland.** Ubuntu and Fedora default to Wayland. Without the PipeWire portal, "great on Linux" is not true for most new Linux users. This is the biggest product risk.
2. **Cap moves to Linux and adds local AI.** They have the team and the brand. Booya must win on privacy, Linux depth and the CLI before that happens.
3. **Unsigned builds.** macOS Gatekeeper and Windows SmartScreen warnings scare people off. Signing costs money and admin time.
4. **ffmpeg as an external dependency.** Every "ffmpeg not found" is a lost user. Bundle it.
5. **Model quality.** The 0.5B model writes plain text and Whisper drops filler words. Be honest in the UI so that AI output never feels like a broken promise.
6. **Scope creep.** Recorder, editor, AI suite and CLI is a lot for one maintainer. Say no to cloud sharing and comments; that is Loom's game.
7. **Licensing.** The OCR models are CC-BY-SA-4.0. Keep them as optional downloads, never bundled, and attribute them.

### Blunt summary

The engine is good and the AI is a real differentiator, but the app did not look or behave like something you would recommend. Fix the first 60 seconds, put the hook (auto blur plus one click demo-ready) on the first screen, ship installers, and pick the "private demo studio" position.

---

## What was built from this review

Built and checked in the app on a virtual X display (screenshots in `docs/media/before-after/`):

- Items 1, 2, 3, 5, 6, 7, 8 and 9: welcome screen with Try the sample, one click Make it demo-ready with a per step report and one undo, recording that falls back to video only when the mic fails, six built in templates, export destinations with progress and Show in folder, the new toolbar / preview / timeline / inspector layout with dark and light themes, the new timeline (ruler, labelled lanes, scrubbing, timeline zoom, move and resize zooms) and keyboard shortcuts with a cheat sheet on `?`.
- Item 4: `.github/workflows/release.yml` builds installers on Linux, macOS and Windows. A tag `v*` publishes them as a draft release.

Critical bug found while testing: in the old `ui/main.js` every Local AI button handler was nested by mistake inside the preset Save handler, so the AI buttons did nothing until you saved a preset. The rewritten UI fixes it.

Still open: items 10 to 15 (draw blur boxes on the preview, window and area picker, webcam, Wayland, bundled ffmpeg, signed builds). Also: dragging a zoom segment can overlap its neighbour.
