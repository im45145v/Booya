from playwright.sync_api import sync_playwright
import os
def shots(jobs, scale=1):
    """jobs: list of (html_path, png_path, w, h)."""
    with sync_playwright() as p:
        b = p.chromium.launch(executable_path='/usr/bin/google-chrome')
        for html, out, wd, ht in jobs:
            pg = b.new_page(viewport={'width': wd, 'height': ht}, device_scale_factor=scale)
            pg.goto('file://' + os.path.abspath(html)); pg.wait_for_load_state('networkidle'); pg.evaluate('document.fonts.ready'); pg.wait_for_timeout(150)
            os.makedirs(os.path.dirname(out), exist_ok=True)
            pg.screenshot(path=out, clip={'x': 0, 'y': 0, 'width': wd, 'height': ht})
            pg.close()
        b.close()
