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
import json, os, re, subprocess, sys, tempfile
from pathlib import Path

BIN = sys.argv[1]
WORK = Path(sys.argv[2] if len(sys.argv) > 2 else tempfile.mkdtemp(prefix="dl-subs-"))
WORK.mkdir(parents=True, exist_ok=True)
MODEL = "whisper-tiny.en"

os.environ["DEMOLENS_DEBUG_TRANSCRIBE"] = "1"
os.environ["DEMOLENS_NO_TRANSCRIPT_CACHE"] = "1"


def run(*args):
    r = subprocess.run([BIN, *map(str, args)], capture_output=True, text=True)
    print("$ demolens", *args, "->", r.returncode)
    print(r.stdout[-3000:], r.stderr[-12000:])
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

# --- a cut keeps captions in step ---
# Filler words are only cut where nothing happens on screen (the "um" here is
# while the search is typed, so autocut rightly keeps it). So make the cut by hand.
out = run("autocut", sp, "--fillers")
proj = json.loads(Path(sp).read_text())
c0 = proj["clips"][0]
cut_a, cut_b = 5600.0, 6600.0
proj["clips"] = [dict(c0, src_end_ms=cut_a), dict(c0, src_start_ms=cut_b)] + proj["clips"][1:]
Path(sp).write_text(json.dumps(proj))
run("captions", sp, "-o", WORK / "b.srt")
cues = read_cues(WORK / "b.srt")
press2 = find(cues, "press")
cut = (cut_b - cut_a) / 1000
check(abs((press[0] - press2[0]) - cut) < 0.2, f"the later captions moved up by the cut ({press[0] - press2[0]:.2f} vs {cut:.2f} s)")
check(abs(find(cues, "invoice")[0] - inv[0]) < 0.01, "captions before the cut did not move")
total = sum((c["src_end_ms"] - c["src_start_ms"]) / c["speed"] for c in proj["clips"]) / 1000
check(cues[-1][1] <= total + 0.05, "no caption runs past the end of the shortened video")

# --- speech that starts 2 s in: no caption before the voice ---
lead = WORK / "lead.mp4"
subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", str(Path(json.loads(Path(sp).read_text())["source"])), "-af", "adelay=2000:all=1", "-c:v", "copy", str(lead)], check=True)
run("new", lead, "-o", WORK / "lead.json")
lp = WORK / "lead.json"
run("transcribe", lp, "--model", MODEL)
lead_caps = json.loads(Path(lp).read_text())["captions"]
check(len(lead_caps) >= 1, "speech after two seconds of silence is captioned")
if lead_caps:
    first = min(w["start_ms"] for c in lead_caps for w in c["words"]) / 1000
    check(first >= 1.8, f"no word starts before the voice does (first word at {first:.2f} s, voice at 2.0 s)")
    check(lead_caps[0]["start_ms"] / 1000 >= 1.8, f"the first caption starts with the voice ({lead_caps[0]['start_ms'] / 1000:.2f} s)")

# --- fast continuous speech, long enough to need several chunks ---
PASSAGES = [
    "Open the settings page and click the billing tab then choose the invoice you want and press export so the file downloads straight away",
    "After that go back to the dashboard and pick last month from the date menu then scroll down to the table of payments and sort it by amount",
    "Now open the filter panel and tick the paid option and the refunded option so that only finished payments are listed in the table below",
    "Finally copy the summary into the spreadsheet and check that the totals match the report before you send it to the finance team",
]
FAST = " ".join(PASSAGES)
parts = []
for i, text in enumerate(PASSAGES):
    part = WORK / f"fast{i}.wav"
    subprocess.run(["espeak-ng", "-s", "230", "-g", "0", "-w", str(part), text], check=True)
    parts.append(part)
wav = WORK / "fast.wav"
ff = ["ffmpeg", "-y", "-v", "error"]
for part in parts:
    ff += ["-i", str(part)]
ff += ["-filter_complex", "".join(f"[{i}:a]" for i in range(len(parts))) + f"concat=n={len(parts)}:v=0:a=1", str(wav)]
subprocess.run(ff, check=True)
fast_mp4 = WORK / "fast.mp4"
subprocess.run(["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=320x240:r=10", "-i", str(wav), "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", str(fast_mp4)], check=True)
run("new", fast_mp4, "-o", WORK / "fast.json")
fp = WORK / "fast.json"
run("transcribe", fp, "--model", MODEL)
caps = json.loads(Path(fp).read_text())["captions"]
heard = [re.sub(r"[^a-z]", "", w["text"].lower()) for c in caps for w in c["words"]]
want = [re.sub(r"[^a-z]", "", w.lower()) for w in FAST.split()]
got = sum(1 for w in set(want) if w in heard)
last_ms = max((w["end_ms"] for c in caps for w in c["words"]), default=0)
print("fast speech heard:", heard)
print(f"fast speech: {got} of {len(set(want))} different words, last word ends at {last_ms / 1000:.1f} s")
# whisper-tiny on synthetic speech is not perfect, but the whole passage must
# be there, start to end, not the first half only.
check(got / len(set(want)) >= 0.45, f"fast speech: {got} of {len(set(want))} different words heard")
check(any(w in heard for w in ["totals", "report", "finance", "spreadsheet"]), "fast speech: the last passage is not lost")
check(any(w in heard for w in ["open", "settings", "page", "billing", "invoice", "export", "file", "downloads", "click", "choose"]), "fast speech: the first passage is there")
check(any(w in heard for w in ["filter", "panel", "paid", "refunded"]), "fast speech: the middle is not skipped")

# --- no speech: no captions, no failure ---
ns = project_path(run("sample", WORK / "nospeech"))
out = run("transcribe", ns, "--model", MODEL)
captions = json.loads(Path(ns).read_text())["captions"]
check(captions == [], f"a recording without speech gets no captions (got {len(captions)})")

if fails:
    sys.exit("FAILED:\n  " + "\n  ".join(fails))
print("all subtitle checks passed")
