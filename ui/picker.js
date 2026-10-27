// Visual source picker: screens and windows with live previews, app icons,
// search, and an area tab with drag to select and aspect ratio presets.
// Uses helpers from main.js ($, invoke, modal, status, fail, esc) at call time.

let recSource = { target: { type: 'screen', display: 0 }, label: 'Entire screen', kind: 'screen' };
function setRecSource(target, label, kind) {
  recSource = { target, label, kind };
  $('rec-source-label').textContent = label;
  $('rec-source').dataset.kind = kind;
  $('rec-source').title = 'What to record: ' + label + '. Click to change.';
}

const PK_ASPECTS = [['Free', 0], ['16:9', 16 / 9], ['4:3', 4 / 3], ['1:1', 1], ['4:5', 4 / 5], ['9:16', 9 / 16]];
const pkThumbs = new Map();
const pkQueue = [];
let pkActive = 0;

function pkPump() {
  while (pkActive < 2 && pkQueue.length) {
    const job = pkQueue.shift();
    pkActive++;
    invoke('capture_thumb', job.args).then(job.ok, job.fail).finally(() => { pkActive--; pkPump(); });
  }
}
const pkObserver = new IntersectionObserver(entries => entries.forEach(en => {
  if (!en.isIntersecting) return;
  pkObserver.unobserve(en.target);
  const card = en.target, s = JSON.parse(card.dataset.src), key = s.kind + ':' + s.id;
  const apply = t => {
    if (!card.isConnected) return;
    const th = card.querySelector('.pk-thumb');
    if (t && t.image) { th.style.backgroundImage = `url(${t.image})`; th.classList.add('has'); }
    else th.classList.add('none');
    if (t && t.icon) card.querySelector('.pk-icon').innerHTML = `<img src="${t.icon}" alt="" />`;
  };
  if (pkThumbs.has(key)) return apply(pkThumbs.get(key));
  pkQueue.push({
    args: { kind: s.kind, id: s.id, bundle: s.bundle, x: s.x, y: s.y, w: s.w, h: s.h },
    ok: t => { pkThumbs.set(key, t); apply(t); },
    fail: () => { pkThumbs.set(key, null); apply(null); },
  });
  pkPump();
}), { rootMargin: '120px' });

function pkHue(name) { let h = 0; for (const c of name || '?') h = (h * 31 + c.charCodeAt(0)) % 360; return h; }

function pkCard(s) {
  const el = document.createElement('button');
  el.className = 'pk-card'; el.type = 'button'; el.dataset.src = JSON.stringify(s);
  const name = s.kind === 'display' ? s.title : s.title;
  const sub = s.kind === 'display' ? `${s.w} x ${s.h}` : s.app;
  const initial = (s.app || s.title || '?').trim().charAt(0).toUpperCase();
  el.innerHTML = `<div class="pk-thumb"><span class="pk-skel"></span><span class="pk-none">${s.kind === 'display' ? '' : 'No preview'}</span></div>
    <div class="pk-meta"><span class="pk-icon${s.kind === 'display' ? ' disp' : ''}" style="--h:${pkHue(s.app)}">${s.kind === 'display' ? '<svg class="ico"><use href="#i-monitor"/></svg>' : esc(initial)}</span><span class="pk-text"><b>${esc(name)}</b><small>${esc(sub)}</small></span></div>`;
  el.onclick = () => {
    setRecSource(s.kind === 'display' ? { type: 'screen', display: +s.id } : { type: 'window', id: s.id }, s.kind === 'display' ? s.title : (s.app ? `${s.app}: ${s.title}` : s.title), s.kind === 'display' ? 'screen' : 'window');
    $('modal').close();
    status('Will record: ' + recSource.label + '. Press Record when you are ready.', 'info');
  };
  return el;
}

function pkRenderList(host, items, empty) {
  host.innerHTML = '';
  if (!items.length) { host.innerHTML = `<div class="pk-empty">${empty}</div>`; return; }
  items.forEach(s => { const c = pkCard(s); host.appendChild(c); pkObserver.observe(c); });
}

async function openPicker(tab = 'screens') {
  $('modal').classList.add('wide');
  $('modal').addEventListener('close', () => { $('modal').classList.remove('wide'); pkQueue.length = 0; }, { once: true });
  modal('What do you want to record?', `<div class="pk-bar">
      <div class="seg pk-tabs" role="tablist"><button role="tab" data-tab="screens">Screens</button><button role="tab" data-tab="windows">Windows</button><button role="tab" data-tab="area">Area</button></div>
      <input id="pk-search" class="field" type="search" placeholder="Search windows" aria-label="Search windows" />
      <button id="pk-refresh" class="btn ghost" title="Look again">Refresh</button>
    </div><div id="pk-body"></div>`);
  let sources = null, err = null;
  const body = $('pk-body');
  const show = t => {
    tab = t;
    document.querySelectorAll('.pk-tabs button').forEach(b => b.setAttribute('aria-checked', b.dataset.tab === t));
    $('pk-search').hidden = t !== 'windows';
    $('pk-refresh').hidden = t === 'area';
    if (t === 'area') return pkArea(body);
    if (err) return (body.innerHTML = `<div class="pk-empty"><b>Could not list what is on screen.</b><br>${esc(err)}</div>`);
    if (!sources) return (body.innerHTML = '<div class="pk-grid">' + '<div class="pk-card skel"><div class="pk-thumb"><span class="pk-skel"></span></div></div>'.repeat(3) + '</div>');
    body.innerHTML = '<div class="pk-grid" id="pk-grid"></div>';
    const q = $('pk-search').value.trim().toLowerCase();
    if (t === 'screens') pkRenderList($('pk-grid'), sources.filter(s => s.kind === 'display'), 'No screens were found.');
    else pkRenderList($('pk-grid'), sources.filter(s => s.kind === 'window' && (!q || (s.title + ' ' + s.app).toLowerCase().includes(q))),
      q ? 'No window matches that search.' : 'No windows found. Open the app you want to record, then press Refresh.');
  };
  const load = async () => {
    sources = null; err = null; show(tab);
    try { sources = await invoke('capture_sources'); } catch (e) { err = humanError(e); }
    if ($('pk-body') === body && $('modal').open) show(tab);
  };
  document.querySelectorAll('.pk-tabs button').forEach(b => (b.onclick = () => show(b.dataset.tab)));
  $('pk-search').oninput = () => show('windows');
  $('pk-refresh').onclick = () => { pkThumbs.clear(); load(); };
  if (tab === 'area') show('area');
  load();
}

// ----- Area: drag to select, aspect presets, numbers -----
async function pkArea(body) {
  body.innerHTML = '<div class="pk-empty">Taking a screenshot to pick from...</div>';
  let s;
  try { s = await invoke('screen_snapshot'); } catch (e) { body.innerHTML = `<div class="pk-empty"><b>Could not take a screenshot.</b><br>${esc(humanError(e))}</div>`; return; }
  if (!$('modal').open || !$('pk-body')) return;
  const W = s.w, H = s.h;
  let aspect = 0, sel = null;
  body.innerHTML = `<p class="muted small">Drag on the screenshot to choose the part to record. Drag the box to move it, or a corner to resize.</p>
    <div class="pk-aspects" role="radiogroup" aria-label="Aspect ratio">${PK_ASPECTS.map(([n], i) => `<button class="chip" role="radio" data-i="${i}" aria-checked="${i === 0}">${n}</button>`).join('')}
      <span class="spacer"></span><button class="chip" id="pk-full">Whole screen</button></div>
    <div class="draw pk-draw" id="pk-draw"><img src="${s.url}" alt="" draggable="false" /><div class="pk-sel" id="pk-sel" hidden><i data-h="nw"></i><i data-h="ne"></i><i data-h="sw"></i><i data-h="se"></i><span class="pk-size" id="pk-size"></span></div></div>
    <div class="row pk-nums"><label class="inline">X<input class="field" id="pk-x" type="number" min="0" /></label><label class="inline">Y<input class="field" id="pk-y" type="number" min="0" /></label><label class="inline">Width<input class="field" id="pk-w" type="number" min="2" /></label><label class="inline">Height<input class="field" id="pk-h" type="number" min="2" /></label>
      <span class="spacer"></span><button class="btn primary" id="pk-use" disabled>Record this area</button></div>`;
  const box = $('pk-draw'), el = $('pk-sel');
  const clampSel = r => {
    let { x, y, w, h } = r;
    w = Math.max(2, Math.min(W, Math.round(w))); h = Math.max(2, Math.min(H, Math.round(h)));
    x = Math.max(0, Math.min(W - w, Math.round(x))); y = Math.max(0, Math.min(H - h, Math.round(y)));
    return { x, y, w, h };
  };
  const paint = () => {
    el.hidden = !sel;
    $('pk-use').disabled = !sel || sel.w < 64 || sel.h < 64;
    if (!sel) return;
    Object.assign(el.style, { left: sel.x / W * 100 + '%', top: sel.y / H * 100 + '%', width: sel.w / W * 100 + '%', height: sel.h / H * 100 + '%' });
    $('pk-size').textContent = `${sel.w & ~1} x ${sel.h & ~1}`;
    ['x', 'y', 'w', 'h'].forEach(k => { if (document.activeElement !== $('pk-' + k)) $('pk-' + k).value = sel[k]; });
  };
  // A rectangle from a fixed corner (ax, ay) to the pointer, honouring the aspect lock and the bounds.
  const fromCorner = (ax, ay, px, py) => {
    const sx = px >= ax ? 1 : -1, sy = py >= ay ? 1 : -1;
    let w = Math.abs(px - ax), h = Math.abs(py - ay);
    if (aspect) { if (w / aspect > h) h = w / aspect; else w = h * aspect; }
    const maxW = sx > 0 ? W - ax : ax, maxH = sy > 0 ? H - ay : ay;
    let k = 1;
    if (w > maxW) k = Math.min(k, maxW / w);
    if (h > maxH) k = Math.min(k, maxH / h);
    if (aspect) { w *= k; h *= k; } else { w = Math.min(w, maxW); h = Math.min(h, maxH); }
    return { x: sx > 0 ? ax : ax - w, y: sy > 0 ? ay : ay - h, w, h };
  };
  const pt = e => { const b = box.getBoundingClientRect(); return [Math.max(0, Math.min(W, (e.clientX - b.left) / b.width * W)), Math.max(0, Math.min(H, (e.clientY - b.top) / b.height * H))]; };
  let drag = null;
  box.onpointerdown = e => {
    box.setPointerCapture(e.pointerId);
    const [px, py] = pt(e), h = e.target.dataset?.h;
    if (h && sel) {
      const ax = h.includes('w') ? sel.x + sel.w : sel.x, ay = h.startsWith('n') ? sel.y + sel.h : sel.y;
      drag = { mode: 'corner', ax, ay };
    } else if (sel && e.target === el) drag = { mode: 'move', ox: px - sel.x, oy: py - sel.y };
    else drag = { mode: 'corner', ax: px, ay: py, fresh: true };
  };
  box.onpointermove = e => {
    if (!drag) return;
    const [px, py] = pt(e);
    if (drag.mode === 'move') sel = clampSel({ ...sel, x: px - drag.ox, y: py - drag.oy });
    else { const r = fromCorner(drag.ax, drag.ay, px, py); if (r.w >= 4 && r.h >= 4) sel = r; }
    paint();
  };
  box.onpointerup = () => { if (sel) sel = clampSel(sel); drag = null; paint(); };
  document.querySelectorAll('.pk-aspects [data-i]').forEach(b => (b.onclick = () => {
    aspect = PK_ASPECTS[+b.dataset.i][1];
    document.querySelectorAll('.pk-aspects [data-i]').forEach(x => x.setAttribute('aria-checked', x === b));
    if (!sel) sel = aspect ? clampSel(fromCorner(W * 0.15, H * 0.15, W * 0.85, H * 0.85)) : null;
    else if (aspect) { const cx = sel.x + sel.w / 2, cy = sel.y + sel.h / 2; let w = sel.w, h = w / aspect; if (h > H) { h = H; w = h * aspect; } if (w > W) { w = W; h = w / aspect; } sel = clampSel({ x: cx - w / 2, y: cy - h / 2, w, h }); }
    paint();
  }));
  $('pk-full').onclick = () => { sel = { x: 0, y: 0, w: W & ~1, h: H & ~1 }; paint(); };
  ['x', 'y', 'w', 'h'].forEach(k => ($('pk-' + k).oninput = () => {
    const cur = sel || { x: 0, y: 0, w: Math.round(W / 2), h: Math.round(H / 2) };
    const v = +$('pk-' + k).value; if (!isFinite(v)) return;
    sel = clampSel({ ...cur, [k]: v }); paint();
  }));
  $('pk-use').onclick = () => {
    const r = { x: sel.x, y: sel.y, w: sel.w & ~1, h: sel.h & ~1 };
    setRecSource({ type: 'area', ...r }, `Area ${r.w} x ${r.h}`, 'area');
    $('modal').close();
    status(`Will record the ${r.w} x ${r.h} area. Press Record when you are ready.`, 'info');
  };
  paint();
}

$('rec-source').onclick = () => openPicker(recSource.kind === 'window' ? 'windows' : recSource.kind === 'area' ? 'area' : 'screens');
