"""Drives the real DemoLens editor UI in Chrome with a stand in for the Tauri
shell. The media, project file and preview frames come from the real pipeline
(`cargo run -p demolens-core --example e2e_fixture`)."""
import functools, http.server, json, os, re, subprocess, threading, urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHROME = next((p for p in ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/opt/google/chrome/chrome"] if os.path.exists(p)), None)


class Handler(http.server.BaseHTTPRequestHandler):
    fx: Path = None
    log: list = None

    def log_message(self, *a):
        pass

    def _send_file(self, path: Path, ctype):
        size = path.stat().st_size
        rng = self.headers.get("Range")
        start, end, code = 0, size - 1, 200
        if rng:
            m = re.match(r"bytes=(\d*)-(\d*)", rng)
            if m:
                if m.group(1):
                    start = int(m.group(1))
                if m.group(2):
                    end = min(int(m.group(2)), size - 1)
                code = 206
            self.log.append(("range", path.name, rng))
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(end - start + 1))
        if code == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()
        with open(path, "rb") as f:
            f.seek(start)
            self.wfile.write(f.read(end - start + 1))

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        q = urllib.parse.parse_qs(u.query)
        ctypes = {".js": "text/javascript", ".css": "text/css", ".html": "text/html", ".svg": "image/svg+xml", ".png": "image/png", ".mp4": "video/mp4", ".m4a": "audio/mp4", ".wav": "audio/wav"}
        if u.path == "/asset":
            p = Path(q["path"][0])
            if self.fx not in p.resolve().parents:
                self.send_error(403)
                return
            return self._send_file(p, ctypes.get(p.suffix, "application/octet-stream"))
        name = "index.html" if u.path == "/" else u.path.lstrip("/")
        p = ROOT / "ui" / name
        if not p.is_file():
            self.send_error(404)
            return
        self._send_file(p, ctypes.get(p.suffix, "application/octet-stream"))


def out_to_src(project, o):
    acc = 0.0
    for c in project["clips"]:
        ln = (c["src_end_ms"] - c["src_start_ms"]) / c["speed"]
        if o <= acc + ln:
            return c["src_start_ms"] + (o - acc) * c["speed"]
        acc += ln
    return project["clips"][-1]["src_end_ms"]


def build_fixture(fx: Path, speech=False):
    fx.mkdir(parents=True, exist_ok=True)
    args = ["cargo", "run", "-q", "-p", "demolens-core", "--example", "e2e_fixture", "--", str(fx)] + (["speech"] if speech else [])
    subprocess.run(args, cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    return fx


class Studio:
    """One browser page on the editor, with a fake backend."""

    def __init__(self, pw, fx: Path, extra=None, headless=True, viewport=(1440, 900), touch=False):
        self.fx = fx.resolve()
        self.calls = []
        self.requests = []
        self.errors = []
        self.extra = extra or {}
        self.project = json.loads((self.fx / "project.json").read_text())
        self.project_path = str(self.fx / "e2e.demolens")
        Handler.fx = self.fx
        Handler.log = self.requests
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        launch = dict(headless=headless, args=["--autoplay-policy=no-user-gesture-required"] if False else [])
        if CHROME:
            launch["executable_path"] = CHROME
        self.browser = pw.chromium.launch(**launch)
        self.ctx = self.browser.new_context(viewport={"width": viewport[0], "height": viewport[1]}, has_touch=touch, is_mobile=False)
        self.page = self.ctx.new_page()
        self.page.on("pageerror", lambda e: self.errors.append(str(e) + " @ " + " | ".join((e.stack or "").splitlines()[1:3])))
        self.page.on("console", lambda m: self.errors.append("console: " + m.text) if m.type == "error" else None)
        self.page.expose_function("__py", self._dispatch)
        self.page.add_init_script(f"window.__ASSET_BASE = {json.dumps(self.base)}; localStorage.setItem('privacy-ack', '1');")
        self.page.add_init_script(path=str(ROOT / "tests" / "e2e" / "shim.js"))

    # ---- fake backend ----
    def _dispatch(self, cmd, args_json):
        args = json.loads(args_json)
        self.calls.append((cmd, args))
        if cmd in self.extra:
            r = self.extra[cmd](self, args)
        else:
            r = getattr(self, "cmd_" + cmd, lambda a: None)(args)
        return json.dumps(r)

    def cmd_make_sample(self, a):
        return {"project": self.project, "path": self.project_path}

    def cmd_open_project(self, a):
        return {"project": self.project, "path": self.project_path, "missing": [], "recovered": None}

    def cmd_preview(self, a):
        t = out_to_src(a["project"], a["outMs"])
        t = int(t // 500) * 500
        frames = sorted((self.fx / "frames").glob("f_*.png"), key=lambda p: int(p.stem[2:]))
        pick = [f for f in frames if int(f.stem[2:]) <= t] or frames
        import base64
        return "data:image/png;base64," + base64.b64encode(pick[-1].read_bytes()).decode()

    def cmd_raw_frame(self, a):
        import base64
        f = sorted((self.fx / "frames").glob("f_*.png"))[0]
        return {"url": "data:image/png;base64," + base64.b64encode(f.read_bytes()).decode(), "w": 960, "h": 540}

    def cmd_ffmpeg_status(self, a):
        return {"found": True, "path": "ffmpeg", "version": "7.0", "encoder": "libx264", "hw": []}

    def cmd_get_settings(self, a):
        return {}

    def cmd_recent_projects(self, a):
        return []

    def cmd_list_presets(self, a):
        return {"builtin": [], "user": []}

    def cmd_mic_list(self, a):
        return ["Default microphone"]

    def cmd_list_recordings(self, a):
        return []

    def cmd_ai_status(self, a):
        return {"features": [], "recommended_missing": [], "models_dir": "/models"}

    def cmd_background_recents(self, a):
        return []

    def cmd_cloud_status(self, a):
        return {"choice": {}, "providers": [], "list": []}

    def cmd_s3_get(self, a):
        return {"config": {}, "secret_set": False}

    def cmd_key_track(self, a):
        return []

    def cmd_get_shortcuts(self, a):
        return {"actions": [], "conflicts": []}

    def cmd_models_list(self, a):
        return {"dir": "/models", "models": []}

    def cmd_audio_preview(self, a):
        """What the real command does: a small m4a plus waveform peaks."""
        src = Path(a["source"])
        out = self.fx / "audio_preview.m4a"
        if not out.exists():
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(src), "-vn", "-ac", "2", "-ar", "48000", "-c:a", "aac", "-b:a", "128k", "-f", "mp4", str(out)], check=True)
        raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(src), "-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"], capture_output=True, check=True).stdout
        import array
        smp = array.array("h"); smp.frombytes(raw[: len(raw) // 2 * 2])
        n = 400
        pk = [max((abs(x) for x in smp[i:i + n]), default=0) for i in range(0, len(smp), n)]
        top = max(pk) or 1
        peaks = [round((v / top) ** 0.5 * 255) for v in pk]
        return {"has_audio": True, "path": str(out), "duration_ms": 12000, "bucket_ms": 50.0, "peaks": peaks, "faint": False}

    def cmd_audio_bytes(self, a):
        return {"__bytes": self.base + "/asset?path=" + urllib.parse.quote(a["path"], safe="")}

    # ---- helpers ----
    def open(self):
        self.page.goto(self.base + "/")
        self.page.wait_for_selector("#welcome")
        return self.page

    def load_sample(self):
        self.page.click("#w-sample")
        self.page.wait_for_function("document.body.classList.contains('has-project')")
        self.page.wait_for_timeout(600)

    def close(self):
        self.browser.close()
        self.server.shutdown()
