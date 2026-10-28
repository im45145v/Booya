// Layout: two draggable splitters, and keeping everything inside the window.
//
//  * #split-v  between the preview and the side panel (panel 260 to 520 px).
//  * #split-h  between the preview and the timeline (timeline 96 to 252 px).
//
// Sizes are saved in localStorage. What is saved is what the user chose; what
// is applied is that choice clamped to the window as it is now, so a small
// window never hides the preview and a bigger one gets the choice back.
// Both splitters work with mouse, pen and finger (pointer events), with the
// arrow keys, and reset on double click (or double tap), Home or Enter.
(() => {
  const KEY = 'demolens.layout.v1';
  const PANEL = { min: 260, max: 520, def: 300 };
  const TL = { min: 96, max: 252, def: 252 };
  // The preview never gets smaller than this because of a splitter.
  const PREVIEW_MIN_W = 360, PREVIEW_MIN_H = 150;
  const root = document.documentElement;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

  let saved = { panel: PANEL.def, tl: TL.def };
  try { Object.assign(saved, JSON.parse(localStorage.getItem(KEY) || '{}')); } catch (_) { /* damaged value: defaults */ }
  const persist = () => { try { localStorage.setItem(KEY, JSON.stringify(saved)); } catch (_) { /* storage full or blocked */ } };

  const ws = () => document.querySelector('.workspace');
  const stage = () => document.querySelector('.stage');

  // The limits right now, from the size of the window.
  function limits() {
    const w = ws() ? ws().clientWidth : window.innerWidth;
    const st = stage();
    const tr = document.querySelector('.transport');
    const stageH = st ? st.clientHeight : window.innerHeight;
    const fixed = (tr ? tr.offsetHeight + 8 : 44) + 8 + 16; // transport, splitter, padding
    return {
      panelMax: Math.max(PANEL.min, Math.min(PANEL.max, w - 6 - PREVIEW_MIN_W)),
      tlMax: Math.max(TL.min, Math.min(TL.max, stageH - fixed - PREVIEW_MIN_H)),
    };
  }

  function apply() {
    const lim = limits();
    const panel = Math.round(clamp(saved.panel, PANEL.min, lim.panelMax));
    const tl = Math.round(clamp(saved.tl, TL.min, lim.tlMax));
    root.style.setProperty('--panel-w', panel + 'px');
    root.style.setProperty('--tl-h', tl + 'px');
    const v = document.getElementById('split-v'), h = document.getElementById('split-h');
    if (v) { v.setAttribute('aria-valuenow', panel); v.setAttribute('aria-valuemax', lim.panelMax); }
    if (h) { h.setAttribute('aria-valuenow', tl); h.setAttribute('aria-valuemax', lim.tlMax); }
    return { panel, tl };
  }

  // One splitter: `axis` 'x' moves the panel edge, 'y' the timeline edge.
  function wire(el, axis) {
    if (!el) return;
    const cls = axis === 'x' ? 'v' : 'h';
    let drag = null, lastTap = 0;
    const current = () => apply()[axis === 'x' ? 'panel' : 'tl'];
    const set = px => {
      const lim = limits();
      if (axis === 'x') saved.panel = Math.round(clamp(px, PANEL.min, lim.panelMax));
      else saved.tl = Math.round(clamp(px, TL.min, lim.tlMax));
      apply();
    };
    const reset = () => { saved[axis === 'x' ? 'panel' : 'tl'] = axis === 'x' ? PANEL.def : TL.def; apply(); persist(); };
    el.addEventListener('pointerdown', e => {
      if (e.button !== undefined && e.button > 0) return;
      // Double click or double tap resets. (dblclick does not fire for touch.)
      // Only a tap that did not move counts, so two quick drags do not.
      const now = performance.now();
      if (now - lastTap < 350) { lastTap = 0; reset(); e.preventDefault(); return; }
      drag = { id: e.pointerId, start: axis === 'x' ? e.clientX : e.clientY, size: current(), moved: false };
      try { el.setPointerCapture(e.pointerId); } catch (_) { /* a synthetic pointer */ }
      el.classList.add('dragging');
      document.body.classList.add('resizing', cls);
      e.preventDefault();
    });
    el.addEventListener('pointermove', e => {
      if (!drag || e.pointerId !== drag.id) return;
      const d = (axis === 'x' ? e.clientX : e.clientY) - drag.start;
      if (Math.abs(d) > 4) drag.moved = true;
      // The panel grows when dragged left, the timeline when dragged up.
      set(drag.size - d);
    });
    const end = e => {
      if (!drag || (e && e.pointerId !== drag.id)) return;
      if (!drag.moved && e && e.type === 'pointerup') lastTap = performance.now();
      drag = null;
      el.classList.remove('dragging');
      document.body.classList.remove('resizing', cls);
      persist();
    };
    el.addEventListener('pointerup', end);
    el.addEventListener('pointercancel', end);
    el.addEventListener('lostpointercapture', () => end());
    el.addEventListener('dblclick', e => { e.preventDefault(); reset(); });
    el.addEventListener('keydown', e => {
      const step = e.shiftKey ? 40 : 10;
      let dir = 0;
      if (axis === 'x') dir = e.key === 'ArrowLeft' ? 1 : e.key === 'ArrowRight' ? -1 : 0;
      else dir = e.key === 'ArrowUp' ? 1 : e.key === 'ArrowDown' ? -1 : 0;
      if (dir) { set(current() + dir * step); persist(); e.preventDefault(); }
      else if (e.key === 'Home' || e.key === 'Enter') { reset(); e.preventDefault(); }
    });
  }

  // The toolbar can wrap onto more rows, which changes the room below it.
  let queued = 0;
  const soon = () => { cancelAnimationFrame(queued); queued = requestAnimationFrame(apply); };
  window.addEventListener('resize', soon);
  if (window.ResizeObserver) {
    const ro = new ResizeObserver(soon);
    ['.toolbar', '.stage', '.workspace', '.transport'].forEach(q => { const n = document.querySelector(q); if (n) ro.observe(n); });
  }
  wire(document.getElementById('split-v'), 'x');
  wire(document.getElementById('split-h'), 'y');
  apply();
  // For tests and for other scripts that need the numbers.
  window.BooyaLayout = { apply, limits, saved: () => ({ ...saved }), KEY, PANEL, TL };
})();
