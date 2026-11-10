// Run with: node --test tests/js
const test = require('node:test');
const assert = require('node:assert/strict');
const C = require('../../ui/campos.js');

const cam = (o) => ({ corner: 'bottom-right', size: 0.24, circle: true, ...o });

test('the bubble matches the renderer in its corner', () => {
  // 1280x720: d = round(0.24 * 720) & ~1 = 172, margin 0.035 * 720 = 25.2.
  const r = C.bubbleRect(cam(), 1280, 720);
  assert.deepEqual([r.w, r.h], [172, 172]);
  assert.ok(Math.abs(r.x - (1280 - 172 - 25.2)) < 1e-9 && Math.abs(r.y - (720 - 172 - 25.2)) < 1e-9);
  const t = C.bubbleRect(cam({ corner: 'top-left', shape: 'rectangle' }), 1280, 720);
  assert.deepEqual([t.x, t.y, t.w, t.h].map(Math.round), [25, 25, 258, 172]);
});

test('a dragged bubble is centred on its spot, and the same share in another shape', () => {
  // The same numbers as a_dragged_bubble_sits_where_it_was_dropped_and_stays_inside in render.rs.
  const r = C.bubbleRect(cam({ position: [0.3, 0.4] }), 1280, 720);
  assert.ok(Math.abs(r.x + r.w / 2 - 384) < 1 && Math.abs(r.y + r.h / 2 - 288) < 1);
  const t = C.bubbleRect(cam({ position: [0.3, 0.4] }), 720, 1280);
  assert.ok(Math.abs(t.x + t.w / 2 - 216) < 1 && Math.abs(t.y + t.h / 2 - 512) < 1);
  for (const p of [[0, 0], [1, 1], [-3, 0.5], [0.5, 9]]) {
    const b = C.bubbleRect(cam({ position: p }), 1280, 720);
    assert.ok(b.x >= 0 && b.y >= 0 && b.x + b.w <= 1280 && b.y + b.h <= 720, JSON.stringify(p));
  }
});

test('dropping near a corner or an edge middle snaps to it and forgets the spot', () => {
  const r = C.bubbleRect(cam({ corner: 'top-left' }), 1280, 720);
  const d = C.drop(cam(), 1280, 720, r.x + r.w / 2 + 12, r.y + r.h / 2 - 10);
  assert.deepEqual(d, { corner: 'top-left', position: null, snapped: 'top-left' });
  const mid = C.bubbleRect(cam({ corner: 'right-center' }), 1280, 720);
  assert.equal(C.drop(cam(), 1280, 720, mid.x + mid.w / 2 - 5, mid.y + mid.h / 2 + 8).corner, 'right-center');
});

test('dropping near the middle centres it, anywhere else keeps the spot', () => {
  assert.deepEqual(C.drop(cam(), 1280, 720, 650, 352).position, [0.5, 0.5]);
  const d = C.drop(cam(), 1280, 720, 400, 250);
  assert.equal(d.snapped, null);
  assert.deepEqual(d.position, [0.313, 0.347]);
  assert.equal(d.corner, 'bottom-right', 'the corner is kept for split layouts');
  // Past the edge: held so the whole bubble stays in the picture.
  const e = C.drop(cam(), 1280, 720, 5000, 330);
  const b = C.bubbleRect(cam({ position: e.position }), 1280, 720);
  assert.ok(b.x + b.w <= 1280 + 1e-9);
});

test('resizing stays in the range the renderer accepts', () => {
  assert.equal(C.resize(0.24, 0.02), 0.26);
  assert.equal(C.resize(0.49, 0.05), 0.5);
  assert.equal(C.resize(0.09, -0.05), 0.08);
});

test('the layout at a time comes from the timeline segments, else the base', () => {
  const L = { segments: [{ start_ms: 1000, end_ms: 2000, kind: 'split' }] };
  assert.equal(C.layoutAt(L, 'bubble', 500), 'bubble');
  assert.equal(C.layoutAt(L, 'bubble', 1500), 'split');
  assert.equal(C.layoutAt(L, 'bubble', 2000), 'bubble');
  assert.equal(C.layoutAt(null, 'screen', 0), 'screen');
});
