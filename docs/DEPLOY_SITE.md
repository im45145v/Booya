# Put the Booya website on Render with a .tech domain

This takes about 15 minutes, plus however long DNS takes to spread (usually under an hour). You need a GitHub account with access to `im45145v/Booya` and your .tech domain. In the steps below, `YOUR-DOMAIN.tech` stands for your real domain, for example `booya.tech`.

The website lives in `site/`. The Render setup lives in `render.yaml` at the repo root, so Render reads the build command, headers, caching and redirects from there and you don't type them in by hand.

## 1. Create the site on Render

1. Go to https://dashboard.render.com and sign in with **GitHub**.
2. Click **New +** in the top right, then **Blueprint**.
3. Under **Connect a repository**, find `im45145v/Booya`. If it isn't listed, click **Configure account** (or **Connect GitHub**), choose your account, allow access to the `Booya` repository and click **Save**. Back on Render, click **Connect** next to `im45145v/Booya`.
4. **Blueprint Name**: type `booya`. **Branch**: `main`.
5. Render reads `render.yaml` and shows one service, `booya-site`, a static site. It asks for **SITE_URL**:
   - If the domain is already set up, enter `https://YOUR-DOMAIN.tech` (no slash at the end).
   - If not, leave it empty for now. The site then uses its `onrender.com` address in its links until you fill this in.
6. Click **Deploy Blueprint**. The first build takes under a minute.
7. Click **booya-site**, then the URL at the top, which looks like `https://booya-site.onrender.com` (Render may add a few letters if the name is taken). Check that the page loads and that the download button says "Download for" your system. Write this URL down, you need it in step 3.

The free plan is enough. Static sites don't sleep.

## 2. Add the domain in Render

1. Open **booya-site**, then **Settings**, and scroll to **Custom Domains**.
2. Click **+ Add Custom Domain**, type `YOUR-DOMAIN.tech` and click **Save**.
3. Render also adds `www.YOUR-DOMAIN.tech` by itself and redirects it to the root domain. Both now show **DNS update needed**. Keep this tab open.

## 3. Point the domain at Render

Log in where you manage the domain's DNS. For most .tech domains that is the registrar you bought it from (get.tech, Namecheap, Hostinger, name.com, GoDaddy and so on). With the GitHub Student Pack domain, log in at https://get.tech, open **My Domains**, then **Manage**, then **DNS Management** (sometimes called **Manage DNS**).

First clean up:

- Delete any **AAAA** records for `@` and `www`. Render only uses IPv4 and these break verification.
- Delete any existing **A** record or **CNAME** for `@` and `www` that points somewhere else, such as a parking page.
- Turn off **domain forwarding** or **URL redirect** if the panel has it switched on.

Then add these two records:

| Type | Host / Name | Value / Points to | TTL |
|---|---|---|---|
| A | `@` (some panels want it empty, or `YOUR-DOMAIN.tech`) | `216.24.57.1` | 3600, or the lowest offered |
| CNAME | `www` | `booya-site.onrender.com` (your URL from step 1.7, without `https://`) | 3600, or the lowest offered |

If your DNS provider supports **ALIAS** or **ANAME** records for the root, you can use one pointing `@` at `booya-site.onrender.com` instead of the A record. Either works.

**Using Cloudflare for DNS?** Don't add the A record. Add a **CNAME** for `@` pointing at `booya-site.onrender.com` (Cloudflare flattens it) and a **CNAME** for `www` pointing at the same, and set both to **DNS only** (grey cloud) until Render shows them as verified.

Save the records.

## 4. Verify

1. Wait a few minutes. To check from a terminal:

   ```sh
   dig +short YOUR-DOMAIN.tech       # should print 216.24.57.1
   dig +short www.YOUR-DOMAIN.tech   # should print booya-site.onrender.com. and an IP
   ```

   Or paste the domain into https://dnschecker.org.
2. Back in Render, **Settings**, **Custom Domains**, click **Verify** next to each domain. When it turns green, Render issues the HTTPS certificate by itself.
3. Open `https://YOUR-DOMAIN.tech`. If you get a 502 or a certificate warning, wait five minutes and reload.

If verification keeps failing after an hour, look for leftover AAAA records or forwarding first. Flushing the public DNS caches helps too: https://developers.google.com/speed/public-dns/cache and https://1.1.1.1/purge-cache/.

## 5. Tell the site its real address

1. In Render, open **booya-site**, then **Environment**.
2. Set `SITE_URL` to `https://YOUR-DOMAIN.tech` and click **Save Changes**.
3. Click **Manual Deploy**, then **Deploy latest commit**. The canonical link, Open Graph tags, sitemap and robots.txt now use your domain.
4. Optional: in `render.yaml`, uncomment the `domains:` lines and put your domain there, so the setup is written down in the repo.

## 6. Finish up

- On GitHub, open the repo, click the gear next to **About** and set **Website** to `https://YOUR-DOMAIN.tech`.
- Optional: add a Website link to the row of links under the README header.
- Check the share preview: paste the URL into https://www.opengraph.xyz. It should show `og-image.png`.
- Optional: add the domain to Google Search Console and submit `https://YOUR-DOMAIN.tech/sitemap.xml`.

## How updates work

Every push to `main` that changes `site/` or `render.yaml` redeploys the site within a minute. Other commits (the Rust app, docs) don't trigger a build. Pull requests that touch the site get their own preview URL, which Render posts on the PR.

## What the Blueprint sets up

- **Build**: `sh site/build.sh` copies `site/` to `site/dist`, fills in `SITE_URL` and adds a content hash to the CSS and JS file names. No Node or npm.
- **Security headers** on every page: a strict Content Security Policy (only the site itself, plus `api.github.com` for the download buttons), HSTS, `X-Frame-Options: DENY`, `nosniff`, a referrer policy, and a permissions policy that turns off camera, microphone and location.
- **Caching**: CSS, JS and fonts are cached for a year (their names change when they change). Images (including `/img/`) for a week. HTML uses Render's default, and every deploy clears the CDN.
- **Short links**: `/download` jumps to the download section, `/github` goes to the repo, `/releases` to the latest release.

If you change the page to load something from another site (a font, an analytics script, an embedded video), add that host to the `Content-Security-Policy` header in `render.yaml`, or the browser will block it.
