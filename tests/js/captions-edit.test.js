const test = require('node:test');
const assert = require('node:assert/strict');
const E = require('../../ui/captions-edit.js');

const cap = () => ({ start_ms: 1000, end_ms: 3000, text: 'hello wrld again', words: [
  { start_ms: 1000, end_ms: 1500, text: 'hello' }, { start_ms: 1500, end_ms: 2100, text: 'wrld' }, { start_ms: 2100, end_ms: 3000, text: 'again' }] });

test('fixing a word keeps its timing and updates the line', () => {
  const c = cap();
  E.setWord(c, 1, 'world');
  assert.equal(c.text, 'hello world again');
  assert.deepEqual(c.words[1], { start_ms: 1500, end_ms: 2100, text: 'world' });
  assert.equal(c.words.length, 3);
});

test('a word replaced by several shares its time, and an empty one is removed', () => {
  const c = cap();
  E.setWord(c, 1, 'big world');
  assert.equal(c.words.length, 4);
  assert.equal(c.words[1].start_ms, 1500);
  assert.equal(c.words[2].end_ms, 2100);
  assert.equal(c.text, 'hello big world again');
  E.setWord(c, 0, '  ');
  assert.equal(c.text, 'big world again');
  assert.equal(c.start_ms <= c.words[0].start_ms, true);
});

test('a line without words gets them from its text', () => {
  const c = { start_ms: 0, end_ms: 1000, text: 'one two', words: [] };
  E.setWord(c, 1, 'too');
  assert.equal(c.text, 'one too');
  assert.equal(c.words.length, 2);
  assert.ok(Math.abs(c.words[1].end_ms - 1000) < 1e-6);
});

test('lines merge and split without losing words or timing', () => {
  const a = cap(), b = { start_ms: 3200, end_ms: 4000, text: 'and more', words: [{ start_ms: 3200, end_ms: 3500, text: 'and' }, { start_ms: 3500, end_ms: 4000, text: 'more' }] };
  const m = E.mergeNext([a, b], 0);
  assert.equal(m.length, 1);
  assert.equal(m[0].text, 'hello wrld again and more');
  assert.equal(m[0].start_ms, 1000);
  assert.equal(m[0].end_ms, 4000);
  const s = E.splitAt(m, 0, 3);
  assert.equal(s.length, 2);
  assert.equal(s[0].text, 'hello wrld again');
  assert.equal(s[1].text, 'and more');
  assert.equal(s[1].start_ms, 3200);
  assert.ok(s[0].end_ms >= 3000 && s[0].end_ms <= 3200);
  assert.equal(E.splitAt(s, 0, 0), s);
  assert.equal(E.mergeNext(s, 1), s);
});

test('timing can be nudged and never inverts or goes below zero', () => {
  const caps = [cap()];
  E.nudge(caps, 0, 'start', 300);
  assert.equal(caps[0].start_ms, 1300);
  E.nudge(caps, 0, 'start', 5000);
  assert.equal(caps[0].start_ms, 2800);
  E.nudge(caps, 0, 'end', -5000);
  assert.equal(caps[0].end_ms, 3000);
  const c2 = [cap()];
  E.nudge(c2, 0, 'both', -5000);
  assert.equal(c2[0].start_ms, 0);
  assert.equal(c2[0].words[0].start_ms, 0);
  assert.equal(c2[0].words[2].end_ms, 2000);
});
