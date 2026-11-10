// Geometry for the camera bubble on the preview: where the renderer puts it,
// and where it lands when you drag it. Mirrors build_scene in
// crates/booya-core/src/render.rs. No DOM here, so tests/js/campos.test.js
// runs it under node. Sizes are in preview pixels (W x H is the picture).
(function (root) {
  'use strict';

  const CORNERS = ['top-left', 'top-center', 'top-right', 'left-center', 'right-center', 'bottom-left', 'bottom-center', 'bottom-right'];

  // Width and height of the bubble for a webcam style.
  function bubbleSize(cam, W, H) {
    const short = Math.min(W, H);
    const size = Math.max(0.08, Math.min(0.5, +cam.size || 0.24));
    const d = Math.max(16, Math.round(size * short) & ~1);
    const shape = cam.shape || (cam.circle ? 'circle' : 'rounded-square');
    return shape === 'rectangle' ? { w: d * 1.5, h: d } : { w: d, h: d };
  }

  // Top left of a w x h box in a corner (or edge middle) with margin m.
  function place(corner, W, H, w, h, m) {
    const cx = (W - w) / 2, cy = (H - h) / 2;
    switch (corner) {
      case 'top-left': return { x: m, y: m };
      case 'top-right': return { x: W - w - m, y: m };
      case 'bottom-left': return { x: m, y: H - h - m };
      case 'top-center': return { x: cx, y: m };
      case 'bottom-center': return { x: cx, y: H - h - m };
      case 'left-center': return { x: m, y: cy };
      case 'right-center': return { x: W - w - m, y: cy };
      default: return { x: W - w - m, y: H - h - m };
    }
  }

  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

  // The bubble as { x, y, w, h } in a W x H picture.
  function bubbleRect(cam, W, H) {
    const { w, h } = bubbleSize(cam, W, H), m = Math.min(W, H) * 0.035;
    const p = cam.position;
    if (Array.isArray(p) && Number.isFinite(p[0]) && Number.isFinite(p[1])) {
      return { x: clamp(clamp(p[0], 0, 1) * W - w / 2, 0, Math.max(0, W - w)), y: clamp(clamp(p[1], 0, 1) * H - h / 2, 0, Math.max(0, H - h)), w, h };
    }
    return { ...place(cam.corner, W, H, w, h, m), w, h };
  }

  // Where a bubble whose centre is dropped at (x, y) ends up. Near one of the
  // eight corner and edge spots it snaps there (corner set, no position), so
  // it keeps its place when the picture changes shape. Near the middle it
  // snaps to the exact centre. Anywhere else it stays where it was dropped.
  // Returns { corner, position, snapped }.
  function drop(cam, W, H, x, y) {
    const { w, h } = bubbleSize(cam, W, H), short = Math.min(W, H), m = short * 0.035, tol = short * 0.045;
    // Keep the whole bubble inside, as the renderer will.
    const cx = clamp(x, w / 2, Math.max(w / 2, W - w / 2)), cy = clamp(y, h / 2, Math.max(h / 2, H - h / 2));
    let best = null;
    for (const c of CORNERS) {
      const o = place(c, W, H, w, h, m), d = Math.hypot(o.x + w / 2 - cx, o.y + h / 2 - cy);
      if (d <= tol && (!best || d < best.d)) best = { c, d };
    }
    if (best) return { corner: best.c, position: null, snapped: best.c };
    if (Math.hypot(cx - W / 2, cy - H / 2) <= tol) return { corner: cam.corner, position: [0.5, 0.5], snapped: 'center' };
    const r = v => Math.round(v * 1000) / 1000;
    return { corner: cam.corner, position: [r(cx / W), r(cy / H)], snapped: null };
  }

  // A new bubble size after a wheel step or a resize drag, in the range the
  // renderer accepts.
  function resize(size, delta) {
    return Math.round(clamp((+size || 0.24) + delta, 0.08, 0.5) * 100) / 100;
  }

  // Which layout is on screen at source time t.
  function layoutAt(layouts, base, t) {
    const seg = ((layouts && layouts.segments) || []).find(s => t >= s.start_ms && t < s.end_ms);
    return seg ? seg.kind : base;
  }

  const api = { CORNERS, bubbleSize, bubbleRect, drop, resize, layoutAt };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.CamPos = api;
})(typeof window !== 'undefined' ? window : globalThis);
