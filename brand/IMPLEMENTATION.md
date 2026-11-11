# Booya brand kit: implementation notes

How to land brand kit v1.0 (direction E, Ghost x Ink mono) in the Tauri app in a single PR.

**The v1 branding must go completely.** v0.7.0 shipped the lavender v1 identity (lavender #E9E4FB, violet #7C5CFA, pink #F7B6C8, sticker style ghost). Every icon, colour, screenshot and line of copy from it gets replaced. Nothing from v1 is kept or mixed in.

Paths below are relative to this kit (`booya-brand/`) and to the app repo root.

## 1. App icons

Copy `assets/tauri-icons/*` into `src-tauri/icons/` and overwrite the old files. The names match what `tauri icon` generates:

| Kit file | Repo path | Used by |
|---|---|---|
| `tauri-icons/32x32.png` | `src-tauri/icons/32x32.png` | Windows, Linux |
| `tauri-icons/128x128.png`, `128x128@2x.png` | same names | Linux, AppImage |
| `tauri-icons/icon.icns` | `src-tauri/icons/icon.icns` | macOS bundle (Apple grid variant, 824 px body) |
| `tauri-icons/icon.ico` | `src-tauri/icons/icon.ico` | Windows exe and installer (16 to 256) |
| `tauri-icons/icon.png` | `src-tauri/icons/icon.png` | fallback, 512 px |
| `tauri-icons/Square*Logo.png`, `StoreLogo.png` | same names | MSIX / Windows Store |

Check `src-tauri/tauri.conf.json`:

```json
"bundle": {
  "icon": [
    "icons/32x32.png",
    "icons/128x128.png",
    "icons/128x128@2x.png",
    "icons/icon.icns",
    "icons/icon.ico"
  ]
}
```

Notes:
- Sizes 32 px and below come from hand tuned SVGs (`assets/app-icon/booya-icon-32.svg`, `-16.svg`) with pixel snapped eyes. Do not regenerate them with `tauri icon` from the 1024 master, because that would replace them with blurry downscales. If you do run `tauri icon`, use `assets/app-icon/booya-icon-1024.png` and then copy the 32x32 and .ico from this kit back over.
- Linux packages (deb, rpm, Flatpak) can install `assets/linux/hicolor/<size>/apps/booya.png` and `scalable/apps/booya.svg` into `/usr/share/icons/hicolor/`.

## 2. Tray icon

There are two states, idle and recording. The only difference is that recording adds the ripple arc. Do not add a red dot, animation or badge.

**macOS:** use the template images in `assets/tray/macos/`. They are pure black with alpha, and macOS tints them for light and dark menu bars. Put them in `src-tauri/icons/tray/`.

- `tray-idleTemplate.png` / `@2x.png` (18 pt)
- `tray-recordingTemplate.png` / `@2x.png`
- PDF and SVG versions are included in case you need vector sources.

**Windows:** use the pixel drawn 16 px glyph from `assets/tray/windows/`.

- `tray-idle-light.ico`, `tray-recording-light.ico`: ink glyph for the **light** taskbar theme
- `tray-idle-dark.ico`, `tray-recording-dark.ico`: paper glyph for the **dark** taskbar theme (the Windows default)

Read `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme` (1 means a light taskbar) at startup and when the theme changes, and pick the matching file.

**Linux:** use the Windows dark PNGs (`tray-*-dark-16.png`) or the hicolor 22/24 px app icon.

Sketch for Tauri v2 (`tauri` with the `tray-icon` and `image-png` features):

```rust
use tauri::{image::Image, tray::TrayIconBuilder};

#[cfg(target_os = "macos")]
const IDLE: &[u8] = include_bytes!("../icons/tray/tray-idleTemplate@2x.png");
#[cfg(target_os = "macos")]
const REC: &[u8] = include_bytes!("../icons/tray/tray-recordingTemplate@2x.png");
// Windows: include both light and dark .ico files and choose by SystemUsesLightTheme.

let tray = TrayIconBuilder::with_id("main")
    .icon(Image::from_bytes(IDLE)?)
    .icon_as_template(cfg!(target_os = "macos"))
    .tooltip("Booya")
    .build(app)?;

// on record start / stop
tray.set_icon(Some(Image::from_bytes(if recording { REC } else { IDLE })?))?;
#[cfg(target_os = "macos")]
tray.set_icon_as_template(true)?;
```

The tooltip reads `Booya` when idle and `Booya, recording 0:42` while recording. Keep the timer in the menu bar title on macOS (`tray.set_title(Some("0:42"))`) as shown in guideline section 17 (In the wild: desktop).

## 3. Web: favicon and manifest

Copy `assets/favicon/*` to the site or docs `public/` folder:

```html
<link rel="icon" href="/favicon.ico" sizes="32x32">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">
<meta name="theme-color" content="#F4F3EF" media="(prefers-color-scheme: light)">
<meta name="theme-color" content="#111110" media="(prefers-color-scheme: dark)">
<meta property="og:image" content="/og-image.png">
```

`favicon.svg` has a `prefers-color-scheme: dark` media query built in, so it flips to paper on ink by itself. If the app frontend has its own `index.html` favicon, change it too.

## 4. README and GitHub

Put `assets/social/readme-header-light.png` and `readme-header-dark.png` in `docs/brand/` (or `.github/`), then start the README with:

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/readme-header-dark.png">
  <img alt="Booya, open source screen recorder" src="docs/brand/readme-header-light.png" width="100%">
</picture>
```

- Repository: https://github.com/im45145v/Booya
- Repo settings, then Social preview: upload `assets/social/github-social-preview.png` (1280x640).
- Change the repo description to: `Open source screen recorder with offline captions and auto zoom on clicks.`
- Replace every v1 screenshot and GIF in the README and docs with captures of the new UI.
- `assets/social/post-template-x-linkedin.png` is the launch post image. Its editable HTML is in `assets/social/src/`.

## 5. Design tokens in the frontend

1. Copy `tokens/tokens.css` to `src/styles/tokens.css`, and `fonts/` to `src/styles/fonts/` (or change the `@font-face` urls, which point to `../fonts/`).
2. Import it once at the entry point: `import "./styles/tokens.css";`
3. Theme handling:
   - By default the theme follows `prefers-color-scheme`.
   - An explicit user choice sets `document.documentElement.dataset.theme = "light" | "dark"`, which overrides the system setting. Remove the attribute to go back to "System".
4. Tailwind (if used): in `tailwind.config.js`, add `presets: [require("./tokens/tailwind.preset.js")]`. Colours map to the CSS variables, so `bg-paper`, `text-ink`, `border-rule`, `bg-signal` and so on switch with the theme.
5. Replace hard coded colours. Search for `#E9E4FB`, `#7C5CFA`, `#F7B6C8`, `lavender`, `violet`, `purple` and `pink`, plus any old font family, and map each to a `--by-*` variable.

Rules that matter in code:
- **Signal fill is for Record and Stop only.** The app's Record button is a Signal fill in idle and while recording, with Ink text. Signal also marks record dots, the recording timer, the playhead, the ripple and the ghost's arc. Primary actions (Run auto edit, Export, Save) are Paper on Night or Ink on Paper, never orange. Keep Signal under 2% of any screen.
- The app UI uses the `--by-ui-*` layer in `tokens.css` (surfaces, borders, text tiers, actions). Its contrast pairs are listed at the end of `tokens/CONTRAST.md`.
- Never use Signal for body text on Paper (2.81:1). For orange text on light backgrounds use `--by-signal-text` (#BF4317, 4.69:1 on Paper).
- Text on a Signal fill is Ink, never white (white is 3.12:1).
- Put the record dot on Surface (the toolbar), not directly on Paper.
- Focus ring: `--by-focus` with a 2 px outline and 2 px offset, already set on `:focus-visible` in tokens.css. Never orange, never a glow.
- Motion: ripple 400 ms, blink 120 ms, UI 150 to 200 ms. `prefers-reduced-motion` is handled in tokens.css. Keep that path working.
- Full contrast results are in `tokens/CONTRAST.md`.

## 6. Copy and voice

- The name is always `Booya`, with a capital B, in UI copy. The wordmark is lowercase artwork.
- Short, plain sentences. No exclamation marks, no emoji in UI, no em dashes.
- Examples: `Saved. 48 MB in Movies/Booya.`, `Captions are written on this device.`, `Couldn't save. The disk is full.`
- Use `Booya` consistently in the window title, `productName` in tauri.conf.json, menu items, About box, installer text, the updater feed name and the docs. Keep the existing `identifier` unless you accept that changing it resets user settings.

## 7. PR checklist

- [ ] `src-tauri/icons/` replaced with `assets/tauri-icons/`; hand tuned 32 px kept
- [ ] macOS: icns shows the Apple grid icon in Dock, Finder and the About box
- [ ] Windows: exe, installer, taskbar and Alt Tab show the new icon; check at 100%, 150% and 200% scaling
- [ ] Linux: hicolor icons installed; .desktop file uses `Icon=booya`
- [ ] Tray: macOS template idle and recording; Windows light and dark .ico chosen by taskbar theme; switches on record start and stop
- [ ] `tokens.css` and Geist fonts imported; Tailwind preset wired if used
- [ ] Light, dark, System and `[data-theme]` override all tested
- [ ] Settings window, toolbar, caption overlay and focus frame match guideline sections 11 to 14
- [ ] Signal only on record, stop, ripple and focus frame; `--by-signal-text` for any orange text
- [ ] Keyboard focus visible on every control; reduced motion respected
- [ ] Favicon, apple touch icon, manifest and og image on the website and docs
- [ ] README header `<picture>` added; GitHub social preview uploaded; repo description updated
- [ ] **All v0.7.0 lavender v1 branding removed:** old icons, tray PNGs, colours (#E9E4FB, #7C5CFA, #F7B6C8), sticker mascot art, screenshots, GIFs and v1 copy. `git grep -i -E "7c5cfa|e9e4fb|f7b6c8|lavender"` returns nothing
- [ ] CHANGELOG entry: "New look: brand kit v1.0."
- [ ] Release notes screenshots retaken

## Kit map

- `assets/logo/`: icon, wordmark, horizontal and stacked lockups in color, on-paper, on-ink, mono-black and mono-white (SVG, PNG 1x and 2x)
- `assets/app-icon/`: 1024 master, macOS variant, hand tuned 16 and 32, PNGs 16 to 1024, booya.icns, booya.ico
- `assets/linux/hicolor/`, `assets/tauri-icons/`, `assets/tray/`, `assets/favicon/`, `assets/social/`, `assets/mascot/`
- `tokens/`: tokens.json, tokens.css, tailwind.preset.js, CONTRAST.md
- `fonts/`: Geist and Geist Mono variable TTFs with OFL licences
- `guideline/`: booya-brand-guideline.pdf, .html, page PNGs
- `src/`: the Python generators. Run `python3 src/build_all.py` (or each `build_*.py`) to rebuild everything after a geometry change in `src/geom.py`
