"""Nothing is clipped at the screen sizes people really have, and the two
splitters work. Run: python3 tests/e2e/test_layout.py <fixture dir> [shots dir]

Sizes are CSS pixels of the window. 1366x768 at 125% is what a 1366x768
laptop screen with Windows scaling gives: about 1093x614."""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright
sys.path.insert(0, str(Path(__file__).parent))
from harness import Studio

fx = Path(sys.argv[1])
shots = Path(sys.argv[2]) if len(sys.argv) > 2 else None
if shots:
    shots.mkdir(parents=True, exist_ok=True)

SIZES = [("1024x640", (1024, 640), 1), ("1280x720", (1280, 720), 1), ("1440x900", (1440, 900), 1), ("1366x768@125", (1093, 614), 1.25)]

CHECK = """() => {
  const vw = document.documentElement.clientWidth, vh = window.innerHeight, bad = [];
  const r = s => { const e = document.querySelector(s); return e && e.getBoundingClientRect(); };
  if (document.documentElement.scrollWidth > vw) bad.push('page scrolls sideways ' + document.documentElement.scrollWidth + ' > ' + vw);
  if (document.body.scrollWidth > vw) bad.push('body wider than the window');
  // The things the user must always reach.
  for (const s of ['#ex-top', '#rec-btn', '#settings-btn', '.inspector', '#timeline', '.transport', '.statusbar', '#preview-wrap']) {
    const b = r(s);
    if (!b) { bad.push(s + ' missing'); continue; }
    if (b.right > vw + 0.5 || b.left < -0.5) bad.push(s + ' is off the window sideways ' + JSON.stringify([b.left, b.right, vw]));
    if (b.bottom > vh + 0.5 || b.top < -0.5) bad.push(s + ' is off the window vertically ' + JSON.stringify([b.top, b.bottom, vh]));
  }
  // Every control in the toolbar and the transport is inside the window.
  for (const e of document.querySelectorAll('.toolbar button, .toolbar select, .transport button, .transport input')) {
    const b = e.getBoundingClientRect();
    if (b.width && (b.right > vw + 0.5 || b.left < -0.5)) bad.push('control off the window: ' + (e.id || e.className) + ' ' + Math.round(b.right));
  }
  // Nothing in the side panel sticks out of it or is cut at its right edge.
  const ins = r('.inspector');
  for (const pane of document.querySelectorAll('.pane')) {
    if (pane.offsetParent === null) continue;
    if (pane.scrollWidth > pane.clientWidth + 1) bad.push('panel scrolls sideways: ' + pane.id + ' ' + pane.scrollWidth + ' > ' + pane.clientWidth);
    for (const e of pane.querySelectorAll('*')) {
      const b = e.getBoundingClientRect();
      if (b.width && b.right > ins.right + 0.5 && getComputedStyle(e).position !== 'fixed') { bad.push('panel content sticks out: ' + (e.id || e.tagName + '.' + e.className) + ' ' + Math.round(b.right) + ' > ' + Math.round(ins.right)); break; }
    }
  }
  const pv = r('#preview-wrap');
  if (pv.width < 300 || pv.height < 120) bad.push('preview is too small ' + Math.round(pv.width) + 'x' + Math.round(pv.height));
  return { bad, panel: ins.width, preview: [Math.round(pv.width), Math.round(pv.height)], tl: r('.timeline-wrap').height, toolbar: r('.toolbar').height };
}"""

failed = []
with sync_playwright() as pw:
    for name, vp, dsf in SIZES:
        s = Studio(pw, fx, viewport=vp, dsf=dsf)
        p = s.open()
        p.screenshot(path=str(shots / f"layout-{name}-empty.png")) if shots else None
        s.load_sample()
        p.wait_for_function("audioState === 'ready'")
        for tab in ["#tab-magic", "#tab-style", "#tab-edit", "#tab-export"]:
            p.click(tab)
            p.wait_for_timeout(120)
            res = p.evaluate(CHECK)
            print(name, tab, {k: v for k, v in res.items() if k != "bad"}, res["bad"])
            if res["bad"]:
                failed.append((name, tab, res["bad"]))
            if shots:
                p.screenshot(path=str(shots / f"layout-{name}{tab.replace('#tab', '')}.png"))
        s.close()

    # Splitters, at a size where there is room to move.
    s = Studio(pw, fx, viewport=(1440, 900))
    p = s.open(); s.load_sample()
    box = lambda sel: p.evaluate("s => { const b = document.querySelector(s).getBoundingClientRect(); return [b.x, b.y, b.width, b.height]; }", sel)
    panel_w = lambda: box(".inspector")[2]
    tl_h = lambda: box(".timeline-wrap")[3]
    assert round(panel_w()) == 300 + 0 or abs(panel_w() - 300) <= 1, panel_w()
    assert abs(tl_h() - 252) <= 1, tl_h()

    def mouse_drag(sel, dx, dy):
        x, y, w, h = box(sel)
        cx, cy = x + w / 2, y + h / 2
        p.mouse.move(cx, cy); p.mouse.down(); p.mouse.move(cx + dx / 2, cy + dy / 2, steps=4); p.mouse.move(cx + dx, cy + dy, steps=4); p.mouse.up()
        p.wait_for_timeout(120)

    # hover cursor
    assert p.evaluate("getComputedStyle(document.getElementById('split-v')).cursor") == "col-resize"
    assert p.evaluate("getComputedStyle(document.getElementById('split-h')).cursor") == "row-resize"
    before = panel_w()
    mouse_drag("#split-v", -120, 0)
    assert abs(panel_w() - (before + 120)) <= 2, (before, panel_w())
    mouse_drag("#split-v", -900, 0)
    assert abs(panel_w() - 520) <= 1, ("panel max", panel_w())
    mouse_drag("#split-v", 900, 0)
    assert abs(panel_w() - 260) <= 1, ("panel min", panel_w())
    # saved, and back after a reload
    saved = p.evaluate("JSON.parse(localStorage.getItem('booya.layout.v1'))")
    assert saved["panel"] == 260, saved
    # horizontal
    pre_h = box("#preview-wrap")[3]
    mouse_drag("#split-h", 0, 100)
    assert abs(tl_h() - 152) <= 2, tl_h()
    assert box("#preview-wrap")[3] > pre_h + 90, "preview takes the room"
    mouse_drag("#split-h", 0, 500)
    assert abs(tl_h() - 96) <= 1, ("timeline min", tl_h())
    # a short timeline still scrolls to the lanes below
    assert p.evaluate("document.querySelector('.timeline-wrap').scrollHeight > document.querySelector('.timeline-wrap').clientHeight")
    mouse_drag("#split-h", 0, -900)
    assert tl_h() <= 252 + 1, tl_h()
    p.reload(); p.wait_for_selector("#welcome")
    assert abs(panel_w() - 260) <= 1, ("saved panel width after reload", panel_w())
    # double click resets both
    p.dblclick("#split-v"); p.dblclick("#split-h")
    p.wait_for_timeout(100)
    assert abs(panel_w() - 300) <= 1 and abs(tl_h() - 252) <= 1, (panel_w(), tl_h())
    assert p.evaluate("JSON.parse(localStorage.getItem('booya.layout.v1'))") == {"panel": 300, "tl": 252}
    # keyboard
    p.focus("#split-v"); p.keyboard.press("ArrowLeft")
    assert abs(panel_w() - 310) <= 1, panel_w()
    p.keyboard.press("Home")
    assert abs(panel_w() - 300) <= 1

    # touch: pointer events of type touch, drag and double tap
    touch = """([sel, dx, dy]) => {
      const el = document.querySelector(sel), b = el.getBoundingClientRect();
      const x = b.x + b.width / 2, y = b.y + b.height / 2;
      const ev = (t, X, Y) => el.dispatchEvent(new PointerEvent(t, { pointerId: 7, pointerType: 'touch', isPrimary: true, clientX: X, clientY: Y, bubbles: true, cancelable: true, button: 0 }));
      ev('pointerdown', x, y); ev('pointermove', x + dx / 2, y + dy / 2); ev('pointermove', x + dx, y + dy); ev('pointerup', x + dx, y + dy);
    }"""
    b4 = panel_w()
    p.wait_for_timeout(450)  # not a double tap with the click before
    p.evaluate(touch, ["#split-v", -80, 0]); p.wait_for_timeout(500)
    assert abs(panel_w() - (b4 + 80)) <= 2, ("touch drag", b4, panel_w())
    p.evaluate(touch, ["#split-v", 0, 0]); p.evaluate(touch, ["#split-v", 0, 0]); p.wait_for_timeout(100)
    assert abs(panel_w() - 300) <= 1, ("double tap resets", panel_w())

    # clamped when the window shrinks, and given back when it grows
    mouse_drag("#split-v", -300, 0)
    big = panel_w()
    p.set_viewport_size({"width": 1024, "height": 640}); p.wait_for_timeout(300)
    res = p.evaluate(CHECK)
    assert not res["bad"], res
    assert box("#preview-wrap")[2] >= 360, box("#preview-wrap")
    assert panel_w() <= big
    p.set_viewport_size({"width": 1440, "height": 900}); p.wait_for_timeout(300)
    assert abs(panel_w() - big) <= 2, ("choice comes back", big, panel_w())
    print("errors:", s.errors[:5])
    assert not s.errors, s.errors
    s.close()

assert not failed, failed
print("layout ok")
