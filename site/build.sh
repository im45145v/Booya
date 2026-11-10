#!/bin/sh
# Builds the static site into site/dist. No Node, no npm, just POSIX sh.
#
# 1. Fills in the site URL (canonical link, Open Graph tags, sitemap, robots).
#    Set SITE_URL in Render, for example https://booya.tech. Until you do,
#    the Render URL of the service is used.
# 2. Gives CSS and JS a content hash in their file names, so they can be cached
#    for a year and still update on the next deploy.
set -eu
cd "$(dirname "$0")"

url="${SITE_URL:-${RENDER_EXTERNAL_URL:-https://booya-site.onrender.com}}"
url="${url%/}"
today="$(date -u +%Y-%m-%d)"

rm -rf dist
mkdir -p dist
cp -R index.html 404.html robots.txt sitemap.xml site.webmanifest \
  favicon.ico favicon.svg apple-touch-icon.png icon-192.png icon-512.png og-image.png \
  fonts styles scripts img dist/

hash() { sha256sum "$1" | cut -c1-10; }
for f in styles/tokens.css styles/site.css scripts/site.js; do
  h="$(hash "dist/$f")"
  base="${f%.*}"; ext="${f##*.}"
  new="$base.$h.$ext"
  mv "dist/$f" "dist/$new"
  for page in dist/index.html dist/404.html; do
    sed -i "s#/$f\"#/$new\"#g" "$page"
  done
done

for page in dist/index.html dist/404.html dist/robots.txt dist/sitemap.xml; do
  sed -i -e "s#__SITE_URL__#$url#g" -e "s#__BUILD_DATE__#$today#g" "$page"
done

if grep -rq "__SITE_URL__" dist; then echo "placeholder left in dist" >&2; exit 1; fi
echo "Built site for $url"
