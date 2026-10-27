// Timeline: a ruler, a clip lane (output time), a zoom lane (source time
// mapped to where it lands in the output) and a speech lane. Drag anywhere on
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
    this.H = 150; this.gutter = 64;
    this.L = { ruler: [0, 22], clips: [28, 64], zoom: [72, 104], words: [112, 132] };
    canvas.addEventListener('pointerdown', e => this.down(e));
    canvas.addEventListener('pointermove', e => this.hover(e));
    window.addEventListener('pointermove', e => this.move(e));
    window.addEventListener('pointerup', () => this.up());
    canvas.addEventListener('dblclick', e => this.dbl(e));
    canvas.addEventListener('wheel', e => this.wheel(e), { passive: false });
    new ResizeObserver(() => this.draw()).observe(canvas);
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
  width() { return this.c.clientWidth; }
  x(ms) { const [a, b] = this.view; return this.gutter + ((ms - a) / Math.max(1, b - a)) * (this.width() - this.gutter - 8); }
  ms(x) { const [a, b] = this.view; return Math.max(0, Math.min(this.outDur(), a + ((x - this.gutter) / (this.width() - this.gutter - 8)) * (b - a))); }
  pos(e) { const r = this.c.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; }
  lane(y) { for (const [k, [a, b]] of Object.entries(this.L)) if (y >= a - 3 && y <= b + 3) return k; return null; }
  zoomAt(x) {
    if (!this.project) return null;
    for (let i = 0; i < this.project.zoom.length; i++) {
      const z = this.project.zoom[i], a = this.x(this.srcToOut(z.start_ms)), b = this.x(this.srcToOut(z.end_ms));
      if (Math.abs(x - a) < 6) return { i, edge: 'start' };
      if (Math.abs(x - b) < 6) return { i, edge: 'end' };
      if (x > a && x < b) return { i, edge: 'move' };
    }
    return null;
  }
  hover(e) {
    if (!this.project || this.drag) return;
    const [x, y] = this.pos(e), lane = this.lane(y);
    let cur = 'text';
    if (lane === 'zoom') { const h = this.zoomAt(x); cur = h ? (h.edge === 'move' ? 'grab' : 'ew-resize') : 'copy'; }
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
        this.selectedZoom = h.i;
        this.drag = { kind: 'zoom', ...h, grab: this.outToSrc(this.ms(x)) - z.start_ms, len: z.end_ms - z.start_ms, moved: false };
        if (h.edge === 'move') this.c.style.cursor = 'grabbing';
        this.onSelect(); this.draw(); return;
      }
      this.selectedZoom = -1; this.onSelect();
    }
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
    const z = this.project.zoom[this.drag.i], src = this.outToSrc(this.ms(x));
    this.drag.moved = true;
    // Zooms may not overlap: clamp against the neighbours on either side.
    const others = this.project.zoom.filter((_, j) => j !== this.drag.i);
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
    if (d.kind === 'zoom') {
      this.c.style.cursor = 'grab';
      if (d.moved) { this.project.zoom.sort((a, b) => a.start_ms - b.start_ms); this.onChange(); }
      else { this.playhead = this.srcToOut(this.project.zoom[d.i].start_ms); this.onSeek(this.playhead); }
      this.draw();
    }
  }
  dbl(e) {
    if (!this.project) return;
    const [x, y] = this.pos(e);
    if (this.lane(y) === 'zoom' && !this.zoomAt(x)) {
      const at = this.outToSrc(this.ms(x));
      this.project.zoom.push({ start_ms: at, end_ms: Math.min(this.project.duration_ms, at + 2000), scale: 2, follow_cursor: true, focus: [this.project.source_width / 2, this.project.source_height / 2], reason: 'manual' });
      this.project.zoom.sort((a, b) => a.start_ms - b.start_ms);
      this.selectedZoom = this.project.zoom.findIndex(z => z.start_ms === at);
      this.onSelect(); this.onChange(); this.draw();
    }
  }
  wheel(e) {
    if (!this.project) return;
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

    // Lane labels
    ctx.fillStyle = C('--faint'); ctx.font = font(10); ctx.textBaseline = 'middle';
    for (const [k, label] of [['clips', 'CLIPS'], ['zoom', 'ZOOM'], ['words', 'SPEECH']]) ctx.fillText(label, 10, (this.L[k][0] + this.L[k][1]) / 2);
    ctx.save(); ctx.beginPath(); ctx.rect(x0 - 2, 0, x1 - x0 + 10, H); ctx.clip();

    // Ruler with nice tick spacing
    const span = v1 - v0, steps = [100, 250, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000, 300000];
    const step = steps.find(s => (s / span) * (x1 - x0) >= 70) || 600000;
    ctx.strokeStyle = C('--line'); ctx.fillStyle = C('--tl-ruler'); ctx.font = font(10);
    for (let t = Math.ceil(v0 / step) * step; t <= v1; t += step) {
      const x = this.x(t); ctx.beginPath(); ctx.moveTo(x + .5, 14); ctx.lineTo(x + .5, H); ctx.globalAlpha = .5; ctx.stroke(); ctx.globalAlpha = 1;
      ctx.fillText(fmt(t, step < 1000), x + 4, 9);
    }
    // Clips
    let acc = 0; const [ca, cb] = this.L.clips;
    this.project.clips.forEach((c, i) => {
      const len = (c.src_end_ms - c.src_start_ms) / c.speed, a = this.x(acc), b = this.x(acc + len);
      ctx.fillStyle = i === this.selectedClip ? C('--tl-clip-sel') : C('--tl-clip');
      rr(ctx, a + 1, ca, Math.max(2, b - a - 2), cb - ca, 6); ctx.fill();
      if (b - a > 40) { ctx.fillStyle = '#fff'; ctx.font = font(11); ctx.fillText(`Clip ${i + 1}${c.speed !== 1 ? '  ' + c.speed + 'x' : ''}`, a + 8, (ca + cb) / 2); }
      acc += len;
    });
    // Zoom segments
    const [za, zb] = this.L.zoom;
    this.project.zoom.forEach((z, i) => {
      const a = this.x(this.srcToOut(z.start_ms)), b = this.x(this.srcToOut(z.end_ms)), sel = i === this.selectedZoom;
      ctx.fillStyle = sel ? C('--tl-zoom-sel') : C('--tl-zoom');
      rr(ctx, a, za, Math.max(3, b - a), zb - za, 6); ctx.fill();
      if (sel) { ctx.fillStyle = '#fff'; ctx.fillRect(a + 2, za + 9, 2, zb - za - 18); ctx.fillRect(b - 4, za + 9, 2, zb - za - 18); }
      if (b - a > 50) { ctx.fillStyle = '#fff'; ctx.font = font(11); ctx.fillText(`${z.scale.toFixed(1)}x ${z.reason}`, a + 8, (za + zb) / 2); }
    });
    // Speech: one bar per visible word, filler words highlighted
    const [wa, wb] = this.L.words;
    const visible = w => [w.start_ms + 40, (w.start_ms + w.end_ms) / 2, w.end_ms - 40].some(t => this.project.clips.some(c => t >= c.src_start_ms && t < c.src_end_ms));
    for (const cap of this.project.captions || []) for (const w of cap.words) {
      if (!visible(w)) continue;
      const a = this.x(this.srcToOut(w.start_ms)), b = this.x(this.srcToOut(w.end_ms));
      ctx.fillStyle = FILLER.test(w.text) ? C('--tl-filler') : C('--tl-word');
      rr(ctx, a, wa, Math.max(2, b - a - 1), wb - wa, 3); ctx.fill();
      if (b - a > ctx.measureText(w.text).width + 6) { ctx.fillStyle = '#fff'; ctx.font = font(10); ctx.fillText(w.text, a + 3, (wa + wb) / 2); }
    }
    // Playhead
    const p = this.x(this.playhead);
    ctx.fillStyle = C('--tl-playhead'); ctx.fillRect(p - 1, 12, 2, H - 12);
    ctx.beginPath(); ctx.moveTo(p - 6, 2); ctx.lineTo(p + 6, 2); ctx.lineTo(p + 6, 8); ctx.lineTo(p, 14); ctx.lineTo(p - 6, 8); ctx.closePath(); ctx.fill();
    ctx.restore();
    // Scrollbar when zoomed in
    const dur = this.outDur();
    if (span < dur - 1) {
      const a = x0 + (v0 / dur) * (x1 - x0), b = x0 + (v1 / dur) * (x1 - x0);
      ctx.fillStyle = C('--line-strong'); rr(ctx, a, H - 6, Math.max(12, b - a), 4, 2); ctx.fill();
    }
  }
}
function rr(ctx, x, y, w, h, r) { r = Math.min(r, w / 2, h / 2); ctx.beginPath(); ctx.roundRect ? ctx.roundRect(x, y, w, h, r) : ctx.rect(x, y, w, h); }
function fmt(ms, tenths = true) {
  const s = Math.max(0, ms) / 1000, m = Math.floor(s / 60), r = s - m * 60;
  return `${m}:${(tenths ? r.toFixed(1) : Math.floor(r).toString()).padStart(tenths ? 4 : 2, '0')}`;
}
