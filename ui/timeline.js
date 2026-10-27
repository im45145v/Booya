// Timeline: clips on the top lane (output time), zoom segments on the bottom
// lane (source time, shown where they land in the output). Drag zoom edges to
// edit them; click anywhere to move the playhead.
class Timeline {
  constructor(canvas, onChange, onSeek) {
    this.c = canvas; this.ctx = canvas.getContext('2d');
    this.onChange = onChange; this.onSeek = onSeek;
    this.project = null; this.playhead = 0; this.selectedZoom = -1; this.selectedClip = 0; this.drag = null;
    canvas.addEventListener('mousedown', e => this.down(e));
    window.addEventListener('mousemove', e => this.move(e));
    window.addEventListener('mouseup', () => { if (this.drag) { this.drag = null; this.onChange(); } });
    new ResizeObserver(() => this.draw()).observe(canvas);
  }
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
  x(ms) { return 10 + (ms / Math.max(1, this.outDur())) * (this.c.width - 20); }
  ms(x) { return Math.max(0, Math.min(this.outDur(), ((x - 10) / (this.c.width - 20)) * this.outDur())); }
  localX(e) { const r = this.c.getBoundingClientRect(); return (e.clientX - r.left) * (this.c.width / r.width); }
  down(e) {
    if (!this.project) return;
    const lx = this.localX(e), r = this.c.getBoundingClientRect(), ly = (e.clientY - r.top) * (this.c.height / r.height);
    if (ly > 60) {
      for (let i = 0; i < this.project.zoom.length; i++) {
        const z = this.project.zoom[i], a = this.x(this.srcToOut(z.start_ms)), b = this.x(this.srcToOut(z.end_ms));
        if (Math.abs(lx - a) < 6) { this.selectedZoom = i; this.drag = { i, edge: 'start' }; this.draw(); return; }
        if (Math.abs(lx - b) < 6) { this.selectedZoom = i; this.drag = { i, edge: 'end' }; this.draw(); return; }
        if (lx > a && lx < b) { this.selectedZoom = i; this.draw(); this.onSeek(this.ms(lx), true); return; }
      }
      this.selectedZoom = -1;
    } else {
      const t = this.ms(lx); let acc = 0;
      this.project.clips.forEach((c, i) => { const len = (c.src_end_ms - c.src_start_ms) / c.speed; if (t >= acc && t <= acc + len) this.selectedClip = i; acc += len; });
    }
    this.playhead = this.ms(lx); this.onSeek(this.playhead, true); this.draw();
  }
  move(e) {
    if (!this.drag) return;
    const z = this.project.zoom[this.drag.i], src = this.outToSrc(this.ms(this.localX(e)));
    if (this.drag.edge === 'start') z.start_ms = Math.min(src, z.end_ms - 200); else z.end_ms = Math.max(src, z.start_ms + 200);
    this.draw();
  }
  draw() {
    const ctx = this.ctx, w = (this.c.width = this.c.clientWidth * devicePixelRatio), h = this.c.height;
    ctx.clearRect(0, 0, w, h);
    if (!this.project) return;
    let acc = 0;
    this.project.clips.forEach((c, i) => {
      const len = (c.src_end_ms - c.src_start_ms) / c.speed, a = this.x(acc), b = this.x(acc + len);
      ctx.fillStyle = i === this.selectedClip ? '#4f46e5' : '#3730a3';
      ctx.fillRect(a + 1, 14, b - a - 2, 40);
      ctx.fillStyle = '#e7e7ea'; ctx.font = `${12 * devicePixelRatio}px system-ui`;
      ctx.fillText(`clip ${i + 1}${c.speed !== 1 ? ' x' + c.speed : ''}`, a + 6, 38);
      acc += len;
    });
    this.project.zoom.forEach((z, i) => {
      const a = this.x(this.srcToOut(z.start_ms)), b = this.x(this.srcToOut(z.end_ms));
      ctx.fillStyle = i === this.selectedZoom ? '#ec4899' : '#9d174d';
      ctx.fillRect(a, 66, Math.max(2, b - a), 36);
      ctx.fillStyle = '#fff'; ctx.fillText(`${z.scale.toFixed(1)}x ${z.reason}`, a + 4, 88);
    });
    // Transcript lane: one tick per word, filler words in orange.
    const visible = w => [w.start_ms + 40, (w.start_ms + w.end_ms) / 2, w.end_ms - 40].some(t => this.project.clips.some(c => t >= c.src_start_ms && t < c.src_end_ms));
    for (const cap of this.project.captions || []) for (const w of cap.words) {
      if (!visible(w)) continue;
      const a = this.x(this.srcToOut(w.start_ms)), b = this.x(this.srcToOut(w.end_ms));
      ctx.fillStyle = /^(u+m+|u+h+|uhm|erm|er|ah|hmm+|mm)\W*$/i.test(w.text) ? '#f59e0b' : '#10b981';
      ctx.fillRect(a, 110, Math.max(2, b - a - 1), 12);
    }
    const p = this.x(this.playhead);
    ctx.fillStyle = '#fbbf24'; ctx.fillRect(p - 1, 0, 2, h);
  }
}
