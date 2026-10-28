// Caption editing helpers. Pure functions on the project's caption objects
// ({start_ms, end_ms, text, words: [{start_ms, end_ms, text}]}), so they can be
// tested without the app. The words are the source of truth: a caption's text
// is always its words joined, which is what burned in captions, SRT and VTT use.
(function (root) {
  'use strict';

  const clean = (t) => String(t == null ? '' : t).trim().split(/\s+/).filter(Boolean);

  // Give a caption words when it has none (an imported line), sharing its time
  // by word length.
  function ensureWords(c) {
    if (c.words && c.words.length) return c;
    const toks = clean(c.text);
    const total = toks.reduce((a, w) => a + w.length + 1, 0) || 1;
    let at = c.start_ms;
    c.words = toks.map((w) => {
      const len = (c.end_ms - c.start_ms) * ((w.length + 1) / total);
      const o = { start_ms: at, end_ms: at + len, text: w };
      at += len;
      return o;
    });
    return c;
  }

  function sync(c) {
    c.text = (c.words || []).map((w) => w.text).join(' ');
    if (c.words && c.words.length) {
      c.start_ms = Math.min(c.start_ms, c.words[0].start_ms);
      c.end_ms = Math.max(c.end_ms, c.words[c.words.length - 1].end_ms);
    }
    return c;
  }

  // Change one word, keeping its timing. Empty text removes the word. Text with
  // spaces becomes several words that share the original word's time.
  function setWord(c, i, text) {
    ensureWords(c);
    const w = c.words[i];
    if (!w) return c;
    const toks = clean(text);
    if (toks.length === 0) c.words.splice(i, 1);
    else if (toks.length === 1) w.text = toks[0];
    else {
      const total = toks.reduce((a, t) => a + t.length + 1, 0);
      let at = w.start_ms;
      const parts = toks.map((t) => {
        const len = (w.end_ms - w.start_ms) * ((t.length + 1) / total);
        const o = { start_ms: at, end_ms: at + len, text: t };
        at += len;
        return o;
      });
      c.words.splice(i, 1, ...parts);
    }
    return sync(c);
  }

  // Join caption i with the one after it. Returns the new list.
  function mergeNext(caps, i) {
    if (i < 0 || i + 1 >= caps.length) return caps;
    const a = ensureWords(caps[i]), b = ensureWords(caps[i + 1]);
    const merged = { start_ms: a.start_ms, end_ms: Math.max(a.end_ms, b.end_ms), text: '', words: a.words.concat(b.words) };
    merged.text = merged.words.map((w) => w.text).join(' ');
    return caps.slice(0, i).concat([merged], caps.slice(i + 2));
  }

  // Start a new caption at word k (the first k words stay). Returns the new list.
  function splitAt(caps, i, k) {
    const c = ensureWords(caps[i]);
    if (!c || k <= 0 || k >= c.words.length) return caps;
    const first = c.words.slice(0, k), second = c.words.slice(k);
    const a = { start_ms: c.start_ms, end_ms: second[0].start_ms, text: first.map((w) => w.text).join(' '), words: first };
    const b = { start_ms: second[0].start_ms, end_ms: c.end_ms, text: second.map((w) => w.text).join(' '), words: second };
    // The first part may not end before its last word does.
    a.end_ms = Math.max(a.end_ms, first[first.length - 1].end_ms);
    b.end_ms = Math.max(b.end_ms, second[second.length - 1].end_ms);
    return caps.slice(0, i).concat([a, b], caps.slice(i + 1));
  }

  // Move a caption's start, end or both (with all its words) by delta ms. The
  // line never gets shorter than 200 ms or crosses its neighbours' starts by
  // more than the move itself, and never goes below zero.
  function nudge(caps, i, which, delta) {
    const c = caps[i];
    if (!c) return caps;
    const min = 200;
    if (which === 'start') c.start_ms = Math.max(0, Math.min(c.start_ms + delta, c.end_ms - min));
    else if (which === 'end') c.end_ms = Math.max(c.start_ms + min, c.end_ms + delta);
    else {
      const d = Math.max(delta, -c.start_ms);
      c.start_ms += d;
      c.end_ms += d;
      (c.words || []).forEach((w) => { w.start_ms += d; w.end_ms += d; });
    }
    return caps;
  }

  const api = { ensureWords, setWord, mergeNext, splitAt, nudge, sync };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.CaptionEdit = api;
})(typeof window !== 'undefined' ? window : globalThis);
