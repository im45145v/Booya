"""The editing controls reach the project: device frame, motion blur, music
(pick, tune, remove), muting one clip, and the caption look, position and capitals.

Run: python3 tests/e2e/test_editing_ui.py <fixture dir>
"""
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).parent))
from harness import Studio

SET = """([id, v]) => { const e = document.getElementById(id); if (e.type === 'checkbox') e.checked = v; else e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); e.dispatchEvent(new Event('change', { bubbles: true })); }"""


def test_frame_and_motion_blur_reach_the_renderer(pw, fx):
    s = Studio(pw, fx)
    try:
        s.open(); s.load_sample()
        p = s.page
        assert p.evaluate("project.style.frame || 'none'") == "none"
        p.evaluate(SET, ["frame-sel", "laptop"])
        p.evaluate(SET, ["motion-blur", True])
        assert p.evaluate("project.style.frame") == "laptop"
        assert p.evaluate("project.style.motion_blur") is True
        p.wait_for_timeout(700)
        last = [a for c, a in s.calls if c == "preview"][-1]
        assert last["project"]["style"]["frame"] == "laptop" and last["project"]["style"]["motion_blur"] is True
        assert [o for o in p.evaluate("[...document.getElementById('frame-sel').options].map(o => o.value)")] == ["none", "laptop", "browser", "phone"]
        assert not s.errors, s.errors
    finally:
        s.close()


def test_music_is_added_tuned_and_removed(pw, fx):
    song = str(fx / "audio_preview.m4a")
    s = Studio(pw, fx, extra={"pick_open": lambda self, a: song if a["name"] == "Music" else None, "audio_native_load": lambda self, a: {"available": True, "device": "Test output", "rate": 48000, "channels": 2, "voice_s": 12.0, "has_music": False, "has_audio": True, "peaks": [10, 200] * 100, "bucket_ms": 50.0, "faint": False}})
    try:
        s.open(); s.load_sample()
        p = s.page
        assert p.evaluate("project.music") is None
        assert p.evaluate("document.getElementById('mu-opts').hidden") is True
        p.click("#tab-style") if p.query_selector("#tab-style") else None
        p.evaluate("document.getElementById('mu-pick').click()")
        p.wait_for_function("project.music && project.music.path")
        m = p.evaluate("project.music")
        assert m["path"] == song and m["duck"] is True and 0 < m["volume"] <= 0.5 and m["looped"] is True
        assert p.evaluate("document.getElementById('mu-opts').hidden") is False
        assert p.text_content("#mu-name") == "audio_preview.m4a"
        p.evaluate(SET, ["mu-volume", "0.6"]); p.evaluate(SET, ["mu-duck-db", "18"]); p.evaluate(SET, ["mu-start", "2500"]); p.evaluate(SET, ["mu-duck", False])
        m = p.evaluate("project.music")
        assert (m["volume"], m["duck_db"], m["start_ms"], m["duck"]) == (0.6, 18, 2500, False), m
        assert p.text_content("#mu-start-out") == "2.5 s" and p.text_content("#mu-volume-out") == "60%"
        p.wait_for_timeout(700)
        assert any(c == "audio_native_music" for c, _ in s.calls), "the native player is told about music changes"
        # it is part of what is previewed and exported
        p.wait_for_timeout(300)
        last = [a for c, a in s.calls if c == "preview"][-1]
        assert last["project"]["music"]["path"] == song
        p.evaluate("document.getElementById('mu-remove').click()")
        assert p.evaluate("project.music") is None and p.evaluate("document.getElementById('mu-opts').hidden") is True
        assert not s.errors, s.errors
    finally:
        s.close()


def test_one_clip_can_be_muted_and_a_split_keeps_it(pw, fx):
    s = Studio(pw, fx)
    try:
        s.open(); s.load_sample()
        p = s.page
        p.evaluate("tl.selectedClip = 0; syncSelection()")
        assert p.evaluate("document.getElementById('clip-mute').checked") is False
        p.evaluate(SET, ["clip-mute", True])
        assert p.evaluate("project.clips[0].mute") is True
        p.evaluate("tl.playhead = tl.outDur() / 2; split()")
        flags = p.evaluate("project.clips.map(c => !!c.mute)")
        assert flags[:2] == [True, True], flags
        p.evaluate("tl.selectedClip = 1; syncSelection()")
        p.evaluate(SET, ["clip-mute", False])
        assert p.evaluate("project.clips.map(c => !!c.mute)")[:2] == [True, False]
        assert not s.errors, s.errors
    finally:
        s.close()



def test_an_export_shows_time_left_and_can_be_cancelled(pw, fx):
    out = str(fx / "cancel-test.mp4")
    s = Studio(pw, fx, extra={"pick_save": lambda self, a: out})
    try:
        s.open(); s.load_sample()
        p = s.page
        # A stand in export that runs until it is cancelled, like the real one.
        p.evaluate("""() => {
          const real = window.__TAURI__.core.invoke;
          window.__TAURI__.core.invoke = (cmd, args) => {
            if (cmd === 'export_video') return new Promise((_, reject) => { window.__rejectExport = reject; });
            if (cmd === 'cancel_export') { window.__cancelAsked = true; setTimeout(() => window.__rejectExport('Export cancelled'), 50); return Promise.resolve(null); }
            return real(cmd, args);
          };
        }""")
        p.evaluate("project.blur = []; $('ex-btn').click()")
        p.wait_for_function("typeof window.__rejectExport === 'function'")
        assert p.is_visible("#ex-cancel"), "Cancel shows while exporting"
        p.wait_for_timeout(1700)
        p.evaluate("window.__emit('export-progress', 0.25)")
        text = p.text_content("#ex-text")
        assert text.startswith("Rendering 25%") and "left" in text, text
        p.click("#ex-cancel")
        p.wait_for_function("document.getElementById('ex-text').textContent === 'Export cancelled'")
        assert p.evaluate("window.__cancelAsked") is True
        assert p.evaluate("document.getElementById('ex-cancel').hidden") is True
        assert p.evaluate("busy") is False and p.evaluate("$('ex-btn').disabled") is False, "the editor is usable again"
        assert not s.errors, s.errors
    finally:
        s.close()


def test_caption_look_position_and_capitals_reach_the_renderer(pw, fx):
    s = Studio(pw, fx)
    try:
        s.open(); s.load_sample()
        p = s.page
        # An older project has none of these, and the panel shows the defaults.
        assert p.evaluate("[$('cap-look').value, $('cap-pos').value, $('cap-upper').checked]") == ["box", "bottom", False]
        p.evaluate(SET, ["cap-look", "pop"]); p.evaluate(SET, ["cap-pos", "middle"]); p.evaluate(SET, ["cap-upper", True])
        st = p.evaluate("project.caption_style")
        assert (st["look"], st["position"], st["uppercase"]) == ("pop", "middle", True), st
        p.wait_for_timeout(700)
        last = [a for c, a in s.calls if c == "preview"][-1]["project"]["caption_style"]
        assert (last["look"], last["position"], last["uppercase"]) == ("pop", "middle", True), last
        assert [o for o in p.evaluate("[...document.getElementById('cap-look').options].map(o => o.value)")] == ["box", "outline", "pop"]
        # Undo brings the old look back into the panel.
        p.keyboard.press("Control+z"); p.wait_for_timeout(200)
        assert p.evaluate("$('cap-look').value") == p.evaluate("project.caption_style.look || 'box'")
        assert not s.errors, s.errors
    finally:
        s.close()

if __name__ == "__main__":
    fx = Path(sys.argv[1])
    with sync_playwright() as pw:
        for name, fn in list(globals().items()):
            if name.startswith("test_"):
                fn(pw, fx)
                print("ok", name)
