// Drag the camera bubble on the preview to move it, scroll on it (or drag
// its grip) to resize it. Near a corner, an edge middle or the centre it
// snaps there. Loaded after main.js and campos.js and shares their globals.
(function () {
  'use strict';
  const handle = $('cam-handle'), grip = $('cam-grip'), img = $('preview');
  let drag = null;

  const NAMES = { 'top-left': 'top left', 'top-center': 'top middle', 'top-right': 'top right', 'left-center': 'left middle', 'right-center': 'right middle', 'bottom-left': 'bottom left', 'bottom-center': 'bottom middle', 'bottom-right': 'bottom right', center: 'the centre' };

  // The bubble shows on the preview only while it is still, in the bubble
  // layout, with the camera on and no drawing tool picked.
  function shown() {
    const w = project && project.webcam;
    if (!w || !w.visible || playing || img.hidden || !img.naturalWidth || (typeof shTool !== 'undefined' && shTool)) return false;
    return CamPos.layoutAt(project.layouts, baseLayout(), tl.outToSrc(tl.playhead)) === 'bubble';
  }

  function picture() {
    const a = img.getBoundingClientRect(), wr = img.parentElement.getBoundingClientRect();
    return { left: a.left - wr.left, top: a.top - wr.top, W: a.width, H: a.height, a };
  }

  function place(r) {
    const p = picture();
    Object.assign(handle.style, { left: p.left + r.x + 'px', top: p.top + r.y + 'px', width: r.w + 'px', height: r.h + 'px' });
    const shape = project.webcam.shape || (project.webcam.circle ? 'circle' : 'rounded-square');
    handle.dataset.shape = shape;
  }

  function update() {
    if (drag) return;
    const on = shown();
    handle.hidden = !on;
    if (!on) return;
    const p = picture();
    place(CamPos.bubbleRect(project.webcam, p.W, p.H));
    handle.setAttribute('aria-valuetext', project.webcam.position ? 'Placed by hand' : 'In the ' + NAMES[project.webcam.corner] + ' corner');
  }

  function commit(res, how) {
    const w = project.webcam;
    w.corner = res.corner;
    if (res.position) w.position = res.position; else delete w.position;
    $('cam-corner').value = w.corner;
    syncCustomOption();
    refresh(); checkpoint(); update();
    status(how || (res.snapped ? 'Camera snapped to ' + NAMES[res.snapped] + '.' : 'Camera moved. Drag it near a corner to snap it back.'), 'success');
  }

  // "Where you dragged it" is only in the list while the bubble has a dragged
  // spot. Added and removed rather than hidden, since WebKit shows hidden options.
  function syncCustomOption() {
    const sel = $('cam-corner'), has = !!(project && project.webcam && project.webcam.position);
    let opt = sel.querySelector('option[value="custom"]');
    if (has && !opt) { opt = document.createElement('option'); opt.value = 'custom'; opt.textContent = 'Where you dragged it'; sel.appendChild(opt); }
    if (!has && opt) opt.remove();
    if (has) sel.value = 'custom';
  }
  window.syncCamCustom = syncCustomOption;

  function setSize(size, quiet) {
    project.webcam.size = size;
    $('cam-size').value = size;
    refresh(); update();
    if (!quiet) scheduleCheckpoint();
  }

  handle.addEventListener('pointerdown', e => {
    if (e.button !== 0) return;
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    const p = picture(), r = CamPos.bubbleRect(project.webcam, p.W, p.H);
    const resizing = e.target === grip;
    drag = { id: e.pointerId, resizing, p, r, x0: e.clientX, y0: e.clientY, size0: project.webcam.size, cx: r.x + r.w / 2, cy: r.y + r.h / 2, moved: false };
    handle.classList.add('dragging');
  });
  handle.addEventListener('pointermove', e => {
    if (!drag || e.pointerId !== drag.id) return;
    const dx = e.clientX - drag.x0, dy = e.clientY - drag.y0;
    if (Math.hypot(dx, dy) > 2) drag.moved = true;
    const { p, r } = drag;
    if (drag.resizing) {
      // Distance from the centre sets the size, so the bubble grows round its middle.
      const short = Math.min(p.W, p.H), grow = (Math.max(dx, dy) * 2) / short;
      const size = CamPos.resize(drag.size0, grow);
      const s = CamPos.bubbleSize({ ...project.webcam, size }, p.W, p.H);
      place({ x: drag.cx - s.w / 2, y: drag.cy - s.h / 2, w: s.w, h: s.h });
      drag.size = size;
      return;
    }
    const cx = r.x + r.w / 2 + dx, cy = r.y + r.h / 2 + dy;
    const res = CamPos.drop(project.webcam, p.W, p.H, cx, cy);
    const at = CamPos.bubbleRect({ ...project.webcam, corner: res.corner, position: res.position }, p.W, p.H);
    place(at);
    handle.classList.toggle('snapped', !!res.snapped);
    drag.res = res;
  });
  function end(e, cancel) {
    if (!drag || (e && e.pointerId !== drag.id)) return;
    const d = drag; drag = null;
    handle.classList.remove('dragging', 'snapped');
    if (cancel || !d.moved) { update(); return; }
    if (d.resizing && d.size) { setSize(d.size, true); checkpoint(); status('Camera size ' + Math.round(d.size * 100) + '%.', 'success'); return; }
    if (d.res) commit(d.res);
  }
  handle.addEventListener('pointerup', e => end(e, false));
  handle.addEventListener('pointercancel', e => end(e, true));
  handle.addEventListener('lostpointercapture', e => drag && end(e, false));

  handle.addEventListener('wheel', e => {
    e.preventDefault();
    setSize(CamPos.resize(project.webcam.size, e.deltaY < 0 ? 0.01 : -0.01));
  }, { passive: false });

  handle.addEventListener('keydown', e => {
    if (drag && e.key === 'Escape') { end(null, true); return; }
    const step = e.shiftKey ? 0.05 : 0.01;
    if (e.key === '+' || e.key === '=') { e.preventDefault(); setSize(CamPos.resize(project.webcam.size, 0.01)); return; }
    if (e.key === '-' || e.key === '_') { e.preventDefault(); setSize(CamPos.resize(project.webcam.size, -0.01)); return; }
    const mv = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key];
    if (!mv) return;
    e.preventDefault(); e.stopPropagation();
    const p = picture(), r = CamPos.bubbleRect(project.webcam, p.W, p.H);
    const nx = (r.x + r.w / 2) / p.W + mv[0], ny = (r.y + r.h / 2) / p.H + mv[1];
    // Nudging never snaps, so small steps stay small.
    const w = project.webcam;
    w.position = [Math.round(Math.max(0, Math.min(1, nx)) * 1000) / 1000, Math.round(Math.max(0, Math.min(1, ny)) * 1000) / 1000];
    syncCustomOption();
    refresh(); scheduleCheckpoint(); update();
  });

  img.addEventListener('load', update);
  window.addEventListener('resize', update);
  if (window.ResizeObserver) new ResizeObserver(update).observe(img.parentElement);
  window.updateCamHandle = update;
})();
