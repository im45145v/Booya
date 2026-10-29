# Booya brand guideline v1

Written with Ludo. This is the source for the UI tokens at the top of `ui/style.css`.
The files in this folder are the master artwork. `src-tauri/icons/` is generated from `icon-1024.png`.

## Name and tagline
- The app is **Booya** in sentences. The wordmark is always lowercase: `booya`.
- Tagline: **Record it. Booya.**

## Palette

| Role | Light | Dark |
|---|---|---|
| Background | `#FBFAFF` | `#14121F` |
| Surface | `#F3F0FC` | `#1E1B2E` |
| Border | `#E2DCF5` | `#2E2A42` |
| Text | `#1E1B2E` | `#F1EEFA` |
| Muted text | `#6B6580` | `#A19BB8` |
| Accent | `#7C5CFA` | `#9B82FF` |
| Accent hover | `#6A48F0` | `#B09CFF` |
| Accent subtle | `#E9E4FB` | `#2A2447` |
| Record red | `#E5484D` | `#FF6369` |

Icon colours: tile `#E9E4FB`, ghost `#FFFFFF`, outline `#1E1B2E`, ripple `#7C5CFA`, blush `#F7B6C8` at about 40%.

Two shades in the CSS are not in the table: `--surface-2` (inputs and buttons) and `--line-strong`. They sit between surface and border and are derived, not new colours.

## Accent rule
Purple is only for primary buttons, active states, 2px focus rings at 40% alpha, links and progress. Keep it under about 10% of any screen. Recording is always red, never purple.

## Type
- UI: **Inter**.
- Timecodes and shortcuts: **JetBrains Mono** with tabular numbers (`font-variant-numeric: tabular-nums`).
- Wordmark: **Nunito Sans** 700, lowercase, letter-spacing -1%, no effects (no shadow, gradient or outline).
- The fonts are bundled in `ui/fonts/` (Latin subsets, WOFF2) because the app works offline. All three are SIL OFL 1.1; the licence texts sit next to them.

## Motion
- The click ripple is one ring. It scales from 0.4 to 1.6 while opacity goes from 0.6 to 0, over 400 ms with `cubic-bezier(0.2, 0.8, 0.2, 1)` (`.ripple` in `style.css`).
- The ghost blinks only when recording starts and stops (`.blink`).
- UI transitions take 150 to 200 ms.
- With `prefers-reduced-motion`, motion becomes a plain fade.

## Voice
Plain and friendly. No exclamation marks, and never "AI-powered" or "seamless". Say what happened and what to do next.

Microcopy to reuse: `Recording`, `Saved. Nice one.`, `Captions ready, all offline.`

## Assets in this folder

| File | Use |
|---|---|
| `icon.svg`, `icon-1024.png` | App icon master and its 1024 px render |
| `favicon.svg`, `favicon.ico` | Docs and web pages. A bolder ghost with no ripple, so it holds up at 16 px |
| `tray-template.svg` and `.png` | macOS menu bar glyph, idle. Black on clear, used as a template image |
| `tray-recording.svg` and `.png` | The same glyph filled in, shown while recording |
| `tray-*@2x.png`, `tray-*@2x.rgba` | 88 px versions. The `.rgba` files are raw pixels that the app embeds |

To rebuild the icon set: render `icon.svg` to `icon-1024.png`, then run `tauri icon brand/icon-1024.png -o src-tauri/icons` and delete the `android` and `ios` folders it creates.

## Not covered yet
- Exported videos keep their own look (background presets, cursor and click ripple colours). This guideline is about the app UI.
- The Windows tray uses the full colour app icon.
