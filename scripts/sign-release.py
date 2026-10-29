#!/usr/bin/env python3
"""Sign a published release's update files and add latest.json to it.

Only needed when the release workflow ran without the TAURI_SIGNING_PRIVATE_KEY
secret. Run it where the updater private key lives:

  GITHUB_TOKEN=... python3 scripts/sign-release.py v0.5.0

The key is read from BOOYA_UPDATER_KEY (default /workspace/secrets/booya-updater.key)
and its password from BOOYA_UPDATER_KEY_PASSWORD_FILE (default: the same path
ending in .password). Neither is printed. Needs node (npx) for the Tauri signer.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request

REPO = os.environ.get("BOOYA_REPO", "im45145v/Booya")
KEY = os.environ.get("BOOYA_UPDATER_KEY", "/workspace/secrets/booya-updater.key")
PW = os.environ.get("BOOYA_UPDATER_KEY_PASSWORD_FILE", KEY.replace(".key", ".password"))
TOKEN = os.environ.get("GITHUB_TOKEN") or os.environ.get("GITHUB_TOKEN_BOOYA")
WANT = [r"\.app\.tar\.gz$", r"\.AppImage$", r"-setup\.exe$"]


def call(method, url, data=None, headers=None, raw=False):
    h = {"Authorization": "Bearer " + TOKEN, "Accept": "application/vnd.github+json", "User-Agent": "booya-sign"}
    h.update(headers or {})
    req = urllib.request.Request(url, method=method, data=data, headers=h)
    with urllib.request.urlopen(req) as r:
        b = r.read()
        return b if raw else (json.loads(b) if b else {})


def main(tag):
    if not TOKEN:
        sys.exit("set GITHUB_TOKEN")
    rel = call("GET", f"https://api.github.com/repos/{REPO}/releases/tags/{tag}")
    assets = {a["name"]: a for a in rel["assets"]}
    todo = [n for n in assets if any(re.search(p, n) for p in WANT) and n + ".sig" not in assets]
    if not todo and "latest.json" in assets:
        print("already signed")
        return
    env = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY_PASSWORD=open(PW).read().strip())
    env.pop("TAURI_SIGNING_PRIVATE_KEY", None)
    with tempfile.TemporaryDirectory() as d:
        for n, a in assets.items():
            if n.endswith(".sig") or any(re.search(p, n) for p in WANT):
                path = os.path.join(d, n)
                if n.endswith(".sig"):
                    open(path, "wb").write(call("GET", a["url"], headers={"Accept": "application/octet-stream"}, raw=True))
                    continue
                open(path, "wb").write(call("GET", a["url"], headers={"Accept": "application/octet-stream"}, raw=True))
        for n in todo:
            subprocess.run(["npx", "--yes", "@tauri-apps/cli@2", "signer", "sign", "-f", KEY, os.path.join(d, n)], check=True, env=env, stdout=subprocess.DEVNULL)
            print("signed", n)
        notes = ""
        try:
            notes = call("GET", f"https://raw.githubusercontent.com/{REPO}/{tag}/docs/release-notes/{tag}.md", raw=True).decode()
        except Exception:
            notes = rel.get("body") or ""
        open(os.path.join(d, "notes.md"), "w").write(notes)
        here = os.path.dirname(os.path.abspath(__file__))
        doc = subprocess.run([sys.executable, os.path.join(here, "make-latest-json.py"), d, tag, REPO, os.path.join(d, "notes.md")], check=True, capture_output=True, text=True).stdout
        open(os.path.join(d, "latest.json"), "w").write(doc)
        up = rel["upload_url"].split("{")[0]
        for n in todo + ["latest.json"]:
            fn = n + ".sig" if n != "latest.json" else n
            if fn in assets:
                call("DELETE", assets[fn]["url"])
            ctype = "application/json" if fn.endswith(".json") else "text/plain"
            call("POST", f"{up}?name={fn}", data=open(os.path.join(d, fn), "rb").read(), headers={"Content-Type": ctype})
            print("uploaded", fn)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
