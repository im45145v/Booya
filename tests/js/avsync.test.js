// Run with: node --test tests/js
const test = require('node:test');
const assert = require('node:assert/strict');
const A = require('../../ui/avsync.js');

test('the round trip is smoothed and bad samples are ignored', () => {
  assert.equal(A.nextLatency(null, 100), 100);
  assert.equal(A.nextLatency(100, 200), 130);
  assert.equal(A.nextLatency(100, NaN), 100);
  assert.equal(A.nextLatency(100, -5), 100);
  assert.equal(A.nextLatency(100, 99999), 100 * 0.7 + 1500 * 0.3);
});

test('while playing the frame asked for is ahead by the round trip, capped', () => {
  assert.equal(A.frameTime(1000, 120, 10000, true), 1120);
  assert.equal(A.frameTime(1000, 120, 10000, false), 1000);
  assert.equal(A.frameTime(1000, 5000, 10000, true), 1400);
  assert.equal(A.frameTime(9900, 300, 10000, true), 10000);
  assert.equal(A.frameTime(0, null, 10000, true), 0);
});

test('the sound is left alone when in step, nudged when close, moved when far', () => {
  assert.deepEqual(A.audioCorrection(5.0, 5.01, 1), { seek: false, rate: 1 });
  const behind = A.audioCorrection(5.1, 5.0, 1);
  assert.equal(behind.seek, false);
  assert.ok(behind.rate > 1 && behind.rate <= 1.06);
  const ahead = A.audioCorrection(5.0, 5.1, 2);
  assert.ok(ahead.rate < 2 && ahead.rate >= 2 * 0.94);
  assert.deepEqual(A.audioCorrection(5.0, 6.0, 1), { seek: true, rate: 1 });
});

test('native sound is re-seeked only beyond 60 ms of drift', () => {
  assert.equal(A.nativeCorrection(10, 10.05), false);
  assert.equal(A.nativeCorrection(10, 9.95), false);
  assert.equal(A.nativeCorrection(10, 10.07), true);
  assert.equal(A.nativeCorrection(10, 9.9), true);
  assert.equal(A.NATIVE_TOLERANCE_S, 0.06);
});
