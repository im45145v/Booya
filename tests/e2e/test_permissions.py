"""The permissions screen: shown on a Mac's first launch and when something
that was on goes off, turns rows green by itself, gates Continue on Screen
Recording, and has the Windows and Linux microphone variant.

Run: python3 tests/e2e/test_permissions.py <fixture dir> [screenshot dir]
"""
import re, sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).parent))
from harness import Studio

MAC = [
    dict(id="screen", title="Screen Recording", detail="Lets DemoLens see your screen so it can record it.", status="denied", required=True, note="macOS may ask you to quit and reopen DemoLens after you switch this on."),
    dict(id="mic", title="Microphone", detail="Records your voice with the video.", status="unknown", required=False, note=""),
    dict(id="input", title="Input Monitoring", detail="Shows the shortcuts you press (like Cmd+K) in the video. Never what you type.", status="unknown", required=False, note=""),
    dict(id="system-audio", title="System audio", detail="Records what your Mac plays. It uses the Screen Recording permission, and stays off until you switch it on in Record options.", status="denied", required=False, note=""),
]
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None


def backend(platform, items, needs=None):
    state = {"items": [dict(i) for i in items], "requests": []}

    def lst(self, a):
        it = state["items"]
        n = any(i["required"] and i["status"] != "granted" for i in it) if needs is None else needs
        return {"platform": platform, "needs_attention": n, "items": it}

    def req(self, a):
        state["requests"].append(a["id"])
        return None
    return state, {"permissions_list": lst, "permissions_request": req}


def shot(page, name):
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=str(SHOTS / name))


def test_first_launch_on_a_mac(pw, fx):
    state, extra = backend("macos", MAC)
    s = Studio(pw, fx, extra=extra, viewport=(1100, 860))
    try:
        s.page.add_init_script("localStorage.removeItem('perm-seen')")
        s.page.goto(s.base + "/")
        s.page.wait_for_selector("#perm-overlay")
        text = s.page.inner_text("#perm-overlay")
        assert "Welcome to DemoLens" in text
        assert "DemoLens never sends your recordings anywhere" in text
        assert "Accessibility permission" in text and "does not need" in text
        assert "Usage data" not in text and "telemetry" not in text.lower()
        assert text.count("REQUIRED") + text.count("Required") == 1
        assert s.page.is_disabled("#perm-go")
        shot(s.page, "permissions-first-run.png")
        # the button asks the system
        s.page.click("[data-perm=screen]")
        assert state["requests"] == ["screen"]
        # switching it on in System Settings turns the row green on its own, within a poll
        state["items"][0]["status"] = "granted"; state["items"][3]["status"] = "granted"
        s.page.wait_for_selector(".perm-row[data-state=granted] .perm-ok", timeout=3000)
        assert "Screen Recording enabled" in s.page.inner_text("#perm-overlay")
        s.page.wait_for_function("!document.getElementById('perm-go').disabled", timeout=3000)
        # an optional row left undone does not block Continue
        state["items"][1]["status"] = "granted"
        s.page.wait_for_timeout(1300)
        shot(s.page, "permissions-granted.png")
        s.page.click("#perm-go")
        assert s.page.query_selector("#perm-overlay") is None
        assert s.page.evaluate("localStorage.getItem('perm-seen')") == "1"
        assert not s.errors, s.errors
    finally:
        s.close()


def test_second_press_opens_the_settings_page_and_denied_mic_says_so(pw, fx):
    items = [dict(i) for i in MAC]
    items[1]["status"] = "denied"
    state, extra = backend("macos", items)
    s = Studio(pw, fx, extra=extra)
    try:
        s.page.add_init_script("localStorage.removeItem('perm-seen')")
        s.page.goto(s.base + "/")
        s.page.wait_for_selector("#perm-overlay")
        assert s.page.inner_text("[data-perm=mic]") == "Open System Settings"
        assert s.page.inner_text("[data-perm=screen]") == "Allow Screen Recording"
        s.page.click("[data-perm=screen]")
        s.page.wait_for_function("document.querySelector('[data-perm=screen]').textContent === 'Open System Settings'")
    finally:
        s.close()


def test_a_revoked_permission_brings_it_back_and_a_granted_mac_does_not_nag(pw, fx):
    granted = [dict(i, status="granted") for i in MAC]
    state, extra = backend("macos", granted, needs=False)
    s = Studio(pw, fx, extra=extra)
    try:
        s.page.add_init_script("if (!sessionStorage.getItem('x')) { sessionStorage.setItem('x', '1'); localStorage.setItem('perm-seen', '1'); localStorage.setItem('perm-granted', JSON.stringify(['screen', 'mic', 'input', 'system-audio'])); }")
        s.page.goto(s.base + "/")
        s.page.wait_for_selector("#welcome")
        s.page.wait_for_timeout(500)
        assert s.page.query_selector("#perm-overlay") is None
        # an update resets the system's grant for the microphone
        state["items"][1]["status"] = "denied"
        s.page.reload()
        s.page.wait_for_selector("#perm-overlay")
        assert "Microphone was switched off" in s.page.inner_text("#perm-overlay")
        assert not s.page.is_disabled("#perm-go")  # screen recording is still on
    finally:
        s.close()


def test_settings_tab_and_the_windows_variant(pw, fx):
    win = [dict(id="mic", title="Microphone", detail="Windows has a switch.", status="denied", required=False, note="")]
    state, extra = backend("windows", win, needs=True)
    s = Studio(pw, fx, extra=extra)
    try:
        s.page.goto(s.base + "/")
        s.page.wait_for_selector("#perm-overlay")
        text = s.page.inner_text("#perm-overlay")
        assert "Screen Recording" not in text and "Input Monitoring" not in text
        assert "Let desktop apps access your microphone" in text
        shot(s.page, "permissions-windows.png")
        s.page.click("[data-perm=mic]")
        assert state["requests"] == ["mic"]
        assert not s.page.is_disabled("#perm-go")  # nothing is required on Windows
        s.page.click("#perm-go")
        # Settings > Permissions shows the same rows, without the welcome
        state["items"][0]["status"] = "granted"
        s.page.evaluate("showSettings('permissions')")
        s.page.wait_for_selector("#perm-body .perm-ok", timeout=4000)
        body = s.page.inner_text("#perm-body")
        assert "Microphone enabled" in body and "Welcome" not in body
        assert "never sends your recordings anywhere" in body
        assert not s.errors, s.errors
    finally:
        s.close()


def test_no_dashes_in_the_copy(pw, fx):
    src = (Path(__file__).resolve().parents[2] / "ui" / "permissions.js").read_text()
    assert "\u2014" not in src and "\u2013" not in src
    assert re.search(r"IOHID|telemetry", src) is None


if __name__ == "__main__":
    fx = Path(sys.argv[1])
    with sync_playwright() as pw:
        for name, fn in list(globals().items()):
            if name.startswith("test_"):
                fn(pw, fx)
                print("ok", name)
