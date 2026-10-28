"""The editor must be able to be heard: the preview sound loads, plays with a
moving clock, is not muted, and says so when it cannot play.

Run: python3 tests/e2e/test_audio.py <fixture dir>   (build it with the e2e_fixture example)
"""
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).parent))
from harness import Studio

STATE = "() => ({ t: pa.currentTime, paused: pa.paused, muted: pa.muted, vol: pa.volume, ready: pa.readyState, err: pa.error && pa.error.message, state: audioState, chip: document.getElementById('audio-chip').textContent })"
ENERGY = """async () => {
  const ctx = new AudioContext(); await ctx.resume();
  const an = ctx.createAnalyser(); an.fftSize = 2048;
  ctx.createMediaStreamSource(pa.captureStream()).connect(an);
  const buf = new Float32Array(an.fftSize); let peak = 0;
  for (let i = 0; i < 20; i++) { await new Promise(r => setTimeout(r, 60)); an.getFloatTimeDomainData(buf); peak = Math.max(peak, ...buf.map(Math.abs)); }
  return peak;
}"""


def wait_state(page, want, timeout=10000):
    page.wait_for_function(f"audioState === {want!r}", timeout=timeout)


def test_plays_and_is_audible(pw, fx):
    s = Studio(pw, fx)
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "ready")
        assert s.page.text_content("#audio-chip").startswith("Sound on")
        s.page.click("#play")
        s.page.wait_for_timeout(1800)
        # read the sound clock and the playhead in the same instant
        both = s.page.evaluate("() => { const st = (" + STATE + ")(); st.tl = tl.outToSrc(tl.playhead) / 1000; return st; }")
        st, tl_src = both, both["tl"]
        print("playing:", st)
        assert not st["paused"], st
        assert st["t"] > 0.8, f"the sound clock did not move: {st}"
        assert not st["muted"] and st["vol"] > 0, st
        assert st["err"] is None, st
        peak = s.page.evaluate(ENERGY)
        print("signal peak:", peak)
        assert peak > 0.01, "the sound element is playing silence"
        # the sound follows the picture
        assert abs(tl_src - st["t"]) < 1.0, (tl_src, st)
        # Mute and the volume control are visible and work, only for the preview
        s.page.click("#vol-btn")
        assert s.page.evaluate("pa.muted") is True
        assert s.page.evaluate("project.audio.mute") is False, "the export setting must not change"
        s.page.click("#vol-btn")
        assert s.page.evaluate("pa.muted") is False
        s.page.fill("#vol", "0.4")
        s.page.dispatch_event("#vol", "input")
        assert abs(s.page.evaluate("pa.volume") - 0.4) < 0.01
        s.page.click("#play")
        assert s.page.evaluate("pa.paused") is True
        assert s.page.locator("#timeline").evaluate("c => c.clientHeight") > 200
        assert not s.errors, s.errors
    finally:
        s.close()


def test_asset_fallback_when_bytes_fail(pw, fx):
    s = Studio(pw, fx, extra={"audio_bytes": lambda self, a: {"__error": "ipc failed"}})
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "ready")
        s.page.click("#play"); s.page.wait_for_timeout(1200)
        st = s.page.evaluate(STATE)
        assert st["t"] > 0.5 and not st["paused"], st
    finally:
        s.close()


def test_says_so_when_the_sound_cannot_load(pw, fx):
    bad = fx / "not-audio.m4a"
    bad.write_bytes(b"this is not an audio file" * 100)
    pv = lambda self, a: {**Studio.cmd_audio_preview(self, a), "path": str(bad)}
    s = Studio(pw, fx, extra={"audio_preview": pv, "audio_preview_wav": lambda self, a: {"__error": "no wav either"}})
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "error", 12000)
        assert "unavailable" in s.page.text_content("#audio-chip").lower()
        assert "sound" in s.page.text_content("#status").lower()
        s.page.click("#play")  # editing still works without sound
        s.page.wait_for_timeout(500)
    finally:
        s.close()


def test_falls_back_to_wav_when_the_web_view_cannot_decode_the_aac(pw, fx):
    # What a WKWebView that refuses the AAC file looks like: it cannot load it.
    bad = fx / "undecodable.m4a"
    bad.write_bytes(b"\x00\x00\x00\x18ftypM4A " + b"\x00" * 4000)
    pv = lambda self, a: {**Studio.cmd_audio_preview(self, a), "path": str(bad)}
    s = Studio(pw, fx, extra={"audio_preview": pv})
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "ready", 15000)
        assert s.page.evaluate("audioIsWav") is True
        s.page.click("#play")
        s.page.wait_for_timeout(1800)
        st = s.page.evaluate(STATE)
        assert not st["paused"] and st["t"] > 0.8 and not st["muted"], st
        assert s.page.evaluate(ENERGY) > 0.01, "the WAV copy is playing silence"
        assert not s.errors, s.errors
    finally:
        s.close()


def test_the_first_click_unlocks_sound_and_leaves_it_unmuted(pw, fx):
    s = Studio(pw, fx)
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "ready")
        s.page.click("#vol-btn"); s.page.click("#vol-btn")  # any click is a gesture
        s.page.wait_for_timeout(300)
        st = s.page.evaluate(STATE)
        assert st["paused"] and not st["muted"] and st["vol"] > 0, st
        assert s.page.evaluate("audioPrimed") is True
    finally:
        s.close()


def test_no_sound_in_the_recording(pw, fx):
    pv = lambda self, a: {"has_audio": False, "path": None, "duration_ms": 12000, "bucket_ms": 50.0, "peaks": [], "faint": False}
    s = Studio(pw, fx, extra={"audio_preview": pv})
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "none")
        assert s.page.text_content("#audio-chip") == "No sound in this recording"
        s.page.click("#play"); s.page.wait_for_timeout(500)
        assert not s.errors, s.errors
    finally:
        s.close()


def test_native_player_is_driven_and_corrected(pw, fx):
    """When the app can play sound itself, the page only drives it: the web view's audio element stays silent, and a drift over 60 ms is corrected with a seek."""
    import time
    clock = {"t0": None, "off": 0.0}

    def load(self, a):
        return {"available": True, "device": "Test output", "rate": 48000, "channels": 2, "voice_s": 12.0, "has_music": False, "has_audio": True, "peaks": [10, 200, 90, 255] * 60, "bucket_ms": 50.0, "faint": False}

    def play(self, a):
        clock["t0"] = time.time() - a["voiceS"]
        return None

    def seek(self, a):
        clock["t0"] = time.time() - a["voiceS"]
        clock["off"] = 0.0
        return None

    def state(self, a):
        pos = (time.time() - clock["t0"]) if clock["t0"] else 0.0
        return {"playing": True, "voice_s": pos + clock["off"], "music_s": pos, "rate": 1.0, "error": None}

    s = Studio(pw, fx, extra={"audio_native_load": load, "audio_play": play, "audio_state": state, "audio_seek": seek})
    try:
        s.open(); s.load_sample()
        wait_state(s.page, "ready")
        assert s.page.text_content("#audio-chip").startswith("Sound on")
        s.page.click("#play")
        s.page.wait_for_timeout(1200)
        assert any(c == "audio_play" for c, _ in s.calls), [c for c, _ in s.calls][-12:]
        st = s.page.evaluate("() => ({ paused: pa.paused, muted: pa.muted })")
        assert st["paused"] or st["muted"], st
        assert not any(c == "audio_seek" for c, _ in s.calls), ("in sync: no seek", [(c, a) for c, a in s.calls if c.startswith("audio_")][:12])
        clock["off"] = 0.5  # the player drifts half a second ahead
        s.page.wait_for_timeout(1500)
        seeks = [a for c, a in s.calls if c == "audio_seek"]
        assert len(seeks) == 1, ("a 500 ms drift is re-seeked once, then stays in sync", seeks)
        s.page.click("#play")
        s.page.wait_for_timeout(300)
        assert any(c == "audio_pause" for c, _ in s.calls)
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
