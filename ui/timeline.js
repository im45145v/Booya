// Timeline: a ruler, a clip lane (output time), a zoom lane (source time
// mapped to where it lands in the output), notes (callouts and shapes), blur
// boxes, the keystrokes that show on screen, and a speech lane. Drag anywhere on
// the ruler or clips to scrub; drag a zoom segment to move it or its edges to
// resize it; Ctrl/Cmd + wheel zooms the timeline, wheel pans it.
const FILLER = /^(u+m+|u+h+|uhm|erm|er|ah|hmm+|mm)\W*$/i;

class Timeline {
  constructor(canvas, { onChange, onSeek, onSelect }) {
    this.c = canvas; this.ctx = canvas.getContext('2d');
    this.onChange = onChange; this.onSeek = onSeek; this.onSelect = onSelect || (() => {});
    this.project = null; this.playhead = 0; this.selectedZoom = -1; this.selectedClip = 0; this.drag = null;
    this.view = [0, 1]; // visible output range in ms; [0, dur] when fitted
    this.fitted = true;
    this.gutter = 86; this.selectedLayout = -1;
    this.selItem = null; this.keyTrack = [];
    // Waveform of the sound: { data: 0..255 per bucket, bucket: ms } and what state the sound is in.
    this.peaks = null; this.audioState = 'none';
    // Edge grab size in px: bigger for a finger or pen than for a mouse.
    this.slop = 6; this.touches = new Map(); this.pinch = null; this.lastTap = null; this.lastDbl = 0;
    canvas.addEventListener('pointerdown', e => this.pdown(e));
    canvas.addEventListener('pointermove', e => this.hover(e));
    window.addEventListener('pointermove', e => this.pmove(e));
    window.addEventListener('pointerup', e => this.pup(e));
    window.addEventListener('pointercancel', e => this.pup(e));
    canvas.addEventListener('dblclick', e => this.dbl(e));
    canvas.addEventListener('wheel', e => this.wheel(e), { passive: false });
    new ResizeObserver(() => this.draw()).observe(canvas);
  }
  // Pointer plumbing for touch: two fingers pinch to zoom and slide to pan,
  // a double tap adds a zoom like a double click, edges are easier to hit.
  pdown(e) {
    this.slop = e.pointerType === 'mouse' ? 6 : 16;
    if (e.pointerType === 'touch') {
      this.touches.set(e.pointerId, [e.clientX, e.clientY]);
      if (this.touches.size === 2) {
        this.drag = null;
        const [a, b] = [...this.touches.values()];
        this.pinch = { d: Math.hypot(a[0] - b[0], a[1] - b[1]) || 1, cx: (a[0] + b[0]) / 2 };
        return;
      }
      const now = performance.now(), l = this.lastTap;
      this.lastTap = { t: now, x: e.clientX, y: e.clientY };
      if (l && now - l.t < 350 && Math.hypot(e.clientX - l.x, e.clientY - l.y) < 30) { this.lastTap = null; this.dbl(e); return; }
    }
    this.down(e);
  }
  pmove(e) {
    if (this.touches.has(e.pointerId)) {
      this.touches.set(e.pointerId, [e.clientX, e.clientY]);
      if (this.pinch && this.touches.size >= 2 && this.project) {
        const [a, b] = [...this.touches.values()], d = Math.hypot(a[0] - b[0], a[1] - b[1]) || 1, cx = (a[0] + b[0]) / 2;
        const r = this.c.getBoundingClientRect();
        if (Math.abs(d / this.pinch.d - 1) > 0.02) { this.zoomBy(d / this.pinch.d, this.ms(cx - r.left)); this.pinch.d = d; }
        const span = this.view[1] - this.view[0];
        if (this.view[1] - this.view[0] < this.outDur()) this.pan(-((cx - this.pinch.cx) / this.width()) * span);
        this.pinch.cx = cx;
        return;
      }
    }
    this.move(e);
  }
  pup(e) {
    const was = this.touches.delete(e.pointerId);
    if (this.pinch) { if (this.touches.size < 2) this.pinch = null; return; }
    void was;
    this.up();
  }
  setProject(p) { this.project = p; this.fit(); }
  outDur() { return this.project ? this.project.clips.reduce((a, c) => a + (c.src_end_ms - c.src_start_ms) / c.speed, 0) : 0; }
  srcToOut(src) {
    let acc = 0;
    for (const c of this.project.clips) {
      if (src < c.src_start_ms) return acc;
      if (src <= c.src_end_ms) return acc + (src - c.src_start_ms) / c.speed;
      acc += (c.src_end_ms - c.src_start_ms) / c.speed;
    }
    return acc;
  }
  outToSrc(out) {
    let acc = 0;
    for (const c of this.project.clips) {
      const len = (c.src_end_ms - c.src_start_ms) / c.speed;
      if (out <= acc + len) return c.src_start_ms + (out - acc) * c.speed;
      acc += len;
    }
    const last = this.project.clips[this.project.clips.length - 1];
    return last ? last.src_end_ms : 0;
  }
  // View (zoom and pan of the timeline itself).
  fit() { this.fitted = true; this.view = [0, Math.max(1, this.outDur())]; this.draw(); }
  zoomBy(f, centerMs) {
    const dur = Math.max(1, this.outDur()), [a, b] = this.view;
    const c = centerMs ?? (this.playhead >= a && this.playhead <= b ? this.playhead : (a + b) / 2);
    let span = Math.min(dur, Math.max(500, (b - a) / f));
    let na = c - (c - a) * (span / (b - a));
    na = Math.max(0, Math.min(dur - span, na));
    this.view = [na, na + span]; this.fitted = span >= dur; this.draw();
  }
  pan(ms) {
    const dur = this.outDur(), span = this.view[1] - this.view[0];
    const a = Math.max(0, Math.min(dur - span, this.view[0] + ms));
    this.view = [a, a + span]; this.draw();
  }
  follow() { // keep the playhead visible during playback
    const [a, b] = this.view, span = b - a;
    if (this.playhead > b || this.playhead < a) this.pan(this.playhead - a - span * 0.1);
  }
  // The lanes. A layout lane (per-segment camera layouts) is there when the
  // recording has a camera, and pushes the lanes below it down.
  get hasLayoutLane() { return !!(this.project && this.project.webcam); }
  get H() { return this.hasLayoutLane ? 282 : 252; }
  get L() {
    const o = this.hasLayoutLane ? 30 : 0;
    const L = { ruler: [0, 22], clips: [28, 62], zoom: [68, 96] };
    if (o) L.layout = [102, 126];
    return Object.assign(L, { marks: [102 + o, 128 + o], blur: [134 + o, 156 + o], keys: [162 + o, 182 + o], words: [188 + o, 210 + o], audio: [216 + o, 244 + o] });
  }
  layoutAt(x) {
    const segs = (this.project && this.project.layouts && this.project.layouts.segments) || [];
    for (let i = 0; i < segs.length; i++) {
      const z = segs[i], a = this.x(this.srcToOut(z.start_ms)), b = this.x(this.srcToOut(z.end_ms));
      if (Math.abs(x - a) < this.slop) return { i, edge: 'start' };
      if (Math.abs(x - b) < this.slop) return { i, edge: 'end' };
      if (x > a && x < b) return { i, edge: 'move' };
    }
    return null;
  }
  width() { return this.c.clientWidth; }
  x(ms) { const [a, b] = this.view; return this.gutter + ((ms - a) / Math.max(1, b - a)) * (this.width() - this.gutter - 8); }
  ms(x) { const [a, b] = this.view; return Math.max(0, Math.min(this.outDur(), a + ((x - this.gutter) / (this.width() - this.gutter - 8)) * (b - a))); }
  pos(e) { const r = this.c.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; }
  lane(y) { for (const [k, [a, b]] of Object.entries(this.L)) if (y >= a - 3 && y <= b + 3) return k; return null; }
  zoomAt(x) {
    if (!this.project) return null;
    for (let i = 0; i < this.project.zoom.length; i++) {
      const z = this.project.zoom[i], a = this.x(this.srcToOut(z.start_ms)), b = this.x(this.srcToOut(z.end_ms));
      if (Math.abs(x - a) < this.slop) return { i, edge: 'start' };
      if (Math.abs(x - b) < this.slop) return { i, edge: 'end' };
      if (x > a && x < b) return { i, edge: 'move' };
    }
    return null;
  }
  // Timed items in the notes and blur lanes (source time), each in one of two rows when they overlap.
  items(lane) {
    const p = this.project, out = [];
    if (lane === 'marks') {
      (p.callouts || []).forEach(o => out.push({ o, arr: p.callouts, label: o.text, color: '--tl-note' }));
      (p.shapes || []).forEach(o => out.push({ o, arr: p.shapes, label: SHAPE_LABEL[o.kind] || o.kind, hex: o.color }));
    } else if (lane === 'blur') (p.blur || []).forEach(o => out.push({ o, arr: p.blur, label: 'Blur', color: '--tl-blur' }));
    out.sort((a, b) => a.o.start_ms - b.o.start_ms);
    const ends = [-1, -1]; let overlap = false;
    out.forEach(it => { let r = ends.findIndex(e => e <= it.o.start_ms + 1); if (r < 0) { r = 1; overlap = true; } ends[r] = it.o.end_ms; it.row = r; });
    out.forEach(it => (it.rows = overlap ? 2 : 1));
    return out;
  }
  itemBox(lane, it) {
    const [a, b] = this.L[lane], h = (b - a) / it.rows;
    return [this.x(this.srcToOut(it.o.start_ms)), this.x(this.srcToOut(it.o.end_ms)), a + it.row * h, a + (it.row + 1) * h - (it.rows > 1 ? 1 : 0)];
  }
  itemAt(lane, x, y) {
    for (const it of this.items(lane).reverse()) {
      const [a, b, ya, yb] = this.itemBox(lane, it);
      if (y < ya - 1 || y > yb + 1) continue;
      if (Math.abs(x - a) < this.slop) return { it, edge: 'start' };
      if (Math.abs(x - b) < this.slop) return { it, edge: 'end' };
      if (x > a && x < b) return { it, edge: 'move' };
    }
    return null;
  }
  hover(e) {
    if (!this.project || this.drag || e.pointerType !== 'mouse') return;
    const [x, y] = this.pos(e), lane = this.lane(y);
    let cur = 'text';
    if (lane === 'zoom') { const h = this.zoomAt(x); cur = h ? (h.edge === 'move' ? 'grab' : 'ew-resize') : 'copy'; }
    if (lane === 'layout') { const h = this.layoutAt(x); cur = h ? (h.edge === 'move' ? 'grab' : 'ew-resize') : 'copy'; }
    if (lane === 'marks' || lane === 'blur') { const h = this.itemAt(lane, x, y); cur = h ? (h.edge === 'move' ? 'grab' : 'ew-resize') : 'default'; }
    this.c.style.cursor = cur;
  }
  down(e) {
    if (!this.project) return;
    this.c.focus();
    const [x, y] = this.pos(e), lane = this.lane(y);
    if (lane === 'zoom') {
      const h = this.zoomAt(x);
      if (h) {
        const z = this.project.zoom[h.i];
        this.selectedZoom = h.i; this.selItem = null; this.selectedLayout = -1;
        this.drag = { kind: 'zoom', ...h, grab: this.outToSrc(this.ms(x)) - z.start_ms, len: z.end_ms - z.start_ms, moved: false };
        if (h.edge === 'move') this.c.style.cursor = 'grabbing';
        this.onSelect(); this.draw(); return;
      }
      this.selectedZoom = -1; this.onSelect();
    }
    if (lane === 'layout') {
      const h = this.layoutAt(x);
      if (h) {
        const z = this.project.layouts.segments[h.i];
        this.selectedLayout = h.i; this.selectedZoom = -1; this.selItem = null;
        this.drag = { kind: 'layout', ...h, grab: this.outToSrc(this.ms(x)) - z.start_ms, len: z.end_ms - z.start_ms, moved: false };
        if (h.edge === 'move') this.c.style.cursor = 'grabbing';
        this.onSelect(); this.draw(); return;
      }
      this.selectedLayout = -1; this.onSelect();
    }
    if (lane === 'marks' || lane === 'blur') {
      const h = this.itemAt(lane, x, y);
      if (h) {
        this.selItem = h.it.o; this.selectedZoom = -1; this.selectedLayout = -1;
        this.drag = { kind: 'item', it: h.it, edge: h.edge, grab: this.outToSrc(this.ms(x)) - h.it.o.start_ms, len: h.it.o.end_ms - h.it.o.start_ms, moved: false };
        if (h.edge === 'move') this.c.style.cursor = 'grabbing';
        this.onSelect(); this.draw(); return;
      }
      this.selItem = null; this.onSelect();
    } else if (lane) { this.selItem = null; }
    if (x < this.gutter) return;
    this.drag = { kind: 'scrub' };
    this.scrubTo(x);
  }
  scrubTo(x) {
    const t = this.ms(x); let acc = 0;
    this.project.clips.forEach((c, i) => { const len = (c.src_end_ms - c.src_start_ms) / c.speed; if (t >= acc && t <= acc + len) this.selectedClip = i; acc += len; });
    this.playhead = t; this.onSeek(t); this.onSelect(); this.draw();
  }
  move(e) {
    if (!this.drag) return;
    const [x] = this.pos(e);
    if (this.drag.kind === 'scrub') return this.scrubTo(x);
    if (this.drag.kind === 'item') {
      const o = this.drag.it.o, src = this.outToSrc(this.ms(x)), dur = this.project.duration_ms;
      this.drag.moved = true;
      if (this.drag.edge === 'start') o.start_ms = Math.max(0, Math.min(src, o.end_ms - 200));
      else if (this.drag.edge === 'end') o.end_ms = Math.min(dur, Math.max(src, o.start_ms + 200));
      else { const s = Math.max(0, Math.min(dur - this.drag.len, src - this.drag.grab)); o.start_ms = s; o.end_ms = s + this.drag.len; }
      return this.draw();
    }
    const list = this.drag.kind === 'layout' ? this.project.layouts.segments : this.project.zoom;
    const z = list[this.drag.i], src = this.outToSrc(this.ms(x));
    this.drag.moved = true;
    // Zooms (and layout changes) may not overlap: clamp against the neighbours on either side.
    const others = list.filter((_, j) => j !== this.drag.i);
    const lo = Math.max(0, ...others.filter(o => o.end_ms <= z.start_ms + 1).map(o => o.end_ms));
    const hi = Math.min(this.project.duration_ms, ...others.filter(o => o.start_ms >= z.end_ms - 1).map(o => o.start_ms));
    if (this.drag.edge === 'start') z.start_ms = Math.max(lo, Math.min(src, z.end_ms - 200));
    else if (this.drag.edge === 'end') z.end_ms = Math.min(hi, Math.max(src, z.start_ms + 200));
    else {
      const s = Math.max(lo, Math.min(hi - this.drag.len, src - this.drag.grab));
      z.start_ms = s; z.end_ms = s + this.drag.len;
    }
    this.draw();
  }
  up() {
    if (!this.drag) return;
    const d = this.drag; this.drag = null;
    if (d.kind === 'item') { this.c.style.cursor = 'grab'; if (d.moved) this.onChange(); else { this.playhead = this.srcToOut(d.it.o.start_ms); this.onSeek(this.playhead); } this.draw(); return; }
    if (d.kind === 'layout') {
      this.c.style.cursor = 'grab';
      const segs = this.project.layouts.segments, cur = segs[d.i];
      if (d.moved) { segs.sort((a, b) => a.start_ms - b.start_ms); this.selectedLayout = segs.indexOf(cur); this.onChange(); }
      else { this.playhead = this.srcToOut(cur.start_ms); this.onSeek(this.playhead); }
      this.draw();
    }
    if (d.kind === 'zoom') {
      this.c.style.cursor = 'grab';
      if (d.moved) { this.project.zoom.sort((a, b) => a.start_ms - b.start_ms); this.onChange(); }
      else { this.playhead = this.srcToOut(this.project.zoom[d.i].start_ms); this.onSeek(this.playhead); }
      this.draw();
    }
  }
  dbl(e) {
    if (!this.project) return;
    // A double tap raises both our own detection and the browser's dblclick.
    const now = performance.now(); if (now - this.lastDbl < 500) return; this.lastDbl = now;
    const [x, y] = this.pos(e);
    if (this.lane(y) === 'layout' && !this.layoutAt(x)) {
      this.addLayout(this.outToSrc(this.ms(x)));
      return;
    }
    if (this.lane(y) === 'zoom' && !this.zoomAt(x)) {
      const at = this.outToSrc(this.ms(x));
      this.project.zoom.push({ start_ms: at, end_ms: Math.min(this.project.duration_ms, at + 2000), scale: 2, follow_cursor: true, focus: [this.project.source_width / 2, this.project.source_height / 2], reason: 'manual' });
      this.project.zoom.sort((a, b) => a.start_ms - b.start_ms);
      this.selectedZoom = this.project.zoom.findIndex(z => z.start_ms === at);
      this.onSelect(); this.onChange(); this.draw();
    }
  }
  // Adds a layout change at `at` (source ms), as long as it fits between its
  // neighbours. Returns the new index, or -1.
  addLayout(at, kind) {
    const p = this.project; if (!p || !p.webcam) return -1;
    const L = p.layouts = p.layouts || { segments: [] }; const segs = L.segments = L.segments || [];
    if (segs.some(g => at >= g.start_ms && at < g.end_ms)) return -1;
    const next = Math.min(p.duration_ms, ...segs.filter(g => g.start_ms > at).map(g => g.start_ms));
    const end = Math.min(next, at + 4000);
    if (end - at < 200) return -1;
    const seg = { start_ms: at, end_ms: end, kind: kind || 'camera' };
    segs.push(seg); segs.sort((a, b) => a.start_ms - b.start_ms);
    this.selectedLayout = segs.indexOf(seg); this.selectedZoom = -1; this.selItem = null;
    this.onSelect(); this.onChange(); this.draw();
    return this.selectedLayout;
  }
  wheel(e) {
    if (!this.project) return;
    // A short timeline panel scrolls up and down on a plain vertical wheel.
    const wrap = this.c.parentElement;
    if (!e.ctrlKey && !e.metaKey && !e.shiftKey && Math.abs(e.deltaY) > Math.abs(e.deltaX) && wrap && wrap.scrollHeight > wrap.clientHeight + 1) { wrap.scrollTop += e.deltaY; e.preventDefault(); return; }
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) { const [x] = this.pos(e); this.zoomBy(e.deltaY < 0 ? 1.25 : 0.8, this.ms(x)); }
    else this.pan(((e.deltaX || e.deltaY) / this.width()) * (this.view[1] - this.view[0]));
  }
  color(name) { return getComputedStyle(document.documentElement).getPropertyValue(name).trim(); }
  draw() {
    const dpr = window.devicePixelRatio || 1, W = this.c.clientWidth, H = this.H, ctx = this.ctx;
    if (this.c.width !== Math.round(W * dpr) || this.c.height !== Math.round(H * dpr)) { this.c.width = Math.round(W * dpr); this.c.height = Math.round(H * dpr); }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    if (!this.project) return;
    if (this.fitted) this.view = [0, Math.max(1, this.outDur())];
    const C = n => this.color(n), font = s => `${s}px ${getComputedStyle(document.body).fontFamily}`;
    const [v0, v1] = this.view, x0 = this.gutter, x1 = W - 8;
    this.c.setAttribute('aria-valuemax', Math.round(this.outDur()));
    this.c.setAttribute('aria-valuenow', Math.round(this.playhead));
    this.c.setAttribute('aria-valuetext', fmt(this.playhead));

    // v2 "instrument" look: mono caps labels in a gutter, hairline lanes,
    // paper clip strips, Signal zoom brackets, words as text, a Signal playhead.
    const MONO = getComputedStyle(document.documentElement).getPropertyValue('--by-font-mono').trim() || 'monospace';
    const mono = (s, w = 500) => `${w} ${s}px ${MONO}`;
    const spaced = (txt, x, y, track = 0.9) => { // letter spacing without ctx.letterSpacing support
      if ('letterSpacing' in ctx) { ctx.letterSpacing = track + 'px'; ctx.fillText(txt, x, y); ctx.letterSpacing = '0px'; return; }
      ctx.fillText(txt, x, y);
    };
    ctx.textBaseline = 'middle';
    // Lane rules and labels
    ctx.fillStyle = C('--by-ui-border-subtle');
    for (const k of ['clips', 'zoom', 'layout', 'marks', 'blur', 'keys', 'words', 'audio']) if (this.L[k]) ctx.fillRect(0, this.L[k][1] + 3, W, 1);
    ctx.fillRect(0, this.L.ruler[1] + 3, W, 1);
    ctx.fillStyle = C('--by-ui-text-3'); ctx.font = mono(9.5);
    for (const [k, label] of [['clips', 'CLIPS'], ['zoom', 'ZOOM'], ['layout', 'LAYOUT'], ['marks', 'NOTES'], ['blur', 'BLUR'], ['keys', 'KEYS'], ['words', 'CAPTIONS'], ['audio', 'AUDIO']]) if (this.L[k]) spaced(label, 10, (this.L[k][0] + this.L[k][1]) / 2);
    ctx.save(); ctx.beginPath(); ctx.rect(x0 - 2, 0, x1 - x0 + 10, H); ctx.clip();

    // Ruler: a minor tick per second (or per label step / 5 when zoomed in or out), labelled major ticks
    const span = v1 - v0, steps = [100, 250, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000, 300000];
    const step = steps.find(s => (s / span) * (x1 - x0) >= 70) || 600000;
    const minor = [1000, step / 5].find(m => (m / span) * (x1 - x0) >= 5 && step % m === 0) || step;
    const rb = this.L.ruler[1];
    ctx.fillStyle = C('--by-ui-border');
    for (let t = Math.ceil(v0 / minor) * minor; t <= v1; t += minor) { const x = Math.round(this.x(t)); ctx.fillRect(x, rb - 3, 1, 3); }
    ctx.font = mono(9, 400);
    for (let t = Math.ceil(v0 / step) * step; t <= v1; t += step) {
      const x = Math.round(this.x(t));
      ctx.fillStyle = C('--by-ui-text-3'); ctx.fillRect(x, rb - 8, 1, 8);
      spaced(fmt(t, step < 1000), x + 4, 9, 0.4);
    }
    // Clips: paper strips with sprocket dashes along both edges
    let acc = 0; const [ca, cb] = this.L.clips;
    this.project.clips.forEach((c, i) => {
      const len = (c.src_end_ms - c.src_start_ms) / c.speed, a = this.x(acc), b = this.x(acc + len), w = Math.max(2, b - a - 3);
      ctx.fillStyle = C('--by-ui-clip'); rr(ctx, a + 1.5, ca, w, cb - ca, 2); ctx.fill();
      if (w > 12) {
        ctx.fillStyle = 'rgba(18, 16, 13, .18)';
        for (let sx = a + 6; sx < a + w - 4; sx += 7) { ctx.fillRect(sx, ca + 2, 3, 2); ctx.fillRect(sx, cb - 4, 3, 2); }
      }
      if (i === this.selectedClip) { ctx.strokeStyle = C('--by-ui-text'); ctx.lineWidth = 1.5; rr(ctx, a + 2.25, ca + .75, w - 1.5, cb - ca - 1.5, 2); ctx.stroke(); ctx.lineWidth = 1; }
      if (w > 48) {
        ctx.fillStyle = C('--by-ui-clip-ink'); ctx.font = mono(9.5);
        spaced(`CLIP ${String(i + 1).padStart(2, '0')}`, a + 8, (ca + cb) / 2 - 5);
        ctx.fillStyle = 'rgba(18, 16, 13, .62)'; ctx.font = mono(9, 400);
        ctx.fillText(c.speed !== 1 ? c.speed + 'x speed' : fmt(len, false), a + 8, (ca + cb) / 2 + 7);
      }
      acc += len;
    });
    // Proposed cuts waiting for review: danger bands over the clips lane.
    if (this.proposed && this.proposed.length) {
      ctx.save(); const dg = C('--by-ui-danger'); ctx.fillStyle = dg; ctx.strokeStyle = dg;
      for (const [s0, e0] of this.proposed) {
        const a = this.x(this.srcToOut(s0)), b = this.x(this.srcToOut(e0));
        ctx.globalAlpha = .45; ctx.fillRect(a, ca - 2, Math.max(3, b - a), cb - ca + 4); ctx.globalAlpha = 1; ctx.strokeRect(a + .5, ca - 1.5, Math.max(3, b - a) - 1, cb - ca + 3);
      }
      ctx.restore();
    }
    // Zoom ranges: Signal brackets with the factor in the middle
    const [za, zb] = this.L.zoom, sig = C('--by-ui-signal');
    this.project.zoom.forEach((z, i) => {
      const a = Math.round(this.x(this.srcToOut(z.start_ms))) + .5, b = Math.round(this.x(this.srcToOut(z.end_ms))) - .5, sel = i === this.selectedZoom;
      const ya = za + 6, yb = zb - 6, arm = Math.min(5, (b - a) / 3);
      if (sel) { ctx.fillStyle = C('--by-ui-hover'); ctx.fillRect(a, ya, b - a, yb - ya); }
      ctx.strokeStyle = sig; ctx.lineWidth = sel ? 2 : 1.5; ctx.beginPath();
      ctx.moveTo(a + arm, ya); ctx.lineTo(a, ya); ctx.lineTo(a, yb); ctx.lineTo(a + arm, yb);
      ctx.moveTo(b - arm, ya); ctx.lineTo(b, ya); ctx.lineTo(b, yb); ctx.lineTo(b - arm, yb); ctx.stroke(); ctx.lineWidth = 1;
      const label = `${z.scale.toFixed(1)}\u00d7`; ctx.font = mono(9, 400); const tw = ctx.measureText(label).width;
      if (b - a > tw + 16) {
        const m = (a + b) / 2, y = (ya + yb) / 2;
        ctx.strokeStyle = C('--by-ui-border-input'); ctx.setLineDash([1, 3]); ctx.beginPath(); ctx.moveTo(a + 6, y + .5); ctx.lineTo(m - tw / 2 - 4, y + .5); ctx.moveTo(m + tw / 2 + 4, y + .5); ctx.lineTo(b - 6, y + .5); ctx.stroke(); ctx.setLineDash([]);
        ctx.fillStyle = sel ? C('--by-ui-text') : C('--by-ui-text-3'); ctx.textAlign = 'center'; ctx.fillText(label, m, y); ctx.textAlign = 'left';
      }
    });
    // Layout changes: outline bars with a mono label
    if (this.hasLayoutLane) {
      const [la, lb] = this.L.layout;
      const names = { screen: 'SCREEN ONLY', bubble: 'CAMERA BUBBLE', split: 'SIDE BY SIDE', camera: 'CAMERA FULL' };
      ((this.project.layouts && this.project.layouts.segments) || []).forEach((g, i) => {
        const a = this.x(this.srcToOut(g.start_ms)), b = this.x(this.srcToOut(g.end_ms)), sel = i === this.selectedLayout;
        if (sel) { ctx.fillStyle = C('--by-ui-hover'); rr(ctx, a, la + 3, Math.max(3, b - a), lb - la - 6, 2); ctx.fill(); }
        ctx.strokeStyle = sel ? C('--by-ui-text') : C('--by-ui-border-input'); ctx.lineWidth = sel ? 1.5 : 1;
        rr(ctx, a + .5, la + 3.5, Math.max(3, b - a - 1), lb - la - 7, 2); ctx.stroke(); ctx.lineWidth = 1;
        if (b - a > 50) { ctx.fillStyle = C('--by-ui-text-2'); ctx.font = mono(9); spaced((names[g.kind] || g.kind.toUpperCase()).slice(0, Math.floor((b - a) / 7)), a + 8, (la + lb) / 2); }
      });
      if (!((this.project.layouts && this.project.layouts.segments) || []).length) { ctx.font = mono(10, 400); ctx.fillStyle = C('--by-ui-text-3'); ctx.fillText('The whole recording uses the base layout. Double click here to change it for a stretch.', x0 + 8, (la + lb) / 2); }
    }
    // Notes (callouts and shapes) and blur boxes
    for (const lane of ['marks', 'blur']) for (const it of this.items(lane)) {
      const [a, b, ya, yb] = this.itemBox(lane, it), sel = it.o === this.selItem;
      ctx.fillStyle = it.hex || C(it.color); ctx.globalAlpha = sel ? 1 : .8;
      rr(ctx, a, ya, Math.max(4, b - a), yb - ya, 2); ctx.fill(); ctx.globalAlpha = 1;
      if (sel) { ctx.strokeStyle = C('--by-ui-text'); ctx.lineWidth = 1.5; rr(ctx, a, ya, Math.max(4, b - a), yb - ya, 2); ctx.stroke(); ctx.lineWidth = 1; }
      if (b - a > 36 && yb - ya > 10) { ctx.fillStyle = it.hex ? '#111' : '#fff'; ctx.font = mono(9.5); ctx.fillText(it.label.slice(0, Math.floor((b - a) / 6)), a + 6, (ya + yb) / 2); }
    }
    // Keystrokes: outlined chips. Overlapping chips move right, never stack.
    { const [ka, kb] = this.L.keys; ctx.font = mono(8.5); let lastEnd = -1e9;
      for (const k of this.keyTrack || []) {
        const a0 = this.x(this.srcToOut(k.start_ms));
        const w = ctx.measureText(k.text).width + 10, a = Math.max(a0, lastEnd + 3);
        if (a > x1 || a + w < x0) { lastEnd = a + w; continue; }
        ctx.strokeStyle = C('--by-ui-border-input'); rr(ctx, a + .5, ka + 3.5, w, kb - ka - 7, 2); ctx.stroke();
        ctx.fillStyle = C('--by-ui-text-2'); ctx.fillText(k.text, a + 5, (ka + kb) / 2 + .5);
        lastEnd = a + w;
      } }
    // Captions: the words as text at their start time. Filler words that will be cut are struck through.
    const [wa, wb] = this.L.words, wy = (wa + wb) / 2;
    const visible = w => [w.start_ms + 40, (w.start_ms + w.end_ms) / 2, w.end_ms - 40].some(t => this.project.clips.some(c => t >= c.src_start_ms && t < c.src_end_ms));
    ctx.font = mono(9.5, 400); let wEnd = -1e9;
    for (const cap of this.project.captions || []) for (const w of cap.words) {
      if (!visible(w)) continue;
      const a = this.x(this.srcToOut(w.start_ms)), tw = ctx.measureText(w.text).width;
      if (a < wEnd + 4 || a > x1) continue;
      const fill = FILLER.test(w.text);
      ctx.fillStyle = fill ? C('--by-ui-warning') : C('--by-ui-text-2'); ctx.fillText(w.text, a, wy);
      if (fill) ctx.fillRect(a - 1, Math.round(wy), tw + 2, 1);
      wEnd = a + tw;
    }
    // Waveform of the sound, laid out the way it will be heard (after cuts and speed changes)
    { const [aa, ab] = this.L.audio, mid = (aa + ab) / 2, amp = (ab - aa) / 2 - 2;
      ctx.font = mono(10, 400); ctx.fillStyle = C('--by-ui-text-3');
      if (this.peaks && this.peaks.data.length) {
        ctx.fillStyle = C('--by-ui-text-3');
        let acc = 0;
        for (const c of this.project.clips) {
          const len = (c.src_end_ms - c.src_start_ms) / c.speed, a = this.x(acc), b = this.x(acc + len), pk = this.peaks.data, bk = this.peaks.bucket;
          for (let px = Math.max(a, x0); px < Math.min(b, x1); px += 2) {
            const s0 = c.src_start_ms + ((px - a) / Math.max(1, b - a)) * len * c.speed, s1 = c.src_start_ms + ((px + 2 - a) / Math.max(1, b - a)) * len * c.speed;
            let v = 0; for (let i = Math.floor(s0 / bk), e = Math.min(pk.length - 1, Math.max(i, Math.floor(s1 / bk))); i <= e && i - Math.floor(s0 / bk) < 40; i++) v = Math.max(v, pk[i] || 0);
            const h = Math.max(1, (v / 255) * amp); ctx.fillRect(px, mid - h, 1.6, h * 2);
          }
          acc += len;
        }
      } else {
        const msg = { building: 'Preparing the sound...', none: 'No sound in this recording', error: 'The sound could not be prepared' }[this.audioState] || '';
        if (msg) ctx.fillText(msg, x0 + 8, mid);
      } }
    // Hints in lanes that are still empty
    ctx.font = mono(10, 400); ctx.fillStyle = C('--by-ui-text-3');
    if (!this.project.zoom.length) ctx.fillText('No zooms yet. Double click here to add one, or run Auto edit.', x0 + 8, (this.L.zoom[0] + this.L.zoom[1]) / 2);
    if (!(this.project.captions || []).length) ctx.fillText('No captions yet. Use Transcribe on the Auto edit tab.', x0 + 8, wy);
    // Playhead: a 1px Signal hairline with a timecode tab on the ruler
    const p = Math.round(this.x(this.playhead));
    ctx.fillStyle = sig; ctx.fillRect(p, 16, 1, H - 16);
    ctx.font = mono(9.5); const tc = fmt(this.playhead), tabW = Math.max(40, ctx.measureText(tc).width + 8);
    const tx = Math.min(Math.max(p - tabW / 2, x0 - 2), x1 + 8 - tabW);
    rr(ctx, tx, 1, tabW, 16, 2); ctx.fill();
    ctx.fillStyle = C('--by-ui-on-signal'); ctx.textAlign = 'center'; ctx.fillText(tc, tx + tabW / 2, 9.5); ctx.textAlign = 'left';
    ctx.restore();
    // Scrollbar when zoomed in
    const dur = this.outDur();
    if (span < dur - 1) {
      const a = x0 + (v0 / dur) * (x1 - x0), b = x0 + (v1 / dur) * (x1 - x0);
      ctx.fillStyle = C('--by-ui-border-input'); rr(ctx, a, H - 6, Math.max(12, b - a), 4, 2); ctx.fill();
    }
  }
}
const SHAPE_LABEL = { arrow: 'Arrow', box: 'Box', ellipse: 'Ellipse', highlight: 'Highlight' };
function rr(ctx, x, y, w, h, r) { r = Math.min(r, w / 2, h / 2); ctx.beginPath(); ctx.roundRect ? ctx.roundRect(x, y, w, h, r) : ctx.rect(x, y, w, h); }
function fmt(ms, tenths = true) {
  const s = Math.max(0, ms) / 1000; let m = Math.floor(s / 60), r = s - m * 60;
  if (tenths && r.toFixed(1) === '60.0') { m += 1; r = 0; }
  return `${String(m).padStart(2, '0')}:${(tenths ? r.toFixed(1) : Math.floor(r).toString()).padStart(tenths ? 4 : 2, '0')}`;
}
