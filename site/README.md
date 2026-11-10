# Booya website

The landing page at the .tech domain. It is plain HTML, CSS and a small script, with no framework and no build tools beyond `sh`.

| File | What it is |
|---|---|
| `index.html` | The whole page |
| `404.html` | Not found page |
| `styles/tokens.css` | Copy of `brand/tokens/tokens.css`. The only change is that the fonts point at Latin subset WOFF2 files |
| `styles/site.css` | Page styles. Every colour, size and duration comes from a `--by-*` token |
| `scripts/site.js` | Finds the visitor's OS and points the download buttons at the latest GitHub release |
| `fonts/` | Geist and Geist Mono, subset to Latin, with their OFL licences |
| `build.sh` | Builds `dist/`: fills in the site URL and adds content hashes to CSS and JS names |

## Run it locally

```sh
sh site/build.sh
python3 -m http.server 8080 --directory site/dist
```

Then open http://localhost:8080. `SITE_URL=https://example.tech sh site/build.sh` sets the URL used in the canonical link, Open Graph tags and sitemap.

## Downloads

`scripts/site.js` asks `api.github.com/repos/im45145v/Booya/releases/latest` for the release and matches assets by name (`.dmg`, `-setup.exe`, `.msi`, `.AppImage`, `.deb`, `.rpm`). The answer is cached in `sessionStorage` for 10 minutes, so a visitor uses one of GitHub's 60 unauthenticated calls per hour at most. If the call fails, every button already links to the releases page. If a release ever renames its files, update `PATTERNS` in the script.

## Brand rules that apply here

- Ink and Paper do the work. Signal orange is only for the record dot, the click ripple, the focus frame and small labels.
- Orange text uses `--by-signal-text`, never `--by-signal`. Text on an orange fill is Ink.
- The hero animation stops for `prefers-reduced-motion` and shows the zoomed frame instead. It also pauses on hover.
- No exclamation marks, no emoji and no em dashes in copy.

## Deploying

See [docs/DEPLOY_SITE.md](../docs/DEPLOY_SITE.md). The Render Blueprint is `render.yaml` in the repo root.
