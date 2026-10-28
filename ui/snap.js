// Pure geometry for the area picker: which window is under the pointer, the
// largest rectangle of an aspect ratio inside a window, and edge snapping.
// No DOM here, so it runs under node for the tests in tests/js/snap.test.js.
// Rectangles are { x, y, w, h } in screenshot pixels. Windows also carry z,
// the stacking order, where 0 is frontmost.
(function (root) {
  'use strict';

  // Rounds a rectangle and clips it to the W x H screen. Null when nothing is left.
  function clipRect(r, W, H) {
    const x0 = Math.max(0, Math.round(r.x)), y0 = Math.max(0, Math.round(r.y));
    const x1 = Math.min(W, Math.round(r.x + r.w)), y1 = Math.min(H, Math.round(r.y + r.h));
    return x1 - x0 > 0 && y1 - y0 > 0 ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : null;
  }

  // Window sources on screen, clipped to the screenshot, ready for hit testing.
  function usableWindows(sources, W, H) {
    const out = [];
    (sources || []).forEach((s, i) => {
      if (s.kind !== 'window' || !(s.w > 0 && s.h > 0)) return;
      const r = clipRect(s, W, H);
      if (!r || r.w < 24 || r.h < 24) return;
      out.push({ ...r, z: Number.isFinite(s.z) ? s.z : i, app: s.app || '', title: s.title || '', id: s.id });
    });
    return out;
  }

  // The frontmost window containing the point. Equal stacking falls back to the smaller window.
  function windowAt(wins, x, y) {
    let best = null;
    for (const w of wins) {
      if (x < w.x || y < w.y || x >= w.x + w.w || y >= w.y + w.h) continue;
      if (!best || w.z < best.z || (w.z === best.z && w.w * w.h < best.w * best.h)) best = w;
    }
    return best;
  }

  // The largest rectangle of the aspect (w / h) centred in r. An aspect of 0 keeps r.
  function fitAspect(r, aspect) {
    if (!aspect) return { x: r.x, y: r.y, w: r.w, h: r.h };
    let w = r.w, h = r.h;
    if (r.w / r.h > aspect) w = Math.round(r.h * aspect); else h = Math.round(r.w / aspect);
    return { x: r.x + Math.round((r.w - w) / 2), y: r.y + Math.round((r.h - h) / 2), w, h };
  }

  // Sorted unique edge positions to snap to: the screen edges and every window edge.
  function edgeTargets(wins, W, H) {
    const xs = new Set([0, W]), ys = new Set([0, H]);
    wins.forEach(w => { xs.add(w.x); xs.add(w.x + w.w); ys.add(w.y); ys.add(w.y + w.h); });
    const asc = (a, b) => a - b;
    return { xs: [...xs].sort(asc), ys: [...ys].sort(asc) };
  }

  // The adjustment that brings one of the values onto the nearest target within tol.
  // Returns { d, t }: the shift and the target, or { d: 0, t: null } when nothing is near.
  function pull(values, targets, tol) {
    let best = { d: 0, t: null }, bestAbs = Infinity;
    for (const v of values) {
      for (const t of targets) {
        const a = Math.abs(t - v);
        if (a <= tol && a < bestAbs) { best = { d: t - v, t }; bestAbs = a; }
      }
    }
    return best;
  }

  // Snaps a point (a corner being dragged) onto nearby edges.
  function snapPoint(px, py, edges, tol) {
    const sx = pull([px], edges.xs, tol), sy = pull([py], edges.ys, tol);
    return { x: px + sx.d, y: py + sy.d, gx: sx.t, gy: sy.t };
  }

  // Snaps a whole rectangle being moved: its left or right edge, and its top or bottom edge.
  function snapMove(r, edges, tol) {
    const sx = pull([r.x, r.x + r.w], edges.xs, tol), sy = pull([r.y, r.y + r.h], edges.ys, tol);
    return { rect: { x: r.x + sx.d, y: r.y + sy.d, w: r.w, h: r.h }, gx: sx.t, gy: sy.t };
  }

  const api = { clipRect, usableWindows, windowAt, fitAspect, edgeTargets, snapPoint, snapMove };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.Snap = api;
})(typeof window !== 'undefined' ? window : globalThis);
