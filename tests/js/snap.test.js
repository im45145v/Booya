// Run with: node --test tests/js
const test = require('node:test');
const assert = require('node:assert/strict');
const S = require('../../ui/snap.js');

const W = 1920, H = 1080;
const src = (o) => ({ kind: 'window', id: '1', title: 't', app: 'App', x: 0, y: 0, w: 400, h: 300, ...o });

test('windows are clipped to the screen and tiny or off screen ones are dropped', () => {
  const v = S.usableWindows([
    src({ id: 'a', x: -50, y: 20, w: 450, h: 300 }),
    src({ id: 'b', x: 1880, y: 0, w: 400, h: 300 }),
    src({ id: 'c', x: 5000, y: 0 }),
    src({ id: 'd', w: 10, h: 10 }),
    src({ id: 'e', w: 0, h: 0 }),
    { kind: 'display', id: '0', x: 0, y: 0, w: W, h: H },
  ], W, H);
  assert.deepEqual(v.map(w => w.id), ['a', 'b']);
  assert.deepEqual([v[0].x, v[0].w], [0, 400]);
  assert.deepEqual([v[1].x, v[1].w], [1880, 40]);
});

test('the frontmost window wins, then the smaller one', () => {
  const wins = S.usableWindows([
    src({ id: 'back', w: 1000, h: 800, z: 5 }),
    src({ id: 'front', x: 100, y: 100, w: 300, h: 200, z: 1 }),
    src({ id: 'tie-big', x: 700, y: 500, w: 200, h: 200, z: 3 }),
    src({ id: 'tie-small', x: 750, y: 550, w: 50, h: 50, z: 3 }),
  ], W, H);
  assert.equal(S.windowAt(wins, 150, 150).id, 'front');
  assert.equal(S.windowAt(wins, 50, 50).id, 'back');
  assert.equal(S.windowAt(wins, 760, 560).id, 'tie-small');
  assert.equal(S.windowAt(wins, 1500, 900), null);
  assert.equal(S.windowAt(wins, 1000, 0), null, 'right edge is exclusive');
});

test('an unlocked aspect keeps the exact window frame', () => {
  const r = { x: 10, y: 20, w: 801, h: 603 };
  assert.deepEqual(S.fitAspect(r, 0), r);
});

test('a locked aspect fits the largest centred rectangle', () => {
  assert.deepEqual(S.fitAspect({ x: 100, y: 100, w: 800, h: 800 }, 16 / 9), { x: 100, y: 275, w: 800, h: 450 });
  assert.deepEqual(S.fitAspect({ x: 0, y: 0, w: 1600, h: 600 }, 1), { x: 500, y: 0, w: 600, h: 600 });
  assert.deepEqual(S.fitAspect({ x: 0, y: 0, w: 1200, h: 800 }, 9 / 16), { x: 375, y: 0, w: 450, h: 800 });
  const f = S.fitAspect({ x: 5, y: 7, w: 777, h: 555 }, 4 / 3);
  assert.ok(f.w <= 777 && f.h <= 555 && f.x >= 5 && f.y >= 7);
  assert.ok(Math.abs(f.w / f.h - 4 / 3) < 0.01);
});

test('edge targets cover the screen and every window edge', () => {
  const wins = S.usableWindows([src({ x: 100, y: 50, w: 400, h: 300 })], W, H);
  const e = S.edgeTargets(wins, W, H);
  assert.deepEqual(e.xs, [0, 100, 500, W]);
  assert.deepEqual(e.ys, [0, 50, 350, H]);
});

test('a dragged corner snaps within the tolerance and not beyond it', () => {
  const e = { xs: [0, 500, W], ys: [0, 350, H] };
  const near = S.snapPoint(494, 357, e, 8);
  assert.deepEqual([near.x, near.y, near.gx, near.gy], [500, 350, 500, 350]);
  const far = S.snapPoint(480, 330, e, 8);
  assert.deepEqual([far.x, far.y, far.gx, far.gy], [480, 330, null, null]);
  const one = S.snapPoint(503, 200, e, 8);
  assert.deepEqual([one.x, one.y, one.gx, one.gy], [500, 200, 500, null]);
});

test('a moved rectangle snaps by whichever edge is closest', () => {
  const e = { xs: [0, 500, 900, W], ys: [0, 350, H] };
  // The right edge (x + w = 897) is 3 away from 900, the left edge is far from everything.
  const m = S.snapMove({ x: 597, y: 100, w: 300, h: 200 }, e, 8);
  assert.deepEqual(m.rect, { x: 600, y: 100, w: 300, h: 200 });
  assert.equal(m.gx, 900);
  assert.equal(m.gy, null);
  // The bottom edge (y + h = 346) snaps to 350 while x stays.
  const n = S.snapMove({ x: 1000, y: 146, w: 300, h: 200 }, e, 8);
  assert.deepEqual(n.rect, { x: 1000, y: 150, w: 300, h: 200 });
  assert.equal(n.gy, 350);
  // Closest wins when two edges are both in range.
  const c = S.snapMove({ x: 502, y: 0, w: 396, h: 100 }, e, 8);
  assert.equal(c.gx, 500);
  assert.equal(c.rect.x, 500);
});
