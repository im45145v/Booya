// Shapes (arrows, boxes, ellipses, highlights) drawn on the preview, and the
// keystroke lane for the timeline. Loaded after main.js and shares its globals.
const SH_NAMES = { arrow: 'Arrow', box: 'Box', ellipse: 'Ellipse', highlight: 'Highlight' };
const SH_COLORS = ['#fbbf24', '#ef4444', '#22c55e', '#38bdf8', '#a78bfa', '#ffffff'];
let shTool = null, shColor = SH_COLORS[0], shDraft = null;

function renderShapes() {
  const box = $('sh-list'); if (!box || !project) return;
  const list = project.shapes || [];
  box.innerHTML = list.length ? '' : '<p class="muted small">No shapes yet.</p>';
  list.forEach((s, i) => {
    const d = document.createElement('div'); d.className = 'item' + (tl.selItem === s ? ' sel' : ''); d.tabIndex = 0;
    const dot = document.createElement('i'); dot.className = 'sh-dot'; dot.style.background = s.color;
    const label = document.createElement('span'); label.textContent = `${SH_NAMES[s.kind]} \u00b7 ${(s.start_ms / 1000).toFixed(1)}s to ${(s.end_ms / 1000).toFixed(1)}s`;
    const x = document.createElement('button'); x.className = 'btn icon ghost sm'; x.innerHTML = icon('i-x'); x.setAttribute('aria-label', 'Remove shape ' + (i + 1));
    x.onclick = e => { e.stopPropagation(); project.shapes.splice(i, 1); if (tl.selItem === s) tl.selItem = null; renderShapes(); tl.draw(); refresh(); };
    d.onclick = () => { tl.selItem = s; tl.playhead = tl.srcToOut(s.start_ms); tl.draw(); renderShapes(); refresh(); };
    const left = document.createElement('span'); left.className = 'sh-row'; left.append(dot, label);
    d.append(left, x); box.appendChild(d);
  });
}

function shSetTool(t) {
  shTool = t;
  document.querySelectorAll('#sh-tools [data-tool]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.tool === t)));
  const layer = $('shape-layer'); layer.hidden = !t;
  $('sh-hint').textContent = t ? 'Drag on the preview to draw. Esc cancels.' : 'Pick a tool, then drag on the preview. It is added at the playhead.';
  if (t) positionLayer();
}
function positionLayer() {
  const img = $('preview'), wrap = img.parentElement, layer = $('shape-layer');
  const a = img.getBoundingClientRect(), w = wrap.getBoundingClientRect();
  Object.assign(layer.style, { left: a.left - w.left + 'px', top: a.top - w.top + 'px', width: a.width + 'px', height: a.height + 'px' });
}
function shDraw() {
  const svg = $('shape-svg'); if (!shDraft) { svg.innerHTML = ''; return; }
  const { x1, y1, x2, y2 } = shDraft, c = shColor, W = svg.clientWidth, H = svg.clientHeight, lw = Math.max(2, 0.006 * Math.min(W, H));
  const X = v => v * W, Y = v => v * H;
  const bx = Math.min(X(x1), X(x2)), by = Math.min(Y(y1), Y(y2)), bw = Math.abs(X(x2) - X(x1)), bh = Math.abs(Y(y2) - Y(y1));
  let s = '';
  if (shTool === 'arrow') {
    const dx = X(x2) - X(x1), dy = Y(y2) - Y(y1), len = Math.hypot(dx, dy) || 1, ux = dx / len, uy = dy / len, head = Math.min(lw * 4.5, len * 0.6);
    const sx = X(x2) - ux * head * .8, sy = Y(y2) - uy * head * .8, nx = -uy, ny = ux;
    s = `<line x1="${X(x1)}" y1="${Y(y1)}" x2="${sx}" y2="${sy}" stroke="${c}" stroke-width="${lw}" stroke-linecap="round"/><polygon fill="${c}" points="${X(x2)},${Y(y2)} ${X(x2) - ux * head + nx * head / 2},${Y(y2) - uy * head + ny * head / 2} ${X(x2) - ux * head - nx * head / 2},${Y(y2) - uy * head - ny * head / 2}"/>`;
  } else if (shTool === 'ellipse') s = `<ellipse cx="${bx + bw / 2}" cy="${by + bh / 2}" rx="${bw / 2}" ry="${bh / 2}" fill="none" stroke="${c}" stroke-width="${lw}"/>`;
  else if (shTool === 'highlight') s = `<rect x="${bx}" y="${by}" width="${bw}" height="${bh}" rx="6" fill="${c}" fill-opacity=".38"/>`;
  else s = `<rect x="${bx}" y="${by}" width="${bw}" height="${bh}" rx="6" fill="none" stroke="${c}" stroke-width="${lw}"/>`;
  svg.innerHTML = s;
}
function shPoint(e) {
  const r = $('shape-layer').getBoundingClientRect();
  return [Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))];
}
function shFinish() {
  const d = shDraft; shDraft = null; shDraw();
  if (!d) return;
  const W = $('shape-svg').clientWidth, H = $('shape-svg').clientHeight, dx = (d.x2 - d.x1) * W, dy = (d.y2 - d.y1) * H;
  const ok = shTool === 'arrow' ? Math.hypot(dx, dy) > 24 : Math.abs(dx) > 14 && Math.abs(dy) > 14;
  if (!ok) return status('Drag a little further to draw the shape.');
  const s0 = tl.outToSrc(tl.playhead), len = +$('sh-len').value;
  const s = { kind: shTool, start_ms: s0, end_ms: Math.min(project.duration_ms, s0 + len), x1: d.x1, y1: d.y1, x2: d.x2, y2: d.y2, color: shColor, width: 0.006 };
  (project.shapes = project.shapes || []).push(s);
  tl.selItem = s; shSetTool(null); renderShapes(); tl.draw(); refresh(); status(`${SH_NAMES[s.kind]} added at the playhead. Drag it on the timeline to adjust.`, 'success');
}

(function initShapes() {
  const layer = $('shape-layer');
  layer.addEventListener('pointerdown', e => { e.preventDefault(); layer.setPointerCapture(e.pointerId); const [x, y] = shPoint(e); shDraft = { x1: x, y1: y, x2: x, y2: y }; shDraw(); });
  layer.addEventListener('pointermove', e => { if (!shDraft) return; [shDraft.x2, shDraft.y2] = shPoint(e); shDraw(); });
  layer.addEventListener('pointerup', shFinish);
  window.addEventListener('keydown', e => { if (e.key === 'Escape' && shTool) { shDraft = null; shDraw(); shSetTool(null); } });
  window.addEventListener('resize', () => shTool && positionLayer());
  document.querySelectorAll('#sh-tools [data-tool]').forEach(b => (b.onclick = () => { if (!project) return; shSetTool(shTool === b.dataset.tool ? null : b.dataset.tool); }));
  const sw = $('sh-colors');
  SH_COLORS.forEach(c => { const b = document.createElement('button'); b.className = 'swatch'; b.style.background = c; b.setAttribute('aria-label', 'Colour ' + c); b.setAttribute('aria-pressed', String(c === shColor));
    b.onclick = () => { shColor = c; sw.querySelectorAll('.swatch').forEach(x => x.setAttribute('aria-pressed', String(x === b))); if (tl.selItem && tl.selItem.kind) { tl.selItem.color = c; renderShapes(); refresh(); } };
    sw.appendChild(b); });
  // Keep lists in step with timeline edits and project swaps.
  const ss = syncSelection; syncSelection = function () { ss(); if (project) { renderShapes(); renderCallouts(); renderBlurList(); } };
  const sp = syncPanel; syncPanel = function () { sp(); loadKeyTrack(); renderShapes(); };
  ['keys-on', 'keys-short'].forEach(id => $(id).addEventListener('change', () => setTimeout(loadKeyTrack, 0)));
})();

// ---------- Keystroke lane ----------
let keyTrackSig = '';
async function loadKeyTrack() {
  if (!project) return;
  const sig = JSON.stringify([project.events, project.keys.shortcuts_only, project.keys.hold_ms]);
  if (sig === keyTrackSig) return;
  keyTrackSig = sig;
  try { tl.keyTrack = project.events ? await invoke('key_track', { project }) : []; } catch { tl.keyTrack = []; }
  tl.draw();
}
