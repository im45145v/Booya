// The timeline reacts to a finger: a bigger edge to grab, a double tap that
// adds a zoom, and a two finger pinch that zooms the view.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

function make() {
  const listeners = {};
  const canvas = {
    clientWidth: 800, style: {}, width: 0, height: 0,
    addEventListener: (n, f) => { (listeners['c:' + n] = listeners['c:' + n] || []).push(f); },
    getContext: () => new Proxy({}, { get: (_, k) => (k === 'measureText' ? () => ({ width: 10 }) : () => {}), set: () => true }),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 222 }),
    focus() {}, setAttribute() {},
  };
  const win = { devicePixelRatio: 1, addEventListener: (n, f) => { (listeners['w:' + n] = listeners['w:' + n] || []).push(f); } };
  let t = 1000;
  const ctx = vm.createContext({
    window: win, document: { documentElement: {}, body: {} }, ResizeObserver: class { observe() {} },
    getComputedStyle: () => ({ getPropertyValue: () => '#000', fontFamily: 'x' }), performance: { now: () => t }, Math, Map, console,
  });
  vm.runInContext(fs.readFileSync(__dirname + '/../../ui/timeline.js', 'utf8') + '\nthis.Timeline = Timeline;', ctx);
  const tl = new ctx.Timeline(canvas, { onChange() {}, onSeek() {}, onSelect() {} });
  tl.draw = () => {};
  tl.setProject({ duration_ms: 10000, source_width: 1920, source_height: 1080, clips: [{ src_start_ms: 0, src_end_ms: 10000, speed: 1 }], zoom: [], callouts: [], shapes: [], blur: [] });
  const fire = (key, e) => (listeners[key] || []).forEach(f => f(e));
  return { tl, fire, tick: ms => { t += ms; } };
}
const ptr = (id, x, y, type = 'touch') => ({ pointerId: id, clientX: x, clientY: y, pointerType: type });

test('a mouse 12 px from an edge does not grab it', () => {
  const { tl, fire } = make();
  tl.project.zoom.push({ start_ms: 2000, end_ms: 4000, scale: 2, follow_cursor: true, focus: [0, 0], reason: 'x' });
  const edge = tl.x(tl.srcToOut(2000));
  fire('c:pointerdown', ptr(1, edge - 12, 80, 'mouse'));
  assert.notEqual(tl.drag && tl.drag.kind, 'zoom'); // a mouse must be within 6 px
});

test('a touch 12 px from an edge still grabs the edge', () => {
  const { tl, fire } = make();
  tl.project.zoom.push({ start_ms: 2000, end_ms: 4000, scale: 2, follow_cursor: true, focus: [0, 0], reason: 'x' });
  const edge = tl.x(tl.srcToOut(2000));
  fire('c:pointerdown', ptr(1, edge - 12, 80, 'touch'));
  assert.equal(tl.drag.kind, 'zoom');
  assert.equal(tl.drag.edge, 'start');
});

test('a double tap on an empty zoom lane adds a zoom', () => {
  const { tl, fire, tick } = make();
  fire('c:pointerdown', ptr(1, 300, 80));
  fire('w:pointerup', ptr(1, 300, 80));
  tick(150);
  fire('c:pointerdown', ptr(2, 302, 81));
  fire('w:pointerup', ptr(2, 302, 81));
  assert.equal(tl.project.zoom.length, 1);
});

test('two fingers moving apart zoom the timeline in', () => {
  const { tl, fire } = make();
  const span0 = tl.view[1] - tl.view[0];
  fire('c:pointerdown', ptr(1, 300, 100));
  fire('c:pointerdown', ptr(2, 400, 100));
  fire('w:pointermove', ptr(2, 500, 100));
  assert.ok(tl.view[1] - tl.view[0] < span0, 'the visible range shrank');
  fire('w:pointerup', ptr(2, 500, 100));
  fire('w:pointerup', ptr(1, 300, 100));
  assert.equal(tl.pinch, null);
});
