"""Dragging the camera bubble on the preview: it moves, snaps to corners and
the centre, resizes on scroll and with its grip, nudges with the arrow keys,
undoes, and only shows in the bubble layout while paused.
Run: python3 tests/e2e/test_camera_drag.py <fixture dir> [shots dir]"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright
sys.path.insert(0, str(Path(__file__).parent))
from harness import Studio

fx = Path(sys.argv[1])
shots = Path(sys.argv[2]) if len(sys.argv) > 2 else None
if shots:
    shots.mkdir(parents=True, exist_ok=True)
failed = []


def check(ok, msg):
    print(("ok   " if ok else "FAIL ") + msg)
    if not ok:
        failed.append(msg)


RECT = "() => { const h = document.getElementById('cam-handle'), i = document.getElementById('preview'); const a = h.getBoundingClientRect(), b = i.getBoundingClientRect(); return { hidden: h.hidden, x: a.left, y: a.top, w: a.width, h: a.height, px: b.left, py: b.top, pw: b.width, ph: b.height }; }"

with sync_playwright() as pw:
    s = Studio(pw, fx, viewport=(1440, 900))
    s.project["webcam"] = {"path": str(fx / "sample.mp4"), "visible": True, "corner": "bottom-right", "size": 0.24, "circle": True, "offset_ms": 0, "layout": "bubble", "mirror": False}
    p = s.open(); s.load_sample()
    p.wait_for_timeout(500)
    r = p.evaluate(RECT)
    check(not r["hidden"], "the bubble handle shows over the preview")
    check(r["x"] > r["px"] + r["pw"] / 2 and r["y"] > r["py"] + r["ph"] / 2, f"it starts in the bottom right {r}")
    check(abs(r["w"] - r["h"]) < 1 and abs(r["w"] - round(0.24 * r["ph"])) <= 2, f"it is the bubble's size {r}")

    # Drag it to an open spot: it stays there.
    cx, cy = r["x"] + r["w"] / 2, r["y"] + r["h"] / 2
    tx, ty = r["px"] + r["pw"] * 0.3, r["py"] + r["ph"] * 0.4
    p.mouse.move(cx, cy); p.mouse.down(); p.mouse.move((cx + tx) / 2, (cy + ty) / 2, steps=4); p.mouse.move(tx, ty, steps=6); p.mouse.up()
    p.wait_for_timeout(300)
    pos = p.evaluate("project.webcam.position")
    check(pos is not None and abs(pos[0] - 0.3) < 0.02 and abs(pos[1] - 0.4) < 0.02, f"the drop spot is saved {pos}")
    check(p.evaluate("$('cam-corner').value") == "custom", "the position list says where you dragged it")
    last = [a for c, a in s.calls if c == "preview"][-1]
    check(last["project"]["webcam"].get("position") == pos, "the preview is asked for with the new spot")
    r2 = p.evaluate(RECT)
    check(abs(r2["x"] + r2["w"] / 2 - tx) < 3 and abs(r2["y"] + r2["h"] / 2 - ty) < 3, f"the handle is where it was dropped {r2}")
    if shots:
        p.screenshot(path=str(shots / "camera-dragged.png"))

    # Undo puts it back in the corner.
    p.keyboard.press("Control+z")
    p.wait_for_timeout(300)
    check(p.evaluate("project.webcam.position") is None and p.evaluate("project.webcam.corner") == "bottom-right", "undo puts it back")
    p.keyboard.press("Control+y") if p.evaluate("$('redo-btn').disabled") is False else None
    p.wait_for_timeout(200)

    # Drag near the top left corner: it snaps into it and forgets the spot.
    r = p.evaluate(RECT)
    cx, cy = r["x"] + r["w"] / 2, r["y"] + r["h"] / 2
    m = 0.035 * r["ph"]
    tx, ty = r["px"] + m + r["w"] / 2 + 9, r["py"] + m + r["h"] / 2 - 7
    p.mouse.move(cx, cy); p.mouse.down(); p.mouse.move(tx, ty, steps=8)
    check(p.evaluate("document.getElementById('cam-handle').classList.contains('snapped')"), "it shows that it will snap")
    p.mouse.up(); p.wait_for_timeout(300)
    check(p.evaluate("[project.webcam.corner, project.webcam.position ?? null]") == ["top-left", None], "it snaps to the top left corner")
    check(p.evaluate("$('cam-corner').value") == "top-left" and p.evaluate("!$('cam-corner').querySelector('option[value=custom]')"), "the list shows the corner again")

    # Scroll on it to resize; the slider follows.
    r = p.evaluate(RECT)
    p.mouse.move(r["x"] + r["w"] / 2, r["y"] + r["h"] / 2)
    p.mouse.wheel(0, -100); p.mouse.wheel(0, -100); p.wait_for_timeout(200)
    size = p.evaluate("project.webcam.size")
    check(abs(size - 0.26) < 1e-9 and abs(float(p.evaluate("$('cam-size').value")) - 0.26) < 1e-9, f"scrolling up grows it {size}")

    # Drag the grip to resize.
    r = p.evaluate(RECT)
    g = p.evaluate("(() => { const b = document.getElementById('cam-grip').getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]; })()")
    p.mouse.move(g[0], g[1]); p.mouse.down(); p.mouse.move(g[0] + 40, g[1] + 40, steps=5); p.mouse.up(); p.wait_for_timeout(200)
    grown = p.evaluate("project.webcam.size")
    check(grown > 0.26 and grown <= 0.5, f"the grip resizes it {grown}")
    check(p.evaluate("project.webcam.corner") == "top-left", "resizing does not move it")

    # Arrow keys nudge it, and that never snaps.
    p.focus("#cam-handle")
    p.keyboard.press("ArrowRight"); p.keyboard.press("Shift+ArrowDown"); p.wait_for_timeout(200)
    pos = p.evaluate("project.webcam.position")
    check(pos is not None and pos[1] > 0.0, f"arrow keys move it by hand {pos}")

    # Picking a corner from the list puts it back.
    p.click("#tab-style")
    p.select_option("#cam-corner", "bottom-left")
    p.wait_for_timeout(200)
    check(p.evaluate("[project.webcam.corner, project.webcam.position ?? null]") == ["bottom-left", None], "choosing a corner clears the dragged spot")

    # Not in other layouts, and not while playing.
    p.select_option("#cam-layout", "split"); p.wait_for_timeout(400)
    check(p.evaluate("document.getElementById('cam-handle').hidden"), "no handle in the side by side layout")
    p.select_option("#cam-layout", "bubble"); p.wait_for_timeout(400)
    check(not p.evaluate("document.getElementById('cam-handle').hidden"), "the handle is back in the bubble layout")
    p.evaluate("setPlaying(true)"); p.wait_for_timeout(100)
    check(p.evaluate("document.getElementById('cam-handle').hidden"), "no handle while playing")
    p.evaluate("setPlaying(false)"); p.wait_for_timeout(300)
    p.evaluate("$('cam-visible').checked = false; readPanel(); refresh();"); p.wait_for_timeout(400)
    check(p.evaluate("document.getElementById('cam-handle').hidden"), "no handle with the camera hidden")

    check(not s.errors, f"no script errors {s.errors}")
    s.close()

    # A recording with no camera never shows it.
    s = Studio(pw, fx)
    p = s.open(); s.load_sample(); p.wait_for_timeout(400)
    check(p.evaluate("document.getElementById('cam-handle').hidden"), "no camera: no handle")
    s.close()

if failed:
    print(f"{len(failed)} failed")
    sys.exit(1)
print("camera drag ok")
