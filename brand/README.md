# Booya brand

This folder holds everything that makes Booya look like Booya. The look is called Ghost x Ink mono: a small ghost mark, ink on warm paper, and one orange that only ever means "recording".

Here is what is in it.

- `guideline/` has the full guideline as a PDF and as a web page, plus one image per page. Read it before you design anything new.
- `assets/` has the logo, the app and tray icons, the favicons, social images and the mascot. Use these files as they are rather than redrawing them.
- `tokens/` has the colours, type, spacing and motion as CSS, JSON and a Tailwind preset. `CONTRAST.md` lists which colour pairs pass accessibility checks.
- `fonts/` has Geist and Geist Mono, both under the SIL Open Font Licence (the `OFL-*.txt` files).
- `src/` has the scripts that build the assets and the guideline.
- `IMPLEMENTATION.md` explains where each file goes in the app.

A few rules matter most. The name is always Booya with a capital B. Orange is only for the record dot, the Stop button, the click ripple and the focus ring. When orange is used for text on a light background, use the darker signal text colour (`#BF4317`). Text on an orange button is ink, never white.

If you change the tokens, copy `tokens/tokens.css` to `ui/styles/tokens.css` and `site/styles/tokens.css` so the app and the website stay in step.
