"""Screenshots of the editor for the release notes and the CI artifact.
Usage: python3 tests/e2e/shots.py <fixture dir> <out dir>"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright
sys.path.insert(0, str(Path(__file__).parent))
from harness import Studio

fx = Path(sys.argv[1]); out = Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
with sync_playwright() as pw:
    s = Studio(pw, fx)
    s.open()
    s.page.screenshot(path=str(out / "01-empty.png"))
    s.load_sample()
    s.page.wait_for_function("audioState === 'ready'")
    s.page.screenshot(path=str(out / "02-loaded.png"))
    s.page.evaluate("tl.selectedZoom = 0; tl.draw(); tl.onSelect()")
    s.page.wait_for_timeout(300)
    s.page.screenshot(path=str(out / "03-zoom-selected.png"))
    s.page.click("#tab-style"); s.page.wait_for_timeout(200)
    s.page.screenshot(path=str(out / "04-style-tab.png"))
    s.page.click("#keys-btn"); s.page.wait_for_timeout(300)
    s.page.screenshot(path=str(out / "05-shortcuts.png"))
    s.page.keyboard.press("Escape")
    print("errors:", s.errors[:8])
    assert not s.errors, s.errors
    s.close()
