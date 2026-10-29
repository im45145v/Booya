# Booya compared with other tools

Researched on 28 October 2026 from each vendor's own pricing and feature pages (links at the end). Prices change; check the vendor before you decide. This is a feature comparison, not a claim that Booya is better at everything: where a tool is ahead, the table says so.

## The short version

Booya is a **free, open source (MIT), local-first** recorder and editor for product demos on macOS, Windows and Linux. Nothing is uploaded unless you turn on a cloud provider or your own S3 bucket, and you are shown what would be sent first. The tools below are mostly paid subscriptions, and the ones with AI features run them in the vendor's cloud.

## Comparison

| | Booya | Screen Studio | Cap | Tella | Loom | Arcade | Descript | Supademo | FocuSee |
|---|---|---|---|---|---|---|---|---|---|
| **Price** | Free, MIT | $29/mo or $9/mo yearly | Free personal; $29/yr or $58 once for commercial; Pro $12/user/mo | $13 to $19/user/mo yearly, 7 day trial | Free (25 videos, 5 min); $15 to $20/creator/mo yearly | Free (1 demo); $42.50/seat/mo | Creator $24/mo yearly | Free; $38/mo yearly and up | $49.99/yr; $199.99 lifetime |
| **Platforms** | macOS, Windows, Linux | macOS only | macOS, Windows | macOS, Windows, web | macOS, Windows, web | Chrome, desktop | macOS, Windows | Chrome, desktop | macOS, Windows |
| **Open source** | Yes | No | Yes (commercial licence for work use) | No | No | No | No | No | No |
| **Works offline** | Yes, everything | Mostly | Studio mode | No | No | No | Partly | No | Mostly |
| **Auto zoom on clicks** | Yes (speed, level, steadiness dead zone, anticipation, merge; subtle, moderate and intense styles) | Yes (the benchmark) | Yes | Yes (100 to 350%, 4 styles) | No | Pan and zoom per step | No | No | Yes |
| **Cursor smoothing and click effects** | Yes | Yes | Yes | Yes | Click emphasis | n/a | No | n/a | Yes |
| **Backgrounds, padding, shadow, rounded corners** | Yes, templates | Yes | Yes | Yes | Frames | n/a | Limited | n/a | Yes |
| **Captions** | Local Whisper, free | Local Whisper or cloud | Pro | Yes (106 languages) | Yes | No | Yes | No | Credits |
| **Edit by transcript, cut fillers and pauses** | Cut fillers and pauses (local); transcript editing planned | Yes (v4) | Planned | Yes | Business + AI only | No | Yes (core feature) | No | Credits |
| **Auto blur of secrets** | Yes, local OCR (emails, keys, cards, Aadhaar, PAN, UPI, passwords) | Manual masks | No | No | Manual blur | Manual blur | No | Manual | Manual |
| **Noise reduction and loudness** | Local RNNoise and -16 LUFS | Local | Planned | Yes | Yes | n/a | Yes | n/a | Credits |
| **Webcam bubble and layouts** | Four layouts (screen only, bubble, side by side, camera full screen) that you can switch for any stretch of the recording, with a blend between them; mirror; a cursor following crop for 9:16 and 1:1; no cutout or face tracking yet | Yes, with cutout and split screen | Yes | Yes (many) | Yes | n/a | Yes | n/a | Yes |
| **iPhone or iPad recording** | Not yet | Yes (USB) | No | Mobile | Mobile app | Figma, media | No | Mobile | Mobile |
| **Instant share link** | Your own S3 or R2 bucket, signed link; no Booya server | Yes (30 min cap) | Yes, or self-host | Yes | Yes | Yes | Yes | Yes | No |
| **Interactive click-through demo** | Yes: one self-contained HTML file from the event log, blur boxes applied, no network calls, shareable by signed link | No | No | No | No | Yes (core) | No | Yes (core) | No |
| **Export** | MP4, WebM, GIF, SRT, VTT | MP4 to 4K 60, GIF | MP4, WebM, MOV, GIF | MP4 to 4K 60 | MP4 | MP4, GIF (paid) | MP4 | MP4, GIF (paid) | MP4 to 4K 60 |
| **Intro and outro cards, callouts** | Yes: title cards, timed text callouts | Yes | Basic | Yes | Basic | Yes | Yes | Yes | Yes |
| **Watermark** | Optional text, off by default; never forced | No (export needs a plan) | No | No | Free plan | Free plan | Free plan | Free plan | Free plan |
| **Team features and analytics** | No | Sharing | Pro | Yes | Yes | Yes | Yes | Yes | No |

Where the others are clearly ahead today: hosted sharing with viewer analytics and comments (Loom, Tella, Cap Pro), device mockups and iPhone capture (Screen Studio), camera cutout and face tracking (Screen Studio 4, Tella), full transcript-based video editing (Descript, Tella, Screen Studio 4), and branching demos with CRM hooks (Arcade, Supademo). Booya has no team workspace and no hosted viewer.

## What Booya does better

1. **Free and open, with nothing held back.** No plan, no watermark, no export limit, no seat count. Screen Studio will not export without a subscription; Loom's free plan stops at 25 videos of 5 minutes.
2. **Local-first by default.** Captions (Whisper), secret detection (OCR), noise removal and doc writing all run on your computer. Screen Studio is the only other tool here that does much on-device, and it is Mac only. Cloud providers in Booya are off until you choose a provider, per task, and each request shows the endpoint and data first.
3. **Automatic secret blurring.** None of the others find emails, API keys, card numbers and Indian IDs (Aadhaar, PAN, UPI) for you; they give you a blur brush.
4. **Cross-platform.** One tool on macOS, Windows and Linux. Screen Studio is Mac only; Cap has no Linux app.
5. **Your storage.** Share through your own S3 or Cloudflare R2 bucket with a signed, expiring link. There is no vendor server in the middle.
6. **Scriptable.** Everything the editor does is in the `booya` command line tool (`record`, `ready`, `export`, `verify`, `bench`), so demos can be rebuilt in CI when the product changes. No other tool here does that.
7. **Honest diagnostics.** `booya doctor` and `booya verify` tell you what is wrong with permissions, audio level and sync instead of leaving you guessing.
8. **Audio that is audible.** Two-pass loudness to -16 LUFS on export and in the editor, a mic level meter before you record, and a quiet-mic warning.

## Where 0.4.0 stands against that list

| Idea | Status |
|---|---|
| Interactive HTML demo from the event log | Done: `booya interactive` and the Auto edit tab, Export tab share |
| Auto chapters and titles, local | Done earlier through the docs pipeline (rule based, optional local LLM); no separate chapter UI yet |
| Webcam layouts | Done in 0.6.0: screen only, bubble, side by side and camera full screen, switched per stretch on the timeline with a blend. Windows webcam recording (DirectShow) was added in the same release |
| Instant share | Signed link to your own S3 or R2 for videos and interactive demos; no hosted viewer |
| Zoom polish | Dead zone camera that holds still and follows only at the edge, anticipates the action, steadiness slider, three zoom styles; no loupe or tilt |
| Intro and outro, callouts, watermark, background photos | Done; 0.3.1 adds arrows, boxes, ellipses and highlights, and twelve wallpapers |
| Visual window and area picker | Done in 0.3.1: live previews, app icons, search, drag to select with aspect presets. 0.3.2 adds hover to snap (highlight a window, click to take its frame) and snap to edges while dragging. macOS parts untested on hardware |
| macOS key track | Written (Input Monitoring poller), not yet run on a real Mac |
| Glass look | Done in 0.4.0: a frosted frame for the export (blur, tint, edge light, radius) and the same look for captions, notes and keys; an optional glass look for the app, with macOS system vibrancy. The macOS vibrancy part has not been run on a Mac |
| Rebindable recording shortcuts | Done in 0.4.0: Settings > Shortcuts, with clash checks. Registration with the OS has been compiled, not run on all three systems |
| Markers while recording | Done in 0.4.0: a shortcut or the pill button adds a zoom to the cursor at that moment |
| Multi-clip projects | Not done |
| Script cleanup and voice retakes | Not done (needs a local TTS model) |
| iPhone capture over USB | Not done |
| Transcript editing | Basic in 0.3.1: edit caption text, find and replace, cut a line from the video; word level cuts from the text are not done |
| Notarised builds, hosted viewer | Not done |

## ScreenCharm (Mac only, $99 once) and the gaps

ScreenCharm (screencharm.com, read on 28 October 2026) is a Mac app sold for a one time price of $99 (regular $149) that does auto zoom, camera layouts, a cursor following vertical video and captions. It is the closest match to what Booya set out to do, so this table checks each of its listed features against the code in Booya 0.6.0.

| ScreenCharm feature | Booya 0.7.0 |
|---|---|
| Smart auto zoom, editable per zoom | Done. Scored and selective, three styles, every zoom is a segment you can drag |
| Smooth cursor, size and speed | Done (size and smoothing) |
| Click highlights | Done (ripples, colour) |
| Camera bubble you can move and resize | Done (corner, size, circle or square, mirror) |
| Camera pinned to the left, right, top or bottom edge | Partly. The bubble goes in a corner, and "side by side" puts the camera in a column on the left or right. A bubble pinned to the middle of an edge is not done |
| Vertical (Reels, Shorts, TikTok) split video with the camera on one side and the screen following the cursor | Done in 0.6.0. In 9:16 and 1:1 the screen shows a window that follows the cursor and blends towards the zoom, and the camera sits above or below |
| Change layout during the video | Done in 0.6.0: a Layout lane on the timeline, with a blend between layouts. ScreenCharm does not list this |
| Captions with a choice of style | Done (local Whisper, burned in or as a file). Fewer caption styles than ScreenCharm |
| Backgrounds: gradient, photo, solid | Done, with twelve wallpapers and a glass frame |
| Device frames (MacBook) | Done in 0.7.0: a vector laptop, browser window and phone frame (simple shapes, no maker artwork) |
| Motion blur on fast cursor moves and zooms | Zoom and pan blur done in 0.7.0 (a toggle). Blur on the cursor itself is not done |
| Background music | Done in 0.7.0: import a file, volume, ducking under the voice, fades, start point and loop |
| Mute single clips | Done in 0.7.0 (and speed per clip) |
| Blur regions | Done, and Booya also finds emails, keys and cards for you |
| Mic enhancement | Done (RNNoise and loudness) |
| Canvas sizes for YouTube, LinkedIn, Shorts, Reels, TikTok | Done |
| Trim, cut, split, per clip speed | Done |
| Undo and redo, save and resume | Done |
| Export up to 4K | Done (4K in the export quality list) |
| Share link, no account for the viewer | Partly. A signed link to your own S3 or R2 bucket; no hosted viewer |
| System audio | Linux, and new in 0.7.0 Windows (WASAPI loopback) and macOS (ScreenCaptureKit audio). Both are compiled in CI and have not been run on hardware |
| Record the camera and mic with the screen | Done on macOS, Linux and, new in 0.6.0, Windows (compiled in CI, not run on a real camera) |
| Price | Free, MIT. ScreenCharm is paid |
| Platforms | macOS, Windows, Linux. ScreenCharm is Mac only |

The gaps worth closing next are blur on the cursor, a hosted viewer for share links, and testing the Windows and macOS audio on real hardware.

## Sources
- ScreenCharm: https://screencharm.com
- Screen Studio: https://screen.studio/ and its v4 release coverage (October 2026)
- Cap: https://cap.so/pricing, https://cap.so/features, https://github.com/CapSoftware/Cap
- Tella: https://www.tella.com/pricing
- Loom: https://www.loom.com/pricing, https://www.atlassian.com/software/loom/pricing
- Arcade: https://www.arcade.software/pricing---interactive-demos
- Descript: https://www.descript.com/pricing
- Supademo: https://supademo.com/pricing
- FocuSee: https://focusee.imobie.com/pricing.htm
