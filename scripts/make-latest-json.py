#!/usr/bin/env python3
"""Build latest.json, the file the in-app updater reads, from signed release files.

usage: make-latest-json.py <dir with assets> <tag> <owner/repo> [notes.md] > latest.json

A platform is listed only when both its update file and the matching .sig are
in the directory. Exits 1 when no platform qualifies.
"""
import datetime
import json
import os
import re
import sys

PLATFORMS = [
    # (updater platform key, file pattern)
    ("darwin-aarch64", r"\.app\.tar\.gz$"),
    ("linux-x86_64", r"\.AppImage$"),
    ("windows-x86_64", r"-setup\.exe$"),
]


def build(directory, tag, repo, notes=""):
    names = sorted(os.listdir(directory))
    platforms = {}
    for key, pat in PLATFORMS:
        for n in names:
            if re.search(pat, n) and n + ".sig" in names:
                sig = open(os.path.join(directory, n + ".sig")).read().strip()
                platforms[key] = {"signature": sig, "url": f"https://github.com/{repo}/releases/download/{tag}/{n}"}
                break
    if not platforms:
        return None
    return {
        "version": tag.lstrip("v"),
        "notes": notes,
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }


if __name__ == "__main__":
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    notes = open(sys.argv[4]).read() if len(sys.argv) > 4 and os.path.exists(sys.argv[4]) else ""
    doc = build(sys.argv[1], sys.argv[2], sys.argv[3], notes)
    if doc is None:
        print("no signed update files found", file=sys.stderr)
        sys.exit(1)
    print(json.dumps(doc, indent=2))
