#!/usr/bin/env python3
"""Subtitles on real speech, end to end, through the shipped CLI.

Needs espeak-ng (narration), ffmpeg and the whisper-tiny.en model. It checks:
  * transcription finds the spoken words, at the right times
  * SRT and VTT agree
  * cutting filler words keeps the captions in step with the shortened video
  * a recording with no speech gets no captions and does not fail
  * captions are saved in the project file

Usage: scripts/subtitles_e2e.py <path to demolens binary> [work dir]
"""
import json, re, subprocess, sys, tempfile
from pathlib import Path

BIN = sys.argv[1]
WORK = Path(sys.argv[2] if len(sys.argv) > 2 else tempfile.mkdtemp(prefix="dl-subs-"))
WORK.mkdir(parents=True, exist_ok=True)
MODEL = "whisper-tiny.en"


def run(*args):
    r = subprocess.run([BIN, *map(str, args)], capture_output=True, text=True)
    print("$ demolens", *args, "->", r.returncode)
    print(r.stdout[-3000:], r.stderr[-400:])
    if r.returncode != 0:
        sys.exit(f"demolens {args[0]} failed")
    return r.stdout


def secs(t):
    h, m, rest = t.replace(",", ".").split(":")
    return int(h) * 3600 + int(m) * 60 + float(rest)


def read_cues(path, vtt=False):
    text = Path(path).read_text()
    cues = []
    for m in re.finditer(r"(\d\d:\d\d:\d\d[,.]\d\d\d) --> (\d\d:\d\d:\d\d[,.]\d\d\d)\n(.+?)(?:\n\n|\Z)", text, re.S):
        cues.append((secs(m.group(1)), secs(m.group(2)), m.group(3).replace("\n", " ")))
    return cues


def find(cues, word):
    for c in cues:
        if word in c[2].lower():
            return c
    sys.exit(f"no caption mentions '{word}': {cues}")


def project_path(out):
    return next(l.split(" ", 1)[1].strip() for l in out.splitlines() if l.startswith("project "))


fails = []


def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        fails.append(what)


run("models", "download", MODEL)

# --- speech ---
sp = project_path(run("sample", WORK / "speech", "--speech"))
run("transcribe", sp, "--model", MODEL)
for c in json.loads(Path(sp).read_text())["captions"]:
    print("  words:", [(w["text"], round(w["start_ms"]), round(w["end_ms"])) for w in c["words"]])
run("captions", sp, "--burn", "on", "-o", WORK / "a.srt")
run("captions", sp, "-o", WORK / "a.vtt")
srt, vtt = read_cues(WORK / "a.srt"), read_cues(WORK / "a.vtt", True)
print(srt)
check(len(srt) >= 3, "at least three caption lines from four spoken sentences")
text = " ".join(c[2].lower() for c in srt)
for w in ["invoice", "march", "save"]:
    check(w in text, f"the word '{w}' was heard")
check(len(srt) == len(vtt) and all(abs(a[0] - b[0]) < 0.002 and abs(a[1] - b[1]) < 0.002 for a, b in zip(srt, vtt)), "SRT and VTT have the same cues and times")
check(all(a[0] < a[1] for a in srt) and all(srt[i][1] <= srt[i + 1][0] + 0.001 for i in range(len(srt) - 1)), "cues are in order and do not overlap")
inv, mar, press = find(srt, "invoice"), find(srt, "march"), find(srt, "press")
check(abs(inv[0] - 0.3) < 1.3, f"'invoices' caption starts near 0.3 s (got {inv[0]:.2f})")
check(abs(mar[0] - 2.4) < 1.5, f"'March' caption starts near 2.4 s (got {mar[0]:.2f})")
check(abs(press[0] - 7.0) < 1.3, f"'Press control S' caption starts near 7.0 s (got {press[0]:.2f})")
saved = json.loads(Path(sp).read_text())
check(len(saved["captions"]) == len(srt) and saved["caption_style"]["burn_in"] is True, "captions and burn in are saved in the project")
check(all(c["words"] for c in saved["captions"]), "every saved caption keeps its word timings")

# --- cutting filler words keeps captions in step ---
run("autocut", sp, "--fillers", "--dry-run")
out = run("autocut", sp, "--fillers")
m = re.search(r"output ([\d.]+)s -> ([\d.]+)s", out)
check(bool(m), "autocut reports the new length")
if m:
    before, after = float(m.group(1)), float(m.group(2))
    cut = before - after
    check(cut > 0.15, f"filler words were cut ({cut:.2f} s)")
    run("captions", sp, "-o", WORK / "b.srt")
    cuts = read_cues(WORK / "b.srt")
    text2 = " ".join(c[2].lower() for c in cuts)
    check(not re.search(r"\b(um|uh|uhm)\b", text2), "filler words are gone from the captions")
    press2 = find(cuts, "press")
    check(abs((press[0] - press2[0]) - cut) < 0.5, f"the later captions moved up by the cut ({press[0] - press2[0]:.2f} vs {cut:.2f} s)")
    check(cuts[-1][1] <= after + 0.2, "no caption runs past the end of the shortened video")

# --- no speech: no captions, no failure ---
ns = project_path(run("sample", WORK / "nospeech"))
out = run("transcribe", ns, "--model", MODEL)
captions = json.loads(Path(ns).read_text())["captions"]
check(captions == [], f"a recording without speech gets no captions (got {len(captions)})")

if fails:
    sys.exit("FAILED:\n  " + "\n  ".join(fails))
print("all subtitle checks passed")
