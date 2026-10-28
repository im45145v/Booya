"""The layout lane on the timeline and the layout controls in the side panel.
Run: python3 tests/e2e/test_scenes.py <fixture dir> [shots dir]"""
import json, sys
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


with sync_playwright() as pw:
    # --- a recording with no camera has no layout lane ---
    s = Studio(pw, fx)
    p = s.open(); s.load_sample()
    check(p.evaluate("tl.hasLayoutLane") is False, "no camera: no layout lane")
    check(p.evaluate("document.getElementById('cam-section').hidden") is True, "no camera: no camera panel")
    check(p.evaluate("tl.H") == 252, "no camera: the timeline is as tall as before")
    s.close()

    # --- with a camera ---
    s = Studio(pw, fx, viewport=(1440, 900))
    s.project["webcam"] = {"path": str(fx / "sample.mp4"), "visible": True, "corner": "bottom-right", "size": 0.24, "circle": True, "offset_ms": 0, "layout": "bubble", "mirror": False}
    p = s.open(); s.load_sample()
    check(p.evaluate("tl.hasLayoutLane") is True and p.evaluate("tl.H") == 282, "camera: there is a layout lane")
    lanes = p.evaluate("tl.L")
    check(lanes["layout"][1] < lanes["marks"][0] and lanes["zoom"][1] < lanes["layout"][0], f"the lanes do not overlap {lanes}")
    p.click("#tab-style")
    check(p.is_visible("#cam-layout") and p.is_visible("#lay-add"), "the layout controls are in the Style tab")
    check(p.evaluate("$('lay-kind').disabled && $('lay-del').disabled"), "nothing selected: no kind or delete")

    # base layout
    p.select_option("#cam-layout", "split")
    base = p.evaluate("[project.layouts.base, project.webcam.layout]")
    check(base == ["split", "side"], f"base layout split is saved (and the older setting follows) {base}")
    p.select_option("#cam-layout", "screen")
    check(p.evaluate("project.layouts.base") == "screen", "base layout screen")
    p.select_option("#cam-layout", "bubble")

    # the preview command gets the layouts
    p.wait_for_timeout(400)
    last = [a for c, a in s.calls if c == "preview"][-1]
    check(last["project"]["layouts"]["base"] == "bubble", "the preview is asked for with the layouts")

    # double click on the lane adds a segment
    D = p.evaluate("project.duration_ms")
    T0 = round(D * 0.25)
    geom = p.evaluate("(t0) => { const r = tl.c.getBoundingClientRect(), L = tl.L.layout; return { x0: r.left + tl.x(tl.srcToOut(t0)), y: r.top + (L[0] + L[1]) / 2 }; }", T0)
    p.mouse.dblclick(geom["x0"], geom["y"])
    p.wait_for_timeout(250)
    segs = p.evaluate("project.layouts.segments")
    check(len(segs) == 1 and segs[0]["kind"] == "camera" and abs(segs[0]["start_ms"] - T0) < 400 and 3500 < segs[0]["end_ms"] - segs[0]["start_ms"] <= 4000, f"double click adds a 4 s camera segment {segs}")
    check(p.evaluate("tl.selectedLayout") == 0 and not p.evaluate("$('lay-kind').disabled"), "the new segment is selected")
    check(p.input_value("#lay-kind") == "camera", "the kind box shows it")

    # change its kind
    p.select_option("#lay-kind", "split")
    check(p.evaluate("project.layouts.segments[0].kind") == "split", "kind can be changed")

    # drag it along
    start0 = segs[0]["start_ms"]
    mid = p.evaluate("""() => { const r = tl.c.getBoundingClientRect(), g = project.layouts.segments[0], L = tl.L.layout; return [r.left + (tl.x(tl.srcToOut(g.start_ms)) + tl.x(tl.srcToOut(g.end_ms))) / 2, r.top + (L[0] + L[1]) / 2]; }""")
    p.mouse.move(*mid); p.mouse.down(); p.mouse.move(mid[0] + 60, mid[1], steps=5); p.mouse.up()
    p.wait_for_timeout(200)
    moved = p.evaluate("project.layouts.segments[0]")
    check(moved["start_ms"] > start0 + 300 and abs((moved["end_ms"] - moved["start_ms"]) - (segs[0]["end_ms"] - segs[0]["start_ms"])) < 2, f"dragging moves a segment and keeps its length {moved}")

    # resize its right edge
    edge = p.evaluate("""() => { const r = tl.c.getBoundingClientRect(), g = project.layouts.segments[0], L = tl.L.layout; return [r.left + tl.x(tl.srcToOut(g.end_ms)), r.top + (L[0] + L[1]) / 2]; }""")
    p.mouse.move(*edge); p.mouse.down(); p.mouse.move(edge[0] - 40, edge[1], steps=5); p.mouse.up()
    p.wait_for_timeout(200)
    shorter = p.evaluate("project.layouts.segments[0]")
    check(shorter["end_ms"] < moved["end_ms"] - 200 and shorter["start_ms"] == moved["start_ms"], f"dragging the edge resizes it {shorter}")

    # a second one by button at the playhead; segments never overlap
    T1 = max(round(D * 0.75), shorter["end_ms"] + 800)
    p.evaluate(f"tl.playhead = tl.srcToOut({T1}); tl.onSeek(tl.playhead)")
    p.click("#lay-add")
    p.wait_for_timeout(200)
    segs = p.evaluate("project.layouts.segments")
    ok = len(segs) == 2 and segs[0]["end_ms"] <= segs[1]["start_ms"] and segs[1]["end_ms"] <= p.evaluate("project.duration_ms")
    check(ok, f"the Add button adds one at the playhead, inside the recording {segs}")
    # drag the second left over the first: clamped
    g2 = p.evaluate("""() => { const r = tl.c.getBoundingClientRect(), g = project.layouts.segments[1], L = tl.L.layout; return [r.left + (tl.x(tl.srcToOut(g.start_ms)) + tl.x(tl.srcToOut(g.end_ms))) / 2, r.top + (L[0] + L[1]) / 2]; }""")
    p.mouse.move(*g2); p.mouse.down(); p.mouse.move(g2[0] - 600, g2[1], steps=8); p.mouse.up()
    p.wait_for_timeout(200)
    segs = p.evaluate("project.layouts.segments")
    check(segs[0]["end_ms"] <= segs[1]["start_ms"] + 1, f"segments cannot be dragged over each other {segs}")
    # no room: the playhead inside a segment
    inside = segs[0]["start_ms"] + 100
    p.evaluate(f"tl.playhead = tl.srcToOut({inside}); tl.onSeek(tl.playhead)")
    p.click("#lay-add")
    check(p.evaluate("project.layouts.segments.length") == 2, "no second segment on top of an existing one")

    # blend and follow switches
    p.evaluate("$('lay-trans').value = 900; $('lay-trans').dispatchEvent(new Event('input', {bubbles: true}))")
    check(p.evaluate("project.layouts.transition_ms") == 900, "blend length is saved")
    p.evaluate("$('lay-follow').checked = false; $('lay-follow').dispatchEvent(new Event('input', {bubbles: true}))")
    check(p.evaluate("project.layouts.follow_cursor") is False, "follow cursor switch is saved")

    if shots:
        p.evaluate("tl.selectedLayout = 0; tl.onSelect()")
        p.wait_for_timeout(300)
        p.screenshot(path=str(shots / "layout-lane.png"))

    # delete with the button, then the key
    p.click("#lay-del")
    check(p.evaluate("project.layouts.segments.length") == 1 and p.evaluate("tl.selectedLayout") == -1, "Delete removes the selected segment")
    p.evaluate("tl.selectedLayout = 0; tl.onSelect()")
    p.evaluate("deleteSelected()")
    check(p.evaluate("project.layouts.segments.length") == 0, "the Delete key removes it too")

    # undo brings the segment back (it is part of the project history)
    errs = [e for e in s.errors if "favicon" not in e]
    check(not errs, f"no script errors {errs[:3]}")
    s.close()

if failed:
    print("FAILED:", *failed, sep="\n  ")
    sys.exit(1)
print("all layout controls ok")
