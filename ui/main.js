// DemoLens editor UI. All pixels come from the Rust renderer via `preview`.
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = id => document.getElementById(id);
const esc = t => String(t).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const baseName = p => String(p).split(/[\\/]/).pop();
const stem = p => baseName(p).replace(/\.demolens\.json$|\.[^.]+$/, '');
const hex = rgb => '#' + rgb.map(v => v.toString(16).padStart(2, '0')).join('');
const rgb = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));
const icon = (id, cls = 'ico') => `<svg class="${cls}" aria-hidden="true"><use href="#${id}"/></svg>`;
const VIDEO_EXT = ['mkv', 'mp4', 'mov', 'webm', 'avi'];

let project = null, projectPath = null, playing = false, recording = false, busy = false, saved = null;
let pending = false, again = false, lastExport = null, models = [];

// File dialogs run in Rust, which then allows writes to exactly the chosen path.
const dialog = {
  open: o => invoke('pick_open', { name: o.filters[0].name, extensions: o.filters[0].extensions }),
  save: o => invoke('pick_save', { defaultPath: o.defaultPath || null, name: o.filters[0].name, extensions: o.filters[0].extensions }),
};

// ---------- Status bar: info, busy, success and error states ----------
function status(msg, kind = 'info') {
  $('status').textContent = msg;
  const bar = document.querySelector('.statusbar');
  bar.className = 'statusbar ' + kind;
  $('status-icon').innerHTML = `<use href="#${{ error: 'i-alert', busy: 'i-chip', success: 'i-check' }[kind] || 'i-check'}"/>`;
}
const fail = e => status(humanError(e), 'error');
function humanError(e) {
  const m = String(e?.message || e);
  if (/ffmpeg|ffprobe/i.test(m) && /not found|No such file|failed to (start|run)|program not found/i.test(m)) { checkFfmpeg(true); return 'ffmpeg is missing. See the dialog for the one line fix.'; }
  if (/Wayland/i.test(m)) return 'Screen capture on Wayland is not supported yet. Log in with an X11 session to record.';
  return m.replace(/^Error:\s*/, '');
}

// ---------- Undo / redo: whole project snapshots ----------
let history = [], hIdx = -1, cpTimer = null;
function updUndo() { $('undo-btn').disabled = hIdx <= 0; $('redo-btn').disabled = hIdx >= history.length - 1; updTitle(); }
function checkpoint() {
  if (!project) return;
  const s = JSON.stringify(project);
  if (history[hIdx] === s) return;
  history = history.slice(0, hIdx + 1); history.push(s);
  if (history.length > 200) history.shift();
  hIdx = history.length - 1; updUndo();
}
const scheduleCheckpoint = () => { clearTimeout(cpTimer); cpTimer = setTimeout(checkpoint, 350); };
function restore() { const t = tl.playhead; const prevSrc = project?.source; project = JSON.parse(history[hIdx]); tl.project = project; if (project.source !== prevSrc) { tl.setProject(project); prepareProxy(); } tl.playhead = Math.min(t, tl.outDur()); syncPanel(); tl.draw(); refresh(); updUndo(); }
function undo() { clearTimeout(cpTimer); checkpoint(); if (hIdx > 0) { hIdx--; restore(); status('Undone'); } }
function redo() { if (hIdx < history.length - 1) { hIdx++; restore(); status('Redone'); } }
function updTitle() {
  if (!project) { $('project-name').textContent = ''; return; }
  const dirty = saved !== null && history[hIdx] !== saved;
  $('project-name').innerHTML = esc(projectPath ? baseName(projectPath) : baseName(project.source) + ' (not saved)') + (dirty ? ' <span class="dirty" title="Unsaved changes">&bull;</span>' : '');
}

// ---------- Project lifecycle ----------
const tl = new Timeline($('timeline'), {
  onChange: () => { syncSelection(); refresh(); },
  onSeek: t => { tl.playhead = t; refresh(); },
  onSelect: () => { syncSelection(); showInspector(); },
});

function setProject(p, path = null) {
  project = p; projectPath = path; tl.selectedZoom = -1; tl.selectedLayout = -1; tl.selectedClip = 0; tl.playhead = 0; tl.setProject(p);
  history = [JSON.stringify(p)]; hIdx = 0; saved = path ? history[0] : null; updUndo();
  markDone(1); $('welcome').hidden = true; $('preview').hidden = false; $('tl-empty').hidden = true;
  $('ready-report').innerHTML = '';
  document.body.classList.add('has-project');
  enableControls(); syncPanel(); syncSelection(); refresh(); prepareProxy();
}
function applyResult(p) { const t = tl.playhead; project = p; tl.project = p; tl.playhead = Math.min(t, tl.outDur()); tl.fitted = true; syncPanel(); syncSelection(); tl.draw(); refresh(); checkpoint(); }
function enableControls() {
  document.querySelectorAll('.needs-project, .transport button').forEach(b => (b.disabled = !project || busy));
  $('ai-models').disabled = busy;
}

// ---------- Inspector <-> project ----------
function syncPanel() {
  const s = project.style, b = s.background;
  setBgType(b.type);
  if (b.type === 'gradient') { $('bg-from').value = hex(b.from); $('bg-to').value = hex(b.to); $('bg-angle').value = b.angle_deg; }
  if (b.type === 'solid') $('bg-from').value = hex(b.color);
  if (b.type === 'image') $('bg-image').value = b.path;
  setFit(project.style.bg_fit || 'fill'); $('bg-dim').value = project.style.bg_dim || 0;
  $('padding').value = s.padding; $('radius').value = s.corner_radius; $('shadow').value = s.shadow;
  { const g = s.glass || {}; $('gl-on').checked = !!g.enabled; $('gl-blur').value = g.blur ?? 0.018; $('gl-tint-op').value = g.tint_opacity ?? 0.16; $('gl-tint').value = hex(g.tint || [255, 255, 255]);
    $('gl-hi').value = g.highlight ?? 0.7; $('gl-radius').value = g.radius ?? 0.04; $('gl-inset').value = g.inset ?? 0.022; $('gl-over').checked = g.overlays ?? true; $('gl-opts').classList.toggle('off', !g.enabled); }
  const c = project.cursor;
  $('cur-visible').checked = c.visible; $('cur-size').value = c.size; $('cur-smooth').value = c.smoothing_ms; $('cur-ripple').checked = c.click_ripples;
  $('keys-on').checked = project.keys.enabled; $('keys-short').checked = project.keys.shortcuts_only; $('keys-sym').checked = project.keys.symbols !== false;
  $('cap-burn').checked = project.caption_style.burn_in; $('cap-size').value = project.caption_style.size;
  const w = project.webcam; $('cam-section').hidden = !w;
  if (w) { $('cam-visible').checked = w.visible; $('cam-circle').checked = w.circle; $('cam-size').value = w.size; $('cam-corner').value = w.corner; $('cam-layout').value = baseLayout(); $('cam-mirror').checked = !!w.mirror; const L = project.layouts || {}; $('lay-trans').value = L.transition_ms ?? 450; }
  $('lay-follow').checked = (project.layouts || {}).follow_cursor !== false;
  const wm = project.watermark || {}; $('wm-text').value = wm.text || ''; $('wm-corner').value = wm.corner || 'bottom-right'; $('wm-opacity').value = wm.opacity || 0.7;
  const ic = project.intro || {}, oc = project.outro || {};
  $('in-title').value = ic.title || ''; $('in-sub').value = ic.subtitle || ''; $('out-title').value = oc.title || ''; $('out-sub').value = oc.subtitle || '';
  $('card-len').value = ((ic.duration_ms || oc.duration_ms || 2500) / 1000);
  const au = project.audio;
  $('aud-denoise').checked = au.denoise; $('aud-mute').checked = au.mute; $('aud-normalize').checked = au.normalize;
  $('aud-gain').value = au.gain_db; $('aud-volume').value = au.volume; $('aud-fadein').value = au.fade_in_ms; $('aud-fadeout').value = au.fade_out_ms; $('aud-offset').value = au.offset_ms;
  const zc = project.zoom_config; $('zc-ease').value = zc.ease_in_ms; $('zc-level').value = zc.level; $('zc-steady').value = zc.steadiness ?? 0.8; $('zc-hold').value = zc.hold_ms ?? 2500; $('zc-merge').value = zc.merge_gap_ms ?? 3000; $('zc-style').value = zc.intensity || 'normal'; $('zc-clicks').value = zc.min_clicks ?? 3; $('zc-window').value = zc.click_window_ms ?? 4500;
  outputs();
  $('ex-aspect').value = project.export.aspect; $('ex-quality').value = project.export.quality; $('ex-fps').value = project.export.fps; $('ex-format').value = project.export.format;
  renderBlurList(); renderCallouts(); markTemplate(); markDestination();
}
// The layout of the whole recording: the chosen one, else what the older Layout setting meant.
function baseLayout() { const w = project.webcam, b = project.layouts && project.layouts.base; return b || (w && w.layout === 'side' ? 'split' : 'bubble'); }
function outputs() {
  const s = (id, t) => ($(id + '-out').textContent = t);
  s('zc-ease', (+$('zc-ease').value / 1000).toFixed(1) + ' s'); s('zc-level', (+$('zc-level').value).toFixed(1) + 'x'); s('zc-steady', Math.round(+$('zc-steady').value * 100) + '%'); s('zc-hold', (+$('zc-hold').value / 1000).toFixed(1) + ' s'); s('zc-merge', (+$('zc-merge').value / 1000).toFixed(1) + ' s'); s('zc-clicks', (+$('zc-clicks').value) + ' clicks'); s('zc-window', (+$('zc-window').value / 1000).toFixed(1) + ' s');
  s('card-len', (+$('card-len').value).toFixed(1) + ' s'); s('aud-gain', (+$('aud-gain').value) + ' dB'); s('aud-volume', Math.round(+$('aud-volume').value * 100) + '%');
  s('aud-fadein', (+$('aud-fadein').value / 1000).toFixed(1) + ' s'); s('aud-fadeout', (+$('aud-fadeout').value / 1000).toFixed(1) + ' s'); s('aud-offset', (+$('aud-offset').value) + ' ms');
}
function readPanel() {
  const t = $('bg-type').value, s = project.style;
  s.bg_fit = $('bg-fit-seg').dataset.fit || 'fill'; s.bg_dim = +$('bg-dim').value;
  s.background = t === 'gradient' ? { type: t, from: rgb($('bg-from').value), to: rgb($('bg-to').value), angle_deg: +$('bg-angle').value }
    : t === 'solid' ? { type: t, color: rgb($('bg-from').value) } : t === 'mesh' ? s.background : { type: t, path: $('bg-image').value };
  s.padding = +$('padding').value; s.corner_radius = +$('radius').value; s.shadow = +$('shadow').value;
  s.glass = { ...(s.glass || {}), enabled: $('gl-on').checked, blur: +$('gl-blur').value, tint: rgb($('gl-tint').value), tint_opacity: +$('gl-tint-op').value, highlight: +$('gl-hi').value, radius: +$('gl-radius').value, inset: +$('gl-inset').value, overlays: $('gl-over').checked };
  $('gl-opts').classList.toggle('off', !s.glass.enabled);
  Object.assign(project.cursor, { visible: $('cur-visible').checked, size: +$('cur-size').value, smoothing_ms: +$('cur-smooth').value, click_ripples: $('cur-ripple').checked });
  Object.assign(project.keys, { enabled: $('keys-on').checked, shortcuts_only: $('keys-short').checked, symbols: $('keys-sym').checked });
  Object.assign(project.caption_style, { burn_in: $('cap-burn').checked, size: +$('cap-size').value });
  if (project.webcam) Object.assign(project.webcam, { visible: $('cam-visible').checked, circle: $('cam-circle').checked, size: +$('cam-size').value, corner: $('cam-corner').value, layout: $('cam-layout').value === 'split' ? 'side' : 'bubble', mirror: $('cam-mirror').checked });
  { const L = project.layouts = project.layouts || { segments: [] }; L.follow_cursor = $('lay-follow').checked; }
  if (project.webcam) { const L = project.layouts; L.base = $('cam-layout').value; L.transition_ms = +$('lay-trans').value; $('lay-trans-out').textContent = (L.transition_ms / 1000).toFixed(2) + ' s'; }
  Object.assign(project.audio, { denoise: $('aud-denoise').checked, mute: $('aud-mute').checked, normalize: $('aud-normalize').checked, gain_db: +$('aud-gain').value, volume: +$('aud-volume').value, fade_in_ms: +$('aud-fadein').value, fade_out_ms: +$('aud-fadeout').value, offset_ms: +$('aud-offset').value });
  { const ms = +$('card-len').value * 1000, mk = (t, sub) => (t.trim() || sub.trim() ? { title: t.trim(), subtitle: sub.trim(), duration_ms: ms } : null);
    project.intro = mk($('in-title').value, $('in-sub').value); project.outro = mk($('out-title').value, $('out-sub').value); }
  { const t = $('wm-text').value.trim(); project.watermark = t ? { text: t, corner: $('wm-corner').value, opacity: +$('wm-opacity').value, size: (project.watermark && project.watermark.size) || 0.028 } : null; }
  Object.assign(project.zoom_config, { ease_in_ms: +$('zc-ease').value, transition_ms: +$('zc-ease').value, ease_out_ms: Math.round(+$('zc-ease').value * 0.9 / 100) * 100, level: +$('zc-level').value, steadiness: +$('zc-steady').value, hold_ms: +$('zc-hold').value, merge_gap_ms: +$('zc-merge').value, min_clicks: +$('zc-clicks').value, click_window_ms: +$('zc-window').value, intensity: $('zc-style').value || 'normal' });
  outputs();
  Object.assign(project.export, { aspect: $('ex-aspect').value, quality: +$('ex-quality').value, fps: +$('ex-fps').value, format: $('ex-format').value });
}
$('aud-mute').addEventListener('input', () => { if (audioState === 'ready') setAudioState('ready', audioText); });
document.querySelectorAll('#pane-style input, #pane-export select').forEach(el => el.addEventListener('input', () => { if (project) { readPanel(); markTemplate(); markDestination(); refresh(); } }));

function setBgType(t) {
  $('bg-type').value = t;
  document.querySelectorAll('.seg button[data-bg]').forEach(b => b.setAttribute('aria-checked', b.dataset.bg === t));
  $('bg-to-wrap').hidden = $('bg-angle-wrap').hidden = t !== 'gradient';
  $('bg-colors').hidden = t === 'image' || t === 'mesh'; $('bg-image-row').hidden = t !== 'image';
  if (typeof markWall === 'function') markWall();
}
document.querySelectorAll('.seg button[data-bg]').forEach(b => (b.onclick = () => { setBgType(b.dataset.bg); if (project) { readPanel(); refresh(); } }));
function setFit(f) { $('bg-fit-seg').dataset.fit = f; document.querySelectorAll('#bg-fit-seg button').forEach(b => b.setAttribute('aria-checked', b.dataset.fit === f)); }
document.querySelectorAll('#bg-fit-seg button').forEach(b => (b.onclick = () => { setFit(b.dataset.fit); if (project) { readPanel(); refresh(); } }));
async function useBackground(path) {
  try {
    const copy = await invoke('background_import', { path, projectSource: project.source });
    $('bg-image').value = copy; readPanel(); refresh(); loadRecents();
    status('Background photo added', 'success');
  } catch (err) { fail(err); }
}
$('bg-pick').onclick = async () => {
  if (!project) return;
  const p = await dialog.open({ multiple: false, filters: [{ name: 'Photo', extensions: ['png', 'jpg', 'jpeg', 'webp', 'heic', 'heif', 'bmp', 'gif'] }] });
  if (p) useBackground(p);
};
// Thumbnails load when a recent photo scrolls into view (the row is hidden until the Image background is chosen).
let recentsIO = null;
function recentsObserver() {
  return (recentsIO ||= new IntersectionObserver(entries => entries.forEach(en => {
    if (!en.isIntersecting) return;
    const b = en.target; recentsIO.unobserve(b);
    invoke('background_thumb', { path: b.dataset.path }).then(u => { b.style.backgroundImage = `url(${u})`; }).catch(() => { b.remove(); });
  }), { rootMargin: '80px' }));
}
async function loadRecents() {
  const box = $('bg-recents'); if (!box) return;
  const list = await invoke('background_recents').catch(() => []);
  box.innerHTML = '';
  for (const p of list) {
    const b = document.createElement('button'); b.className = 'recent'; b.title = baseName(p); b.setAttribute('aria-label', 'Use ' + baseName(p));
    b.onclick = () => { $('bg-image').value = p; setBgType('image'); readPanel(); refresh(); };
    box.appendChild(b);
    b.dataset.path = p; recentsObserver().observe(b);
  }
}
loadRecents();

function renderBlurList() {
  $('blur-list').innerHTML = project.blur.length ? '' : '<p class="muted small">No blur boxes yet. Blur secrets in the Auto edit tab finds them for you.</p>';
  project.blur.forEach((b, i) => {
    const d = document.createElement('div'); d.className = 'item';
    d.innerHTML = `<span>${b.x},${b.y} ${b.w}x${b.h} &middot; ${(b.start_ms / 1000).toFixed(1)}-${(b.end_ms / 1000).toFixed(1)}s</span>`;
    const x = document.createElement('button'); x.className = 'btn icon ghost sm'; x.innerHTML = icon('i-x'); x.setAttribute('aria-label', 'Remove blur box ' + (i + 1));
    x.onclick = () => { project.blur.splice(i, 1); renderBlurList(); refresh(); };
    d.appendChild(x); $('blur-list').appendChild(d);
  });
}

function renderCallouts() {
  const box = $('co-list'); box.innerHTML = project.callouts && project.callouts.length ? '' : '<p class="muted small">No callouts yet.</p>';
  (project.callouts || []).forEach((c, i) => {
    const d = document.createElement('div'); d.className = 'item';
    // The text is editable in place: change it and press Enter or click away.
    const label = document.createElement('input'); label.className = 'field sm grow'; label.value = c.text; label.maxLength = 80; label.setAttribute('aria-label', `Callout ${i + 1} text`); label.title = 'Edit the note. Enter or click away to save.';
    label.onchange = () => { c.text = label.value.trim() || c.text; label.value = c.text; refresh(); scheduleCheckpoint(); };
    label.onkeydown = e => { e.stopPropagation(); if (e.key === 'Enter') label.blur(); };
    const when = document.createElement('button'); when.className = 'btn ghost sm'; when.textContent = (c.start_ms / 1000).toFixed(1) + 's'; when.title = 'Jump to this callout';
    when.onclick = () => { tl.playhead = tl.srcToOut(c.start_ms + 1); tl.draw(); refresh(); };
    const x = document.createElement('button'); x.className = 'btn icon ghost sm'; x.innerHTML = icon('i-x'); x.setAttribute('aria-label', 'Remove callout ' + (i + 1));
    x.onclick = () => { project.callouts.splice(i, 1); renderCallouts(); refresh(); };
    d.append(label, when, x); box.appendChild(d);
  });
}
$('co-add').onclick = () => {
  const t = $('co-text').value.trim(); if (!project) return;
  if (!t) return status('Type the note first, for example: Click here', 'error');
  const s = tl.outToSrc(tl.playhead);
  (project.callouts = project.callouts || []).push({ text: t, start_ms: s, end_ms: Math.min(project.duration_ms, s + +$('co-len').value), x: 0.5, y: +$('co-pos').value });
  $('co-text').value = ''; renderCallouts(); refresh(); status('Callout added at the playhead.', 'success');
};

function syncSelection() {
  if (!project) return;
  const c = project.clips[tl.selectedClip];
  if (c) { $('clip-info').textContent = `Clip ${tl.selectedClip + 1} of ${project.clips.length}: ${fmt(c.src_start_ms)} to ${fmt(c.src_end_ms)} of the recording.`; $('speed').value = String(c.speed); }
  const z = project.zoom[tl.selectedZoom];
  $('zoom-scale').disabled = $('zoom-follow').disabled = !z;
  if (z) {
    $('zoom-info').textContent = `Zoom ${tl.selectedZoom + 1} of ${project.zoom.length}: ${fmt(z.start_ms)} to ${fmt(z.end_ms)} (${z.reason}).`;
    $('zoom-scale').value = z.scale; $('zoom-follow').checked = z.follow_cursor;
  } else $('zoom-info').textContent = project.zoom.length ? 'Select a zoom on the timeline. Drag it to move it, drag its edges to resize, double click the zoom lane to add one.' : 'No zooms yet. Press Z or double click the zoom lane to add one.';
  $('zoom-scale-out').textContent = (+$('zoom-scale').value).toFixed(1) + 'x';
  const g = project.webcam && project.layouts && (project.layouts.segments || [])[tl.selectedLayout];
  $('lay-kind').disabled = $('lay-del').disabled = !g;
  if (g) { $('lay-kind').value = g.kind; $('lay-info').textContent = `Layout change ${tl.selectedLayout + 1} of ${project.layouts.segments.length}: ${fmt(g.start_ms)} to ${fmt(g.end_ms)}. Drag it on the timeline to move it, drag its edges to resize.`; }
  else if ($('lay-info')) $('lay-info').textContent = project.webcam ? 'Double click the Layout lane on the timeline to use another layout for a stretch of the recording.' : 'This recording has no camera, so there is only the screen layout.';
}
$('lay-kind').onchange = () => { const g = project?.layouts?.segments?.[tl.selectedLayout]; if (g) { g.kind = $('lay-kind').value; tl.draw(); refresh(); checkpoint(); } };
$('lay-add').onclick = () => {
  if (!project?.webcam) return status('This recording has no camera.', 'info');
  const at = tl.outToSrc(tl.playhead);
  if (tl.addLayout(at, baseLayout() === 'camera' ? 'screen' : 'camera') < 0) status('There is no room for a layout change here. Move the playhead to a free spot.', 'info');
  else { syncSelection(); checkpoint(); }
};
$('lay-del').onclick = () => { const segs = project?.layouts?.segments; if (segs && tl.selectedLayout >= 0) { segs.splice(tl.selectedLayout, 1); tl.selectedLayout = -1; tl.draw(); syncSelection(); refresh(); checkpoint(); status('Layout change deleted.', 'success'); } };

// ---------- Preview and playback ----------
let frameLatency = null; // smoothed round trip of a preview frame, ms
async function refresh() {
  if (!project) return;
  if (!playing) scheduleCheckpoint();
  $('time').textContent = `${fmt(tl.playhead)} / ${fmt(tl.outDur())}`;
  if (pending) { again = true; return; }
  pending = true;
  try {
    const asked = performance.now();
    // While playing, ask for the frame the clock will have reached when this one is on screen.
    const url = await invoke('preview', { project, outMs: AvSync.frameTime(tl.playhead, frameLatency, tl.outDur(), playing), shortSide: playing ? 360 : 540, playing });
    if (playing) frameLatency = AvSync.nextLatency(frameLatency, performance.now() - asked);
    $('preview').src = url;
  }
  catch (e) { fail(e); }
  pending = false;
  if (again) { again = false; refresh(); }
}
function setPlaying(v) {
  playing = v;
  $('play').innerHTML = icon(v ? 'i-pause' : 'i-play'); $('play').setAttribute('aria-label', v ? 'Pause' : 'Play');
  if (v) playLoop();
}
async function playLoop() {
  // The clock starts when the sound really does, so they begin together.
  await audioStarted();
  if (!playing) return;
  let last = performance.now();
  if (tl.playhead >= tl.outDur() - 50) tl.playhead = 0;
  while (playing) {
    const now = performance.now();
    tl.playhead += now - last; last = now;
    if (tl.playhead >= tl.outDur()) { tl.playhead = tl.outDur(); setPlaying(false); }
    tl.follow(); tl.draw();
    pending = false; await refresh();
  }
}
// Stop playback when the window is hidden so it does not keep rendering frames
// nobody sees.
document.addEventListener('visibilitychange', () => { if (document.hidden && playing) setPlaying(false); });
const seek = t => { tl.playhead = Math.max(0, Math.min(tl.outDur(), t)); tl.follow(); tl.draw(); refresh(); };

// ---------- Editing ----------
function split() {
  const at = tl.outToSrc(tl.playhead), i = project.clips.findIndex(c => at > c.src_start_ms + 50 && at < c.src_end_ms - 50);
  if (i < 0) return status('Move the playhead inside a clip to split it.');
  const c = project.clips[i];
  project.clips.splice(i + 1, 0, { src_start_ms: at, src_end_ms: c.src_end_ms, speed: c.speed });
  c.src_end_ms = at; tl.selectedClip = i + 1; tl.draw(); syncSelection(); refresh(); status('Split clip ' + (i + 1));
}
function deleteSelected() {
  if (tl.selItem) {
    for (const k of ['callouts', 'shapes', 'blur']) { const i = (project[k] || []).indexOf(tl.selItem); if (i >= 0) { project[k].splice(i, 1); break; } }
    tl.selItem = null; status('Removed from the timeline'); tl.draw(); syncSelection(); refresh(); return;
  }
  if (tl.selectedLayout >= 0 && project.layouts?.segments?.[tl.selectedLayout]) { project.layouts.segments.splice(tl.selectedLayout, 1); tl.selectedLayout = -1; status('Layout change deleted'); }
  else if (tl.selectedZoom >= 0) { project.zoom.splice(tl.selectedZoom, 1); tl.selectedZoom = -1; status('Zoom deleted'); }
  else if (project.clips.length > 1) { project.clips.splice(tl.selectedClip, 1); tl.selectedClip = Math.max(0, tl.selectedClip - 1); tl.playhead = Math.min(tl.playhead, tl.outDur()); status('Clip deleted'); }
  else return status('A project needs at least one clip.');
  tl.draw(); syncSelection(); refresh();
}
function addZoom() {
  const at = tl.outToSrc(tl.playhead);
  project.zoom.push({ start_ms: at, end_ms: Math.min(project.duration_ms, at + 2000), scale: Math.min(2, project.zoom_config?.level || 1.5), follow_cursor: true, focus: [project.source_width / 2, project.source_height / 2], reason: 'manual' });
  project.zoom.sort((a, b) => a.start_ms - b.start_ms);
  tl.selectedZoom = project.zoom.findIndex(z => z.start_ms === at);
  tl.draw(); syncSelection(); refresh(); status('Zoom added. Drag it on the timeline to adjust.');
}
$('split').onclick = $('split2').onclick = () => project && split();
$('del-sel').onclick = () => project && deleteSelected();
$('del-clip').onclick = () => { if (project) { tl.selectedZoom = -1; deleteSelected(); } };
$('add-zoom').onclick = () => project && addZoom();
$('speed').onchange = () => { const c = project?.clips[tl.selectedClip]; if (c) { c.speed = +$('speed').value; tl.draw(); refresh(); } };
$('zoom-scale').oninput = () => { const z = project?.zoom[tl.selectedZoom]; $('zoom-scale-out').textContent = (+$('zoom-scale').value).toFixed(1) + 'x'; if (z) { z.scale = +$('zoom-scale').value; tl.draw(); refresh(); } };
// Pick the spot a zoom centres on by clicking a picture of the screen.
function pickPoint(title, url, w, h, hint, at0) {
  return new Promise(res => {
    modal(title, `<p class="muted small">${esc(hint)}</p><div class="draw" id="draw"><img src="${url}" alt="" draggable="false" /><div class="rect pt" id="pt-mark" style="width:14px;height:14px;border-radius:50%;transform:translate(-50%,-50%);left:${(at0[0] / w) * 100}%;top:${(at0[1] / h) * 100}%"></div></div>
      <div class="row end"><button class="btn ghost" id="pt-cancel">Cancel</button><button class="btn primary" id="pt-ok">Use this point</button></div>`);
    const box = $('draw'); let pt = at0, done = false;
    box.onpointerdown = e => { const b = box.getBoundingClientRect(); pt = [Math.round(Math.max(0, Math.min(1, (e.clientX - b.left) / b.width)) * w), Math.round(Math.max(0, Math.min(1, (e.clientY - b.top) / b.height)) * h)]; const m = $('pt-mark'); m.style.left = (pt[0] / w) * 100 + '%'; m.style.top = (pt[1] / h) * 100 + '%'; };
    const fin = v => { if (done) return; done = true; $('modal').close(); res(v); };
    $('pt-ok').onclick = () => fin(pt); $('pt-cancel').onclick = () => fin(null);
    $('modal').addEventListener('close', () => fin(null), { once: true });
  });
}
$('zoom-focus').onclick = async () => {
  const z = project?.zoom[tl.selectedZoom]; if (!z) return status('Select a zoom on the timeline first.', 'info');
  try {
    const f = await invoke('raw_frame', { project, srcMs: Math.min(project.duration_ms, (z.start_ms + z.end_ms) / 2) });
    const pt = await pickPoint('Where should this zoom look?', f.url, f.w, f.h, 'Click the spot to centre the zoom on. The picture is clamped so it never shows outside the screen.', z.focus);
    if (pt) { z.focus = pt; z.follow_cursor = false; $('zoom-follow').checked = false; tl.draw(); refresh(); checkpoint(); status('Zoom focus set. "Follow the cursor" is off for this zoom.', 'success'); }
  } catch (e) { fail(e); }
};
$('zoom-del').onclick = () => { if (project && tl.selectedZoom >= 0) { project.zoom.splice(tl.selectedZoom, 1); tl.selectedZoom = -1; tl.draw(); syncSelection(); refresh(); checkpoint(); status('Zoom deleted.', 'success'); } else status('Select a zoom on the timeline first.', 'info'); };
$('zoom-follow').onchange = () => { const z = project?.zoom[tl.selectedZoom]; if (z) { z.follow_cursor = $('zoom-follow').checked; refresh(); } };
$('zc-apply').onclick = () => $('auto-zoom').click();
// Mirrors ZoomConfig::with_intensity in crates/demolens-core/src/zoom.rs.
const ZOOM_STYLES = { subtle: { level: 1.5, hold: 2000, ease: 1000, clicks: 4 }, normal: { level: 1.6, hold: 2500, ease: 900, clicks: 3 }, lively: { level: 1.8, hold: 3000, ease: 800, clicks: 2 } };
$('zc-style').onchange = () => { const z = ZOOM_STYLES[$('zc-style').value]; if (!z || !project) return; $('zc-level').value = z.level; $('zc-hold').value = z.hold; $('zc-ease').value = z.ease; $('zc-clicks').value = z.clicks; readPanel(); outputs(); status('Zoom intensity set. Press "Apply to auto zoom" to rebuild the zooms.', 'info'); };
$('auto-zoom').onclick = async () => {
  try { project.zoom = await invoke('regenerate_zoom', { project }); tl.selectedZoom = -1; tl.draw(); syncSelection(); refresh(); status(project.zoom.length ? `${project.zoom.length} zoom${project.zoom.length === 1 ? '' : 's'} from clusters of clicks and typing` : (project.events ? `No zooms: nothing had ${project.zoom_config.min_clicks} clicks in one small area within ${(project.zoom_config.click_window_ms / 1000).toFixed(1)} s. Lower the clicks needed to zoom more, or add zooms by hand.` : 'No input log for this video, so there is nothing to zoom on.')); } catch (e) { fail(e); }
};
$('blur-add').onclick = () => {
  const v = $('blur-rect').value.split(',').map(Number);
  if (v.length !== 4 || v.some(isNaN)) return status('Type the blur box as x,y,w,h, for example 400,300,600,80', 'error');
  project.blur.push({ x: v[0], y: v[1], w: v[2], h: v[3], start_ms: tl.outToSrc(tl.playhead), end_ms: project.duration_ms, radius: 18 });
  $('blur-rect').value = ''; renderBlurList(); refresh();
};
$('tl-in').onclick = () => tl.zoomBy(1.6);
$('tl-out').onclick = () => tl.zoomBy(1 / 1.6);
$('tl-fit').onclick = () => tl.fit();
$('play').onclick = () => project && setPlaying(!playing);

// ---------- Open, save, sample ----------
async function openAny(path) {
  try {
    if (!path) path = await dialog.open({ multiple: false, filters: [{ name: 'Video or DemoLens project', extensions: [...VIDEO_EXT, 'json'] }] });
    if (!path) return;
    showBusy('Opening ' + baseName(path) + '...');
    if (path.endsWith('.json')) await openProjectPath(path);
    else {
      const events = path.replace(/\.[^.\\/]+$/, '.events.jsonl');
      let p; try { p = await invoke('open_recording', { video: path, events }); } catch { p = await invoke('open_recording', { video: path, events: null }); }
      setProject(p, null);
    }
    status('Opened ' + baseName(path), 'success');
  } catch (e) { fail(e); }
  hideBusy();
}
const isLooseProject = p => /\.json$/i.test(p || '');
async function save(as) {
  if (!project) return;
  try {
    let target = projectPath;
    if (as || !target || isLooseProject(target)) {
      // Projects are folders ending in .demolens, with the recording copied in.
      target = await dialog.save({ defaultPath: (!as && !isLooseProject(projectPath) && projectPath) || stem(project.source) + '.demolens', filters: [{ name: 'DemoLens project', extensions: ['demolens'] }] });
      if (!target) return;
    }
    readPanel(); checkpoint();
    showBusy('Saving the project folder...');
    try { await invoke('save_project_bundle', { path: target, project }); } finally { hideBusy(); }
    projectPath = target;
    saved = history[hIdx]; updTitle(); status('Saved ' + baseName(projectPath) + '. It is a folder with the recording inside, so you can move or copy it.', 'success');
    if (typeof renderRecent === 'function') renderRecent();
  } catch (e) { fail(e); }
}
async function trySample() {
  showBusy('Building the sample recording...');
  try { const r = await invoke('make_sample'); setProject(r.project, r.path); status('Sample loaded. Press Run auto edit to try it.', 'success'); selectTab('magic'); }
  catch (e) { fail(e); }
  hideBusy();
}
$('open-btn').onclick = () => openAny();
$('save-btn').onclick = e => save(e.shiftKey);
$('undo-btn').onclick = undo; $('redo-btn').onclick = redo;
$('w-open').onclick = () => openAny();
$('w-sample').onclick = trySample;
$('w-record').onclick = () => $('rec-btn').click();
$('w-models').onclick = () => showModels();
$('w-keys').onclick = () => showShortcuts();

function showBusy(t) { $('busy-text').textContent = t; $('busy-overlay').hidden = false; status(t, 'busy'); }
function hideBusy() { $('busy-overlay').hidden = true; }

// ---------- Recording ----------
document.querySelectorAll('.toggle').forEach(b => (b.onclick = () => b.setAttribute('aria-pressed', b.getAttribute('aria-pressed') !== 'true')));
// Key capture is on unless it was turned off. Turning it on explains what is
// kept and, on a Mac, asks for the Input Monitoring permission it needs.
$('rec-keys').setAttribute('aria-pressed', localStorage.getItem('rec-keys') === '0' ? 'false' : 'true');
async function keysPermissionCheck(prompt) {
  if (!/Mac/i.test(navigator.platform)) return true;
  let ok = true;
  try { ok = await invoke('input_monitoring_status', { prompt }); } catch { return true; }
  if (!ok) status('Keys need Input Monitoring. In System Settings > Privacy & Security > Input Monitoring switch DemoLens on, then quit and reopen DemoLens. Until then shortcuts such as Cmd+Tab are not shown.', 'error');
  return ok;
}
$('rec-keys').addEventListener('click', async () => {
  const on = $('rec-keys').getAttribute('aria-pressed') === 'true';
  localStorage.setItem('rec-keys', on ? '1' : '0');
  if (!on) return;
  const ok = await keysPermissionCheck(true);
  if (!ok) { try { await invoke('open_input_monitoring_settings'); } catch { } return; }
  status('Key capture is on. Only shortcuts and the timing of typing are kept, never the letters.' + (IS_MAC ? '' : ' Password fields are not special-cased, so turn it off while typing secrets.'), 'info');
});
if (localStorage.getItem('rec-keys') !== '0') setTimeout(() => keysPermissionCheck(false), 1500);
function setRecUi(on) {
  recording = on;
  $('rec-btn').classList.toggle('recording', on);
  $('rec-btn').innerHTML = icon(on ? 'i-stop' : 'i-record') + `<span>${on ? 'Stop' : 'Record'}</span>`;
  try { applyShortcutHints(); } catch { }
  $('rec-pause').hidden = !on; if (!on) setPausedUi(false);
}
function setPausedUi(p) { $('rec-pause').innerHTML = icon(p ? 'i-play' : 'i-pause') + `<span>${p ? 'Resume' : 'Pause'}</span>`; $('rec-pause').dataset.paused = p; }
$('rec-pause').onclick = () => invoke('pause_recording', { paused: $('rec-pause').dataset.paused !== 'true' }).catch(fail);
window.__TAURI__.event.listen('rec-paused', e => { setPausedUi(!!e.payload); if (recording) status(e.payload ? 'Paused. Input capture is stopped and this part will be cut.' : 'Recording.', 'busy'); });
$('rec-btn').onclick = async () => {
  if ($('rec-btn').disabled) return;
  try {
    if (!recording) {
      const target = recSource.target;
      $('rec-btn').disabled = true; status('Starting in 3 seconds. Click the number to cancel. If macOS asks for permission, allow it.', 'busy');
      await meterStopAndWait();
      showRecWarn([]);
      let warn;
      try { warn = await invoke('start_recording', { target, mic: $('rec-mic').getAttribute('aria-pressed') === 'true', systemAudio: $('rec-sys').getAttribute('aria-pressed') === 'true', keys: $('rec-keys').getAttribute('aria-pressed') === 'true' ? 'on' : 'off', webcam: $('rec-cam').getAttribute('aria-pressed') === 'true', micDevice: $('rec-mic-dev').value || null, maxHeight: $('rec-res').value === '' ? null : +$('rec-res').value, countdown: 3 }); }
      finally { $('rec-btn').disabled = false; }
      setRecUi(true); localStorage.setItem('mic-ok', '1');
      if (!IS_WIN && $('rec-mic').getAttribute('aria-pressed') === 'true') meterStart();
      const w = warn.filter(x => x !== 'recording without audio');
      showRecWarn(w);
      checkMicSoon();
      status(w.length ? 'Recording. ' + w.join(' ') : 'Recording. Press Stop or Ctrl+R when you are done.', w.length ? 'error' : 'busy');
    } else {
      setRecUi(false); meterStop(); $('rec-btn').disabled = true; showBusy('Finishing the recording...');
      const ins = typeof takeInsertArgs === 'function' ? takeInsertArgs() : null;
      try {
        const p = await invoke('stop_recording', { insert: ins });
        micWatch = 0;
        if (ins) { applySplice(p, ins); } else setProject(p, null);
      } finally { $('rec-btn').disabled = false; }
      if (!ins && $('rec-warn').hidden) status('Recording loaded. Try Run auto edit.', 'success'); setTimeout(() => meterStart(), 500);
      hideBusy();
    }
  } catch (e) { setRecUi(false); hideBusy(); if (typeof clearTakePlan === 'function') clearTakePlan(); if (!/cancelled/i.test(String(e))) fail(e); else status('Recording cancelled.', 'info'); }
};

// Selecting a zoom or a note on the timeline opens its settings (the Edit tab),
// so the controls for what you clicked are never hidden.
let lastPicked = null;
function showInspector() {
  const pick = tl.selectedZoom >= 0 ? 'z' + tl.selectedZoom : tl.selItem ? 'i' + ['callouts', 'shapes', 'blur'].map(k => (tl.project[k] || []).indexOf(tl.selItem)).join(':') : null;
  if (pick && pick !== lastPicked && typeof selectTab === 'function') selectTab('edit');
  lastPicked = pick;
}

// ---------- Tabs ----------
function selectTab(name) {
  document.querySelectorAll('.tab').forEach(t => { const on = t.id === 'tab-' + name; t.setAttribute('aria-selected', on); t.tabIndex = on ? 0 : -1; });
  document.querySelectorAll('.pane').forEach(p => (p.hidden = p.id !== 'pane-' + name));
}
document.querySelectorAll('.tab').forEach((t, i, all) => {
  t.onclick = () => selectTab(t.id.slice(4));
  t.onkeydown = e => { if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') { const n = all[(i + (e.key === 'ArrowRight' ? 1 : all.length - 1)) % all.length]; n.click(); n.focus(); } };
});

// ---------- Templates ----------
let templates = { builtin: [], user: [] };
const bgCss = b => b.type === 'gradient' ? `linear-gradient(${90 + b.angle_deg}deg, ${hex(b.from)}, ${hex(b.to)})` : b.type === 'solid' ? hex(b.color) : 'var(--surface-3)';
async function loadTemplates() {
  templates = await invoke('list_presets');
  const all = [...templates.builtin, ...templates.user];
  $('templates').innerHTML = all.map((p, i) => `<button class="tpl needs-project" data-i="${i}" title="Apply ${esc(p.name)}"><span class="sw" style="background:${bgCss(p.style.background)}"><i style="border-radius:${Math.round(p.style.corner_radius * 300)}px"></i></span>${esc(p.name)}</button>`).join('');
  document.querySelectorAll('.tpl').forEach(b => (b.onclick = async () => {
    if (!project) return;
    applyResult(await invoke('apply_preset', { name: all[+b.dataset.i].name, project }));
    status('Applied the ' + all[+b.dataset.i].name + ' look', 'success');
  }));
  enableControls(); if (project) markTemplate();
}
function markTemplate() {
  const all = [...templates.builtin, ...templates.user], cur = JSON.stringify(project.style);
  document.querySelectorAll('.tpl').forEach(b => b.classList.toggle('active', JSON.stringify(all[+b.dataset.i].style) === cur));
}
$('preset-save').onclick = async () => {
  const name = $('preset-name').value.trim();
  if (!name) return status('Give the template a name first, for example your product name.', 'error');
  try { readPanel(); await invoke('save_preset', { name, project }); $('preset-name').value = ''; await loadTemplates(); status('Saved template ' + name, 'success'); } catch (e) { fail(e); }
};

// ---------- Export ----------
const DESTS = [
  { id: 'youtube', name: 'YouTube', note: '16:9 1080p MP4', aspect: '16:9', quality: 1080, fps: 30, format: 'mp4' },
  { id: 'x', name: 'X / LinkedIn', note: '16:9 1080p MP4', aspect: '16:9', quality: 1080, fps: 30, format: 'mp4', alt: true },
  { id: 'square', name: 'Feed post', note: '1:1 1080p MP4', aspect: '1:1', quality: 1080, fps: 30, format: 'mp4' },
  { id: 'reels', name: 'Reels / Shorts / TikTok', note: '9:16 1080p MP4', aspect: '9:16', quality: 1080, fps: 30, format: 'mp4' },
  { id: 'gif', name: 'README or PR', note: 'GIF 480p 15 fps', aspect: '16:9', quality: 480, fps: 15, format: 'gif' },
  { id: '4k', name: 'Product site', note: '16:9 4K 60 fps', aspect: '16:9', quality: 2160, fps: 60, format: 'mp4' },
];
$('destinations').innerHTML = DESTS.map(d => `<button data-d="${d.id}" aria-pressed="false">${esc(d.name)}<small>${esc(d.note)}</small></button>`).join('');
document.querySelectorAll('#destinations button').forEach(b => (b.onclick = () => {
  const d = DESTS.find(x => x.id === b.dataset.d);
  $('ex-aspect').value = d.aspect; $('ex-quality').value = d.quality; $('ex-fps').value = d.fps; $('ex-format').value = d.format;
  if (project) { readPanel(); refresh(); }
  document.querySelectorAll('#destinations button').forEach(x => x.setAttribute('aria-pressed', x === b));
}));
function markDestination() {
  const e = project.export, first = DESTS.find(d => d.aspect === e.aspect && d.quality === e.quality && d.fps === e.fps && d.format === e.format);
  document.querySelectorAll('#destinations button').forEach(x => x.setAttribute('aria-pressed', !!first && x.dataset.d === first.id));
}
// First-run checklist on the welcome card. Remembered on this computer only.
function markDone(n) { try { localStorage.setItem('dl.step' + n, '1'); } catch (_) {} renderSteps(); }
function renderSteps() {
  document.querySelectorAll('.steps li').forEach((li, i) => {
    let d = false; try { d = localStorage.getItem('dl.step' + (i + 1)) === '1'; } catch (_) {}
    li.classList.toggle('done', d); const b = li.querySelector('b'); if (b) b.textContent = d ? '\u2713' : String(i + 1);
  });
}
renderSteps();

async function doExport() {
  if (!project || busy) return;
  selectTab('export'); readPanel();
  const fmtx = project.export.format, names = { gif: 'GIF', webm: 'WebM video', mp4: 'MP4 video' };
  if (project.blur.length && !(await reviewWarning())) return;
  const out = await dialog.save({ defaultPath: stem(projectPath || project.source) + '-demo.' + fmtx, filters: [{ name: names[fmtx] || 'Video', extensions: [fmtx] }] });
  if (!out) return;
  busy = true; enableControls();
  $('ex-state').hidden = false; $('ex-reveal').hidden = true; $('ex-bar').style.width = '0%'; $('ex-text').textContent = 'Starting...';
  status('Exporting ' + baseName(out) + '...', 'busy');
  const t0 = performance.now();
  try {
    const r = await invoke('export_video', { project, out });
    lastExport = out; $('ex-bar').style.width = '100%';
    $('ex-text').textContent = `Done in ${((performance.now() - t0) / 1000).toFixed(0)} s: ${baseName(out)}`; $('ex-reveal').hidden = false; $('ex-upload').hidden = !storageReady;
    markDone(3); status('Exported ' + r, 'success');
  } catch (e) { $('ex-text').textContent = 'Export failed'; fail(e); }
  busy = false; enableControls();
}
window.__TAURI__.event.listen('export-progress', e => { $('ex-bar').style.width = (e.payload * 100).toFixed(1) + '%'; $('ex-text').textContent = `Rendering ${(e.payload * 100).toFixed(0)}%`; });
$('ex-btn').onclick = doExport;
$('ex-top').onclick = () => { selectTab('export'); $('ex-btn').focus(); };
$('ex-reveal').onclick = () => lastExport && invoke('reveal', { path: lastExport }).catch(fail);

// ---------- Local AI ----------
function modal(title, html) { $('modal-title').textContent = title; $('modal-body').innerHTML = html; if (!$('modal').open) $('modal').showModal(); }
$('modal-close').onclick = () => $('modal').close();

async function aiTask(label, fn) {
  if (!project || busy) return;
  if (playing) setPlaying(false);
  busy = true; readPanel(); checkpoint(); enableControls();
  $('ai-bar-wrap').hidden = false; $('ai-bar-wrap').classList.add('indet'); status(label + '...', 'busy');
  try { await fn(); markDone(2); } catch (e) {
    fail(e);
    if (/model|install|download/i.test(String(e))) showModels(humanError(e));
  }
  busy = false; $('ai-bar-wrap').hidden = true; enableControls(); refreshModels(); loadAiStatus();
}
window.__TAURI__.event.listen('ai-progress', e => {
  const p = e.payload;
  if (p.task.startsWith('download:')) {
    const id = p.task.slice(9), bar = document.querySelector(`.bar[data-model="${id}"]`);
    if (bar) { bar.hidden = false; bar.firstElementChild.style.width = (p.progress * 100) + '%'; }
    const lbl = document.querySelector(`small[data-model="${id}"]`); if (lbl) lbl.textContent = 'Downloading ' + p.message;
  } else { $('ai-bar-wrap').classList.remove('indet'); $('ai-bar').style.width = (p.progress * 100) + '%'; status(p.message, 'busy'); }
});

$('ready-btn').onclick = () => aiTask('Running auto edit', async () => {
  const r = await invoke('ai_demo_ready', { project, zoom: $('r-zoom').checked, captions: $('r-captions').checked, cut: $('r-cut').checked, cutPauses: $('r-pauses').checked, blur: $('r-blur').checked, denoise: $('r-denoise').checked });
  applyResult(r.project);
  const ic = { done: 'i-check', skipped: 'i-skip', failed: 'i-alert' };
  $('ready-report').innerHTML = r.report.steps.map(s => `<li class="${s.state}">${icon(ic[s.state])}<span><b>${esc(s.name)}</b><small>${esc(s.detail)}</small></span></li>`).join('') +
    `<li class="done">${icon('i-check')}<span><b>Length</b><small>${fmt(r.report.before_ms)} to ${fmt(r.report.after_ms)}. Ctrl+Z undoes everything at once.</small></span></li>`;
  const skipped = r.report.steps.filter(s => s.state !== 'done').length;
  status(skipped ? `Auto edit done, ${skipped} step(s) skipped. The Auto edit tab says why.` : 'Auto edit done. Press Space to watch it, then Export.', 'success');
});
$('ai-transcribe').onclick = () => aiTask('Transcribing', async () => {
  const cloud = await confirmCloud('stt');
  if (cloud === null) return status('Cancelled. Nothing was sent.', 'info');
  const r = await invoke('ai_transcribe', { project, model: null, cloud });
  const rp = r.report;
  if (!rp.words) {
    applyResult(r.project);
    const why = { no_audio: 'This recording has no sound track, so there is nothing to caption.', too_quiet: 'The microphone only picked up a faint noise floor, so there is nothing to caption.' }[rp.reason] || 'No speech was heard, so no captions were added.';
    return status(why, 'info');
  }
  r.project.caption_style.burn_in = true; applyResult(r.project);
  status(`Transcribed ${rp.words} words (${rp.fillers} filler words). Captions are burned in; switch that off in Style.`, 'success');
});
// Auto cut proposes first. Nothing is removed until the red bands on the timeline have been reviewed.
function clearProposal() { tl.proposed = null; $('cut-review').hidden = true; tl.draw(); }
function showProposal(cuts, had) {
  const total = cuts.reduce((a, c) => a + (c[1] - c[0]), 0), f = cuts.filter(c => c[2] === 'filler').length;
  tl.proposed = cuts.map(c => [c[0], c[1]]);
  $('cut-review-text').textContent = `${cuts.length} proposed cut${cuts.length === 1 ? '' : 's'} (${f} filler words, ${cuts.length - f} still pauses), ${fmt(total)} in all. Red bands on the timeline show them.`;
  $('cut-review').hidden = false; tl.draw();
  if (cuts[0]) { tl.playhead = Math.max(0, tl.srcToOut(cuts[0][0]) - 800); tl.draw(); refresh(); }
}
let proposal = null;
$('ai-autocut').onclick = () => aiTask('Finding pauses and filler words', async () => {
  const fillers = $('cut-fillers').checked, silences = $('cut-silences').checked;
  const args = { fillers, silences, minSilenceMs: +$('cut-min').value };
  const r = await invoke('ai_autocut', { project, ...args, apply: false });
  if (!r.report.cuts.length) {
    clearProposal(); proposal = null;
    return status(silences ? 'Nothing to cut. Every quiet stretch has something happening on screen, so all of it is kept.' : 'No filler words found.' + (!r.report.had_transcript && fillers ? ' Transcribe first so filler words can be found.' : ''), 'info');
  }
  proposal = args; showProposal(r.report.cuts, r.report.had_transcript);
  status('Review the red bands on the timeline, then press Apply cuts. Nothing has been removed yet.', 'info');
});
$('cut-apply').onclick = () => aiTask('Applying cuts', async () => {
  const r = await invoke('ai_autocut', { project, ...proposal, apply: true });
  clearProposal(); applyResult(r.project);
  status(`Removed ${r.report.cuts.length} cut${r.report.cuts.length === 1 ? '' : 's'}: ${fmt(r.report.before_ms)} to ${fmt(r.report.after_ms)}. Press Ctrl+Z to put them back.`, 'success');
});
$('cut-discard').onclick = () => { clearProposal(); status('Proposed cuts discarded. The video is unchanged.', 'info'); };
$('ai-redact').onclick = () => aiTask('Scanning for secrets', async () => {
  const r = await invoke('ai_redact', { project });
  applyResult(r.project);
  status(r.report.found.length ? `Found ${r.report.found.length} secret${r.report.found.length === 1 ? '' : 's'} (${[...new Set(r.report.found.map(f => f[0]))].join(', ')}) and blurred ${r.report.added} region${r.report.added === 1 ? '' : 's'}.` : 'Found 0 secrets on the sampled frames. Still watch the export before sharing.', 'success');
});
$('ai-interactive').onclick = async () => {
  if (!project || busy) return;
  readPanel();
  if (!project.events) return status('This recording has no click log, so there is nothing to click through. Record with DemoLens to get one.', 'error');
  const ok = await confirmBox('Interactive demo', `<p>Makes one HTML file: a screenshot for every click, a pulsing dot where you clicked, and a short caption from your narration. It works offline and uploads nothing.</p>
    ${project.blur.length ? '<p class="found">Your blurred regions are applied to every screenshot.</p>' : '<p class="warn-note">No blurred regions yet. Run "Blur secrets" first if the screen shows anything private.</p>'}
    <label class="stack">Title <input class="field" id="ix-title" placeholder="Leave empty to write one from the recording" /></label>
    <label class="stack">Button text (optional) <input class="field" id="ix-cta" placeholder="Start free trial" /></label>
    <label class="stack">Button link (optional, https://...) <input class="field" id="ix-url" placeholder="https://example.com/signup" /></label>
    <label><input type="checkbox" id="ix-badge" checked /> Show "Made with DemoLens"</label>`, 'Choose where to save');
  if (!ok) return;
  const v = id => ($(id) && $(id).value || '').trim();
  const title = v('ix-title'), cta = v('ix-cta'), url = v('ix-url'), badge = $('ix-badge') ? $('ix-badge').checked : true;
  const out = await dialog.save({ defaultPath: stem(projectPath || project.source) + '-interactive.html', filters: [{ name: 'Interactive demo', extensions: ['html'] }] });
  if (!out) return;
  aiTask('Building the interactive demo', async () => {
    const r = await invoke('export_interactive', { project, out, title, ctaText: cta, ctaUrl: url, badge });
    lastExport = out;
    status('Interactive demo saved: ' + baseName(out) + ' (' + r + ')', 'success');
    if (storageReady) await uploadExport(out);
    else status('Interactive demo saved: ' + baseName(out) + ' (' + r + '). To share it with a link, add a bucket in Settings > Upload (S3).', 'success');
  });
};
$('ai-docs').onclick = () => aiTask('Writing docs', async () => {
  const cloud = $('docs-llm').checked ? await confirmCloud('llm') : false;
  if (cloud === null) return status('Cancelled. Nothing was sent.', 'info');
  let r;
  try { r = await invoke('ai_docs', { project, useLlm: $('docs-llm').checked, cloud }); }
  catch (err) {
    const m = String(err).match(/REMOTE_LLM:(\S+)/);
    if (!m) throw err;
    const ok = await confirmBox('Send the transcript to another computer?', `<p>DEMOLENS_LLM_URL points at <b>${esc(m[1])}</b>, which is not this computer. The transcript text and chapter timings would be sent there.</p>`, 'Send it');
    if (!ok) return status('Cancelled. Nothing was sent.', 'info');
    r = await invoke('ai_docs', { project, useLlm: true, cloud, confirmRemote: true });
  }
  status('Docs written by the ' + r.docs.generated_by, 'success');
  modal('Demo docs', `<p>Written by ${esc(r.docs.generated_by)}.</p>
    <div class="row"><button class="btn" id="doc-save">${icon('i-save')}<span>Save guide.md...</span></button><button class="btn" id="doc-copy-ch">Copy chapters</button><button class="btn" id="doc-copy-post">Copy post</button>${cloudState.choice?.tts ? '<button class="btn" id="doc-tts">Voiceover (cloud)...</button>' : ''}</div>
    <pre class="doc">${esc(r.markdown)}</pre>`);
  $('doc-save').onclick = async () => { const p = await dialog.save({ defaultPath: 'guide.md', filters: [{ name: 'Markdown', extensions: ['md'] }] }); if (p) { await invoke('write_text', { path: p, text: r.markdown }); status('Saved ' + baseName(p), 'success'); } };
  $('doc-copy-ch').onclick = () => navigator.clipboard.writeText(r.chapters).then(() => status('Chapters copied', 'success'));
  if ($('doc-tts')) $('doc-tts').onclick = () => cloudVoiceover(r.docs);
  $('doc-copy-post').onclick = () => navigator.clipboard.writeText(r.docs.social_post).then(() => status('Post copied', 'success'));
});
$('cap-export').onclick = async () => {
  if (!project) return;
  if (!project.captions.length) return status('Transcribe first to get subtitles.', 'error');
  const p = await dialog.save({ defaultPath: stem(project.source) + '.srt', filters: [{ name: 'Subtitles', extensions: ['srt', 'vtt'] }] });
  if (p) { try { await invoke('export_captions', { project, path: p }); status('Wrote ' + baseName(p), 'success'); } catch (e) { fail(e); } }
};

async function refreshModels() {
  loadAiStatus();
  try { models = (await invoke('models_list')).models; } catch { return; }
  const has = k => models.some(m => m.kind === k && m.installed);
  const ocr = models.filter(m => m.kind === 'ocr').every(m => m.installed);
  const pill = (el, ok, label) => { el.className = 'pill ' + (ok ? 'ok' : 'missing'); el.textContent = ok ? 'ready' : label; el.title = ok ? 'Model installed' : 'Click to download the model'; el.onclick = ok ? null : e => { e.preventDefault(); showModels(); }; };
  pill($('pill-speech'), has('speech'), 'needs model'); pill($('pill-ocr'), ocr, 'needs model');
}
async function showModels(note) {
  const m = await invoke('models_list');
  const kinds = { speech: 'Speech to text', ocr: 'Text recognition (blur secrets)', llm: 'Writing (docs)', runtime: 'LLM runtime' };
  modal('Local models', `<p>${note ? `<span class="found">${esc(note)}</span><br>` : ''}Models run on this computer and are stored in <code>${esc(m.dir)}</code>. Nothing is uploaded. For docs, Ollama or any local OpenAI compatible server is used automatically when it is running.</p>` +
    m.models.map(x => `<div class="model"><div><b>${esc(x.title)}</b><small data-model="${x.id}">${kinds[x.kind] || x.kind} &middot; ${x.size_human} &middot; ${esc(x.license)}</small></div>
      <span class="ok-text">${x.installed ? icon('i-check') + ' Installed' : ''}</span>
      <button class="btn ${x.installed ? 'ghost' : ''}" data-id="${x.id}" data-act="${x.installed ? 'delete' : 'download'}">${x.installed ? 'Delete' : 'Download ' + x.size_human}</button>
      <div class="bar" data-model="${x.id}" hidden><div></div></div></div>`).join(''));
  document.querySelectorAll('#modal-body button[data-id]').forEach(b => (b.onclick = async () => {
    b.disabled = true; b.textContent = b.dataset.act === 'delete' ? 'Deleting...' : 'Downloading...';
    try { await invoke(b.dataset.act === 'delete' ? 'model_delete' : 'model_download', { id: b.dataset.id }); } catch (e) { fail(e); }
    showModels(); refreshModels();
  }));
}
$('ai-models').onclick = () => showModels();

// ---------- Theme ----------
function setTheme(t) {
  document.documentElement.dataset.theme = t; localStorage.setItem('theme', t);
  $('theme-btn').innerHTML = icon(t === 'dark' ? 'i-sun' : 'i-moon');
  $('theme-btn').title = t === 'dark' ? 'Switch to light theme' : 'Switch to dark theme';
  tl.draw();
}
$('theme-btn').onclick = () => setTheme(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark');
setTheme(localStorage.getItem('theme') || (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'));

// ---------- Keyboard shortcuts ----------
const SHORTCUTS = [
  ['Playback', [['Space / K', 'Play or pause'], ['Left / Right', 'Step 0.1 s (Shift: 1 s)'], ['Home / End', 'Go to start or end'], ['J / L', 'Jump 5 s back or forward'], ['M', 'Mute or unmute the preview sound']]],
  ['Editing', [['S', 'Split clip at playhead'], ['Z', 'Add zoom at playhead'], ['Delete', 'Delete selected zoom or clip'], ['Ctrl Z', 'Undo'], ['Ctrl Shift Z', 'Redo'], ['D', 'Run auto edit']]],
  ['Timeline', [['+ / -', 'Zoom timeline in or out'], ['0', 'Fit timeline'], ['Ctrl Wheel', 'Zoom around the mouse'], ['Wheel', 'Pan when zoomed in'], ['Double-click', 'Add a zoom on the zoom lane'], ['Drag', 'Move a zoom, note or blur. Drag its edges to resize it']]],
  ['File', [['Ctrl O', 'Open'], ['Ctrl S', 'Save'], ['Ctrl Shift S', 'Save as'], ['Ctrl E', 'Export'], ['Ctrl ,', 'Settings'], ['Ctrl /', 'This list (also ?)']]],
];
const IS_MAC = /Mac/i.test(navigator.platform);
function showShortcuts() {
  const map = k => IS_MAC ? k.replace(/Ctrl/g, 'Cmd') : k;
  const rec = (shortcutState.actions || []).filter(a => a.text).map(a => `<div><span>${esc(a.label)}</span><span>${Shortcuts.parts(a.text).map(y => `<kbd>${esc(y)}</kbd>`).join('')}</span></div>`).join('');
  modal('Keyboard shortcuts', `<p class="muted small">${IS_MAC ? 'Cmd is the Command key.' : ''} Recording shortcuts also work while another app is in front. <button class="link" id="sc-change">Change them</button></p><div class="shortcuts">${rec ? '<h4>Recording (works from any app)</h4>' + rec : ''}${SHORTCUTS.map(([g, list]) => `<h4>${g}</h4>` + list.map(([k0, d]) => { const k = map(k0); return `<div><span>${d}</span><span>${k.split(' / ').map(x => x.split(' ').map(y => `<kbd>${esc(y)}</kbd>`).join('')).join(' / ')}</span></div>`; }).join('')).join('')}</div>`);
  $('sc-change').onclick = () => showSettings('shortcuts');
}
$('keys-btn').onclick = showShortcuts;

// ---------- ffmpeg check and About ----------
async function checkFfmpeg(force) {
  let st; try { st = await invoke('ffmpeg_status'); } catch { return; }
  if (st.found && !force) return st;
  if (!st.found) modal('ffmpeg is needed', `<p>DemoLens uses ffmpeg to record and export. The installer normally includes it, but no working copy was found on this computer.</p><p>Install it with this one line, then restart DemoLens:</p><pre class="code">${esc(st.hint)}</pre><p class="hint">Windows: run it in PowerShell. macOS: needs <a href="https://brew.sh" target="_blank">Homebrew</a>. Or download it from <a href="https://ffmpeg.org/download.html" target="_blank">ffmpeg.org</a>.</p>`);
  return st;
}
async function showAbout() {
  let st = {}; try { st = await invoke('ffmpeg_status'); } catch {}
  modal('About DemoLens', `<p><b>DemoLens ${esc(st.app_version || '')}</b>. Free and open source under the MIT license. <a href="https://github.com/im45145v/DemoLens" target="_blank">github.com/im45145v/DemoLens</a></p>
  <p><b>ffmpeg:</b> ${st.found ? esc(st.version) + (st.bundled ? ' (bundled)' : '') + '<br><span class="hint">' + esc(st.path) + '</span>' : 'not found'}</p>
  <p class="hint">This app bundles FFmpeg (https://ffmpeg.org), licensed under the GNU GPL version 2 or later (the bundled builds include GPL components). FFmpeg is a trademark of Fabrice Bellard. Source code for the bundled builds is available from the builders: BtbN/FFmpeg-Builds (Linux, Windows) and ffmpeg.martin-riedl.de (macOS). See THIRD_PARTY_NOTICES.md in the repository.</p>
  <p class="hint">Models are downloaded separately and keep their own licenses.</p>`);
}
$('about-btn').onclick = () => showSettings('about');
setTimeout(() => checkFfmpeg(false), 400);

// Every command goes through act(), so the native menu, the keyboard and the
// buttons all do the same thing. The menu and the keydown can both fire for
// one key press on macOS, so repeats within 200 ms are ignored.
let lastAct = { name: '', t: 0 };
const isTyping = el => (el.tagName === 'INPUT' && !['range', 'checkbox', 'color', 'radio', 'button'].includes(el.type)) || el.tagName === 'SELECT' || el.tagName === 'TEXTAREA' || el.isContentEditable;
function act(name) {
  const now = performance.now();
  if (lastAct.name === name && now - lastAct.t < 200) return;
  lastAct = { name, t: now };
  const typing = isTyping(document.activeElement);
  switch (name) {
    case 'undo': if (typing) document.execCommand('undo'); else undo(); break;
    case 'redo': if (typing) document.execCommand('redo'); else redo(); break;
    case 'save': save(false); break;
    case 'save-as': save(true); break;
    case 'open': openAny(); break;
    case 'open-folder': openProjectFolder(); break;
    case 'duplicate': duplicateProject(); break;
    case 'import-clip': importClip(); break;
    case 'record-here': recordHere(); break;
    case 'export': doExport(); break;
    case 'settings': showSettings(); break;
    case 'shortcuts': showShortcuts(); break;
    case 'ai-status': selectTab('magic'); $('ai-status').scrollIntoView({ block: 'center' }); break;
    case 'record': $('rec-btn').click(); break;
  }
}
window.__TAURI__.event.listen('menu', e => act(e.payload));
window.__TAURI__.event.listen('global-record', () => $('rec-btn').click());
window.__TAURI__.event.listen('rec-stop-request', () => { if (recording) $('rec-btn').click(); });
// ---------- Configurable global shortcuts ----------
let shortcutState = { mac: IS_MAC, actions: [] };
const scText = id => (shortcutState.actions.find(a => a.id === id) || {}).text || '';
const withKeys = (label, id) => { const k = scText(id); return k ? `${label} (${k})` : label; };
async function loadShortcuts() {
  try { shortcutState = await invoke('get_shortcuts'); } catch { return; }
  applyShortcutHints();
}
function applyShortcutHints() {
  const rb = $('rec-btn');
  rb.title = withKeys(recording ? 'Stop recording' : 'Start recording', 'record_toggle');
  $('rec-pause').title = withKeys('Pause input capture and cut this part', 'pause');
  $('rec-mic').title = withKeys('Record microphone', 'toggle_mic');
  $('rec-cam').title = withKeys('Record the webcam for a picture in picture overlay', 'toggle_webcam');
  $('rec-keys').title = withKeys('Capture key presses (off by default). Only shortcut names are kept, never what you type.', 'toggle_keys');
  $('rec-source').title = [withKeys('Record an area', 'record_area'), withKeys('a window', 'record_window'), withKeys('the screen', 'record_screen')].join(', ') + '. Click to pick a source.';
}
// A shortcut fired from any app (or the menu or tray). The editor owns the
// recording choices, so it does the work.
async function runShortcut(id) {
  const toggle = (btn, what) => { const b = $(btn); b.click(); status(`${what} ${b.getAttribute('aria-pressed') === 'true' ? 'on' : 'off'}${recording ? ' for the next recording' : ''}.`, 'info'); };
  const startPicked = async tab => {
    if (recording) return status('Already recording. Stop first.', 'info');
    $('modal').close();
    await openPicker(tab, { record: true });
  };
  try {
    switch (id) {
      case 'record_area': return await startPicked('area');
      case 'record_window': return await startPicked('windows');
      case 'record_screen':
        if (recording) return status('Already recording. Stop first.', 'info');
        setRecSource({ type: 'screen', display: 0 }, 'Entire screen', 'screen'); return $('rec-btn').click();
      case 'restart':
        if (!recording) return status('Nothing to restart. Start a recording first.', 'info');
        await invoke('discard_recording'); setRecUi(false); $('rec-btn').disabled = false;
        return setTimeout(() => $('rec-btn').click(), 300);
      case 'discard':
        if (!recording) return;
        return void (await invoke('discard_recording'));
      case 'toggle_mic': return toggle('rec-mic', 'Microphone');
      case 'toggle_webcam': return toggle('rec-cam', 'Webcam');
      case 'toggle_keys': return toggle('rec-keys', 'Keystroke capture');
    }
  } catch (e) { fail(e); }
}
window.__TAURI__.event.listen('shortcut', e => runShortcut(e.payload));
window.__TAURI__.event.listen('rec-marker', () => { if (recording) status('Marker dropped. The editor will zoom to the cursor here.', 'success'); });
loadShortcuts();

// ---------- Glass look for the app ----------
const reduceMq = window.matchMedia ? matchMedia('(prefers-reduced-transparency: reduce)') : { matches: false };
let glassPref = false, osReduce = false, glassNative = false;
async function applyGlassUi() {
  const reduce = !!(reduceMq.matches || osReduce);
  const on = glassPref && !reduce;
  document.documentElement.classList.toggle('glass-ui', on);
  if (on !== glassNative) { glassNative = on ? !!(await invoke('glass_native', { on: true }).catch(() => false)) : (await invoke('glass_native', { on: false }).catch(() => false), false); }
  document.documentElement.classList.toggle('glass-native', glassNative && on);
  return { on, reduce, native: glassNative };
}
async function loadGlassUi() {
  try { glassPref = !!(await invoke('get_settings')).glass_ui; osReduce = !!(await invoke('os_reduce_transparency')); } catch { }
  await applyGlassUi();
}
if (reduceMq.addEventListener) reduceMq.addEventListener('change', applyGlassUi);
// The system setting can change while the app is open; look again when the window is used.
window.addEventListener('focus', async () => { try { const v = !!(await invoke('os_reduce_transparency')); if (v !== osReduce) { osReduce = v; applyGlassUi(); } } catch { } });
loadGlassUi();

window.__TAURI__.event.listen('rec-discarded', () => { setRecUi(false); hideBusy(); status('Recording discarded. Nothing was saved.', 'info'); });

window.addEventListener('keydown', e => {
  const el = document.activeElement, typing = isTyping(el);
  const mod = e.ctrlKey || e.metaKey;
  // e.code is the physical key, so shortcuts work on any keyboard layout.
  const k = (e.code.startsWith('Key') ? e.code.slice(3) : e.key).toLowerCase();
  if ($('modal').open && !mod) return;
  if (mod && !e.altKey) {
    const m = { z: e.shiftKey ? 'redo' : 'undo', y: 'redo', s: e.shiftKey ? 'save-as' : 'save', o: 'open', e: 'export', r: e.shiftKey ? 'record' : 'record', ',': 'settings', '/': 'shortcuts' }[e.code === 'Comma' ? ',' : e.code === 'Slash' ? '/' : k];
    if (m) {
      if ((m === 'undo' || m === 'redo') && typing) return; // let the text field handle its own undo
      e.preventDefault(); act(m);
    }
    return;
  }
  if (typing || e.altKey) return;
  if (e.key === '?') { e.preventDefault(); return showShortcuts(); }
  if (!project) return;
  const step = e.shiftKey ? 1000 : 100;
  const onButton = el.tagName === 'BUTTON' || (el.tagName === 'INPUT' && ['checkbox', 'radio'].includes(el.type));
  if ((e.code === 'Space') && !onButton) { e.preventDefault(); setPlaying(!playing); }
  else if (e.key === 'ArrowLeft' && !el.closest('.tabs') && el.type !== 'range') { e.preventDefault(); seek(tl.playhead - step); }
  else if (e.key === 'ArrowRight' && !el.closest('.tabs') && el.type !== 'range') { e.preventDefault(); seek(tl.playhead + step); }
  else if (e.key === 'Home') { e.preventDefault(); seek(0); }
  else if (e.key === 'End') { e.preventDefault(); seek(tl.outDur()); }
  else if (k === 'j') seek(tl.playhead - 5000);
  else if (k === 'k') setPlaying(!playing);
  else if (k === 'm') $('vol-btn').click();
  else if (k === 'l') seek(tl.playhead + 5000);
  else if (k === 's') split();
  else if (k === 'z') addZoom();
  else if (k === 'd' && !busy) $('ready-btn').click();
  else if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); deleteSelected(); }
  else if (e.key === '+' || e.key === '=') tl.zoomBy(1.6);
  else if (e.key === '-') tl.zoomBy(1 / 1.6);
  else if (e.key === '0') tl.fit();
});


// ---------- Microphone: chooser and level meter ----------
const IS_WIN = navigator.userAgent.includes('Windows');
let meterOn = false, quietSince = 0;
async function loadMics() {
  let list = []; try { list = await invoke('mic_list'); } catch { }
  const sel = $('rec-mic-dev'), want = localStorage.getItem('mic-dev') || '';
  sel.innerHTML = '<option value="">Default microphone</option>' + list.map(n => `<option value="${esc(n)}">${esc(n)}</option>`).join('');
  sel.value = list.includes(want) ? want : '';
  sel.hidden = !list.length;
}
{ const rr = $('rec-res'); rr.value = localStorage.getItem('rec-res') || ''; rr.onchange = () => localStorage.setItem('rec-res', rr.value); }
$('rec-mic-dev').onchange = () => { localStorage.setItem('mic-dev', $('rec-mic-dev').value); if (!recording) meterStart(); };
async function meterStart() {
  if ($('rec-mic').getAttribute('aria-pressed') !== 'true') return meterStop();
  try { await invoke('mic_meter_start', { device: $('rec-mic-dev').value || null }); meterOn = true; $('mic-meter').hidden = false; localStorage.setItem('mic-ok', '1'); }
  catch (e) { meterOn = false; $('mic-meter').hidden = true; status('Cannot read the microphone: ' + humanError(e) + ' On a Mac, allow DemoLens under System Settings > Privacy & Security > Microphone.', 'error'); }
}
// Nothing may hold the microphone when the recorder opens it (Windows lets one
// app lock it), so wait until the meter has really let go.
async function meterStopAndWait() { meterOn = false; $('mic-meter').hidden = true; try { await invoke('mic_meter_stop'); } catch { } }
// A recording problem stays on screen until dismissed.
function showRecWarn(list) {
  const msgs = (list || []).filter(x => x && x !== 'recording without audio');
  $('rec-warn').hidden = !msgs.length;
  $('rec-warn-text').textContent = msgs.join(' ');
}
$('rec-warn-x').onclick = () => showRecWarn([]);
// About two seconds in, check that the microphone really delivers sound.
let micWatch = 0;
function checkMicSoon() {
  if ($('rec-mic').getAttribute('aria-pressed') !== 'true') return;
  const id = ++micWatch;
  setTimeout(async () => {
    if (id !== micWatch || !recording) return;
    try { const r = await invoke('rec_mic_status'); if (r.message) showRecWarn([r.message]); } catch { }
  }, 2000);
}
window.__TAURI__.event.listen('rec-mic', e => { if (e.payload?.message) showRecWarn([e.payload.message]); });
window.__TAURI__.event.listen('rec-warnings', e => { const w = (e.payload || []).filter(x => x !== 'recording without audio'); if (w.length) { showRecWarn(w); status(w.join(' '), 'error'); } });
function meterStop() { meterOn = false; $('mic-meter').hidden = true; invoke('mic_meter_stop').catch(() => { }); }
window.__TAURI__.event.listen('mic-level', e => {
  const { rms, peak } = e.payload, bars = [...$('mic-meter').children];
  const lvl = Math.max(0, Math.min(1, (peak + 60) / 60));
  bars.forEach((b, i) => { const on = lvl > i / bars.length + 0.02; b.classList.toggle('on', on); b.style.height = (4 + (on ? (i + 1) * 2.2 : 0)) + 'px'; });
  if (rms < -55) quietSince ||= performance.now(); else quietSince = 0;
  const quiet = quietSince && performance.now() - quietSince > 5000;
  $('mic-meter').classList.toggle('quiet', !!quiet);
  $('mic-meter').title = quiet ? 'Very quiet. Raise the input volume in System Settings > Sound > Input, or pick another microphone.' : 'Microphone level. Speak to see it move.';
});
document.querySelectorAll('#rec-mic').forEach(b => b.addEventListener('click', () => setTimeout(() => (b.getAttribute('aria-pressed') === 'true' ? meterStart() : meterStop()), 0)));
loadMics().then(() => { if (localStorage.getItem('mic-ok')) meterStart(); });

// ---------- Editing proxy and sound ----------
// A 720p copy of the video is built in the background. The editor reads that
// instead of the 2940x1912 original (the backend picks it up on its own), and
// the sound is played by a normal audio element on the proxy, in step with the
// timeline. The proxy's loudness is already evened out.
const pa = new Audio(); pa.preload = 'auto';
// The native player (playback.rs): the sound comes out of the app itself,
// not out of the web view. `on` is true once it is loaded for this recording.
const nat = { on: false, device: '', reason: '', clip: -1, checkedAt: 0, playing: false, loadedFor: null, last: null };
let proxyFor = null, audioBlob = null, audioState = 'none', audioText = '', audioIsWav = false, audioWatch = 0;
// The video proxy only speeds up the picture. The sound has its own small
// file (built in seconds, loudness already evened out) that is played from a
// blob, so it works the same in every web view and never waits for the proxy.
async function prepareProxy() {
  const src = project?.source; if (!src) return;
  prepareAudio();
  const have = await invoke('proxy_path', { source: src }).catch(() => null);
  if (have) return (proxyFor = src);
  $('proxy-note')?.remove();
  invoke('proxy_ensure', { source: src }).then(() => { proxyFor = src; }).catch(() => { });
}
const volPref = () => ({ muted: localStorage.getItem('preview-muted') === '1', volume: Math.max(0, Math.min(1, +(localStorage.getItem('preview-volume') ?? 1))) });
function applyVolume() {
  const v = volPref(), exp = Math.min(1, Math.max(0, project?.audio?.volume ?? 1));
  pa.muted = v.muted; pa.volume = Math.max(0, Math.min(1, v.volume * exp));
  // The native player gets the same level; the audio element is silent while it is in use.
  if (nat.on) { pa.muted = true; invoke('audio_set_volume', { volume: v.volume * exp, muted: v.muted }).catch(() => { }); }
  $('vol').value = v.muted ? 0 : v.volume;
  const b = $('vol-btn'); b.setAttribute('aria-pressed', String(v.muted || v.volume === 0)); b.setAttribute('aria-label', v.muted ? 'Unmute preview sound' : 'Mute preview sound');
}
function setAudioState(state, text) {
  audioState = state; audioText = text; tl.audioState = state;
  const c = $('audio-chip'); c.dataset.state = state; c.textContent = text;
  const muted = project?.audio?.mute;
  c.classList.toggle('warn', !!muted && state === 'ready');
  if (muted && state === 'ready') { c.textContent = 'Sound on, but the export is muted'; c.title = 'Mute is switched on under Style > Audio, so the exported video will have no sound.'; }
  else c.title = 'The sound you hear while editing. It is the recording\'s own microphone track.';
  tl.draw();
}
function loadAudio(url) {
  return new Promise((res, rej) => {
    const clean = () => { pa.removeEventListener('canplay', ok); pa.removeEventListener('error', bad); clearTimeout(t); };
    const ok = () => { clean(); res(); };
    const bad = () => { clean(); rej(new Error(pa.error?.message || 'the sound file could not be decoded')); };
    const t = setTimeout(() => { clean(); rej(new Error('the sound file took too long to load')); }, 8000);
    pa.addEventListener('canplay', ok); pa.addEventListener('error', bad);
    pa.src = url; pa.load();
  });
}
// The WAV copy of the sound: bigger, but plain PCM that every web view plays.
async function loadWav(src) {
  const path = await invoke('audio_preview_wav', { source: src });
  const blob = URL.createObjectURL(new Blob([await invoke('audio_bytes', { path })], { type: 'audio/wav' }));
  try { await loadAudio(blob); } catch (e) { URL.revokeObjectURL(blob); throw e; }
  if (audioBlob) URL.revokeObjectURL(audioBlob);
  audioBlob = blob; audioIsWav = true;
}
// A web view can say it loaded the AAC sound and then play nothing. A second
// after playing starts the clock must be moving and bytes must be decoded; if
// not, switch to the WAV copy and carry on from the same place.
function watchAudio() {
  clearTimeout(audioWatch);
  if (audioIsWav || audioState !== 'ready') return;
  const src = project?.source, t0 = pa.currentTime;
  audioWatch = setTimeout(async () => {
    if (!playing || audioIsWav || project?.source !== src || pa.paused) return;
    const stuck = pa.currentTime - t0 < 0.2 * (pa.playbackRate || 1);
    const silent = typeof pa.webkitAudioDecodedByteCount === 'number' && pa.webkitAudioDecodedByteCount === 0;
    if (!stuck && !silent) return;
    setAudioState('building', 'Switching the sound to a compatible format...');
    try { await loadWav(src); } catch (e) { return setAudioState('error', 'Sound unavailable'); }
    if (project?.source !== src) return;
    applyVolume(); setAudioState('ready', 'Sound on (compatible format)');
    if (playing) syncAudio(true);
  }, 1300);
}
// Browsers only let a media element start after a click or key press. The
// first one anywhere in the editor "unlocks" the element with a muted blip,
// so pressing Play later (even after a slow frame request) is never refused.
let audioPrimed = false;
function primeAudio() {
  if (audioPrimed || audioState !== 'ready' || pa.readyState < 2 || !pa.paused) return;
  audioPrimed = true;
  const keep = pa.currentTime;
  pa.muted = true;
  const p = pa.play();
  // If Play was pressed in the same instant, the sound is already meant to run.
  const done = () => { if (playing) { applyVolume(); watchAudio(); return; } pa.pause(); try { pa.currentTime = keep; } catch { } applyVolume(); };
  if (p && p.then) p.then(done, () => { audioPrimed = false; applyVolume(); }); else done();
}
['pointerdown', 'keydown', 'touchend'].forEach(ev => document.addEventListener(ev, primeAudio, true));
async function prepareAudio() {
  const src = project?.source; if (!src) return;
  pa.pause(); pa.removeAttribute('src'); pa.load(); audioIsWav = false; clearTimeout(audioWatch);
  if (audioBlob) { URL.revokeObjectURL(audioBlob); audioBlob = null; }
  tl.peaks = null; setAudioState('building', 'Preparing sound...');
  nat.on = false; nat.playing = false; nat.loadedFor = null; nat.reason = '';
  // First choice: play from the app itself. If that cannot start (no output
  // device, Linux), the audio element below takes over.
  if (await nativeLoad(src)) return;
  let r;
  try { r = await invoke('audio_preview', { source: src }); }
  catch (e) { if (project?.source !== src) return; setAudioState('error', 'Sound unavailable'); return status('Could not prepare the sound for the preview: ' + humanError(e), 'error'); }
  if (project?.source !== src) return;
  if (!r.has_audio) { setAudioState('none', 'No sound in this recording'); return; }
  tl.peaks = { data: r.peaks, bucket: r.bucket_ms };
  diagLog('using the web view audio element (' + (nat.reason || 'native playback not available') + ')');
  try {
    try { audioBlob = URL.createObjectURL(new Blob([await invoke('audio_bytes', { path: r.path })], { type: 'audio/mp4' })); await loadAudio(audioBlob); }
    catch (first) {
      // Second try: let the web view fetch the file itself through the asset protocol.
      try { await loadAudio(window.__TAURI__.core.convertFileSrc(r.path)); }
      catch { // Third: plain WAV, which every web view can play (WKWebView on a Mac may not decode the AAC file).
        try { await loadWav(src); } catch { throw first; }
      }
    }
  } catch (e) {
    if (project?.source !== src) return;
    setAudioState('error', 'Sound unavailable');
    return status('The sound could not be loaded into the preview (' + humanError(e) + '). Editing and export still work.', 'error');
  }
  if (project?.source !== src) return;
  applyVolume();
  setAudioState('ready', r.faint ? 'Sound: very quiet recording' : 'Sound on');
  if (playing) syncAudio(true);
}
$('vol').oninput = () => { const v = +$('vol').value; localStorage.setItem('preview-volume', String(v)); localStorage.setItem('preview-muted', v === 0 ? '1' : '0'); applyVolume(); };
$('vol-btn').onclick = () => { const v = volPref(); const mute = !(v.muted || v.volume === 0); localStorage.setItem('preview-muted', mute ? '1' : '0'); if (!mute && v.volume === 0) localStorage.setItem('preview-volume', '1'); applyVolume(); status(mute ? 'Preview sound muted. The export is not affected.' : 'Preview sound on.', 'info'); };
applyVolume();
window.__TAURI__.event.listen('proxy-progress', e => {
  const p = e.payload; if (p.source !== project?.source) return;
  if (p.error) status('Could not build the faster preview copy. Editing still works. ' + p.error, 'info');
  else if (p.done) { proxyFor = p.source; if (!playing) refresh(); }
});
function audioClip(srcMs) { return project.clips.find(c => srcMs >= c.src_start_ms && srcMs < c.src_end_ms) || project.clips[project.clips.length - 1]; }
// ---------- Audio diagnostics (Settings > Audio diagnostics) ----------
const diagLines = [];
function diagLog(t) {
  diagLines.push(new Date().toLocaleTimeString() + '  ' + t); if (diagLines.length > 80) diagLines.shift();
  invoke('audio_diag_log', { text: t }).catch(() => { });
}
pa.addEventListener('error', () => diagLog('audio element error: code ' + (pa.error?.code ?? '?') + ' ' + (pa.error?.message || '')));
const READY = ['HAVE_NOTHING', 'HAVE_METADATA', 'HAVE_CURRENT_DATA', 'HAVE_FUTURE_DATA', 'HAVE_ENOUGH_DATA'], NETWORK = ['NETWORK_EMPTY', 'NETWORK_IDLE', 'NETWORK_LOADING', 'NETWORK_NO_SOURCE'];
async function audioDiagnostics() {
  const L = [];
  L.push('Playing through: ' + (nat.on ? `the app itself (native), device '${nat.device}'` : 'the web view audio element' + (nat.reason ? ` (native playback: ${nat.reason})` : '')));
  L.push('Sound state in the editor: ' + audioState + ' (' + audioText + ')');
  L.push('Audio element src: ' + (pa.currentSrc || pa.src || '(none)'));
  L.push(`readyState: ${pa.readyState} ${READY[pa.readyState] || ''}`);
  L.push(`networkState: ${pa.networkState} ${NETWORK[pa.networkState] || ''}`);
  L.push('error code: ' + (pa.error ? `${pa.error.code} ${pa.error.message || ''}` : 'none'));
  let ctx = 'not available';
  try { const C = window.AudioContext || window.webkitAudioContext; const c = new C(); ctx = c.state + ' at ' + c.sampleRate + ' Hz'; c.close(); } catch (e) { ctx = 'failed: ' + (e.message || e); }
  L.push('AudioContext state: ' + ctx);
  L.push(`element volume: ${pa.volume.toFixed(2)}, muted: ${pa.muted}, paused: ${pa.paused}, time: ${pa.currentTime.toFixed(2)} s, length: ${isFinite(pa.duration) ? pa.duration.toFixed(2) + ' s' : 'unknown'}`);
  L.push(`can play AAC (mp4): "${pa.canPlayType('audio/mp4; codecs="mp4a.40.2"')}", WAV: "${pa.canPlayType('audio/wav')}"`);
  const v = volPref();
  L.push(`editor volume: ${v.volume}, muted: ${v.muted}, project volume: ${project?.audio?.volume ?? 1}`);
  try {
    const st = await invoke('audio_state');
    L.push(`native player: ${st.playing ? 'playing' : 'stopped'}, voice at ${st.voice_s.toFixed(2)} s, music at ${st.music_s.toFixed(2)} s, speed ${st.rate}x, volume ${st.volume}${st.muted ? ' (muted)' : ''}, output peak ${st.peak.toFixed(3)}, voice track ${st.has_voice ? 'loaded' : 'none'}, music ${st.has_music ? 'loaded' : 'none'}${st.error ? ', device error: ' + st.error : ''}`);
  } catch (e) { L.push('native player: ' + humanError(e)); }
  L.push('', 'Recent events:', ...diagLines.slice(-12));
  return L.join('\n');
}
async function renderAudioDiag() {
  const box = $('ad-body'); if (!box) return;
  const text = await audioDiagnostics();
  box.textContent = text;
  diagLog('diagnostics readout:\n' + text.split('\n').slice(0, 12).join('\n'));
}
function wireAudioDiag() {
  $('ad-refresh').onclick = renderAudioDiag;
  $('ad-copy').onclick = () => navigator.clipboard?.writeText($('ad-body').textContent).then(() => status('Copied.', 'success'), () => status('Could not copy.', 'error'));
  $('ad-test').onclick = async () => {
    if (!nat.on) return status('Native playback is not in use for this recording, so there is nothing to test here. Press Play in the editor instead.', 'info');
    await invoke('audio_play', { voiceS: 0, musicS: 0, rate: 1 }).catch(fail);
    status('Playing 2 seconds of the recording from the start...', 'info');
    setTimeout(async () => { const st = await invoke('audio_pause').catch(() => null); status(st && st.peak > 0.001 ? `The app played sound (output peak ${st.peak.toFixed(2)}).` : 'The app sent silence to the output. Check the system volume and the output device.', st && st.peak > 0.001 ? 'success' : 'error'); renderAudioDiag(); }, 2000);
  };
  renderAudioDiag();
}

// ---------- Native sound ----------
const outTotalMs = () => tl.outDur() + (project?.intro?.duration_ms || 0) + (project?.outro?.duration_ms || 0);
// Seconds into the finished video, which the music follows (the title card plays first).
const musicSec = () => ((tl.playhead + (project?.intro?.duration_ms || 0)) / 1000);
async function nativeLoad(src) {
  let r;
  try { r = await invoke('audio_native_load', { source: src, music: project?.music || null, totalS: outTotalMs() / 1000 }); }
  catch (e) { nat.reason = humanError(e); diagLog('native playback failed to load: ' + nat.reason); return false; }
  if (project?.source !== src) return true;
  if (!r || !r.available) { nat.reason = r?.reason || 'not available'; diagLog('native playback not available: ' + nat.reason); return false; }
  nat.on = true; nat.device = r.device; nat.loadedFor = src; nat.clip = -1;
  pa.pause(); pa.muted = true;
  if (!r.has_audio && !r.has_music) { nat.on = false; setAudioState('none', 'No sound in this recording'); return true; }
  tl.peaks = r.has_audio ? { data: r.peaks, bucket: r.bucket_ms } : null;
  applyVolume();
  setAudioState('ready', r.faint ? 'Sound: very quiet recording' : 'Sound on');
  diagLog(`native playback ready on '${r.device}' (${r.rate} Hz, ${r.channels} ch)`);
  if (playing) syncAudio(true);
  return true;
}
// A change to the music (file, volume, ducking) reloads just that track.
let musicTimer = 0;
function nativeMusicChanged() {
  if (!nat.on || !project) return;
  clearTimeout(musicTimer);
  musicTimer = setTimeout(() => invoke('audio_native_music', { music: project.music || null, totalS: outTotalMs() / 1000 }).catch(err => diagLog('music reload failed: ' + humanError(err))), 400);
}
function syncNative(start) {
  const a = project.audio, srcMs = tl.outToSrc(tl.playhead), clip = audioClip(srcMs);
  const idx = project.clips.indexOf(clip), speed = Math.max(0.25, Math.min(4, clip?.speed || 1));
  const want = (srcMs - (a.offset_ms || 0)) / 1000;
  if (clip?.mute) { invoke('audio_set_volume', { volume: 0, muted: true }).catch(() => { }); } else if (nat.mutedClip) applyVolume();
  nat.mutedClip = !!clip?.mute;
  const sound = (clip?.speed || 1) <= 4;
  if (!sound) { if (nat.playing) { nat.playing = false; invoke('audio_pause').catch(() => { }); } return; }
  const now = performance.now();
  if (start || !nat.playing || idx !== nat.clip) {
    nat.clip = idx; nat.playing = true; nat.checkedAt = now;
    nat.starting = invoke('audio_play', { voiceS: Math.max(0, want), musicS: musicSec(), rate: speed }).catch(err => { nat.playing = false; diagLog('audio_play failed: ' + humanError(err)); });
    return;
  }
  if (now - nat.checkedAt < 250) return;
  nat.checkedAt = now;
  invoke('audio_state').then(st => {
    nat.last = st;
    if (!playing || !nat.playing) return;
    if (st.rate !== speed) invoke('audio_set_rate', { rate: speed }).catch(() => { });
    if (AvSync.nativeCorrection(want, st.voice_s) || AvSync.nativeCorrection(musicSec(), st.music_s)) invoke('audio_seek', { voiceS: Math.max(0, want), musicS: musicSec() }).catch(() => { });
    if (st.error) diagLog('output device error: ' + st.error);
  }).catch(() => { });
}
function syncAudio(start) {
  if (audioState !== 'ready' || !project) return;
  if (nat.on) return syncNative(start);
  const a = project.audio, srcMs = tl.outToSrc(tl.playhead), clip = audioClip(srcMs);
  const want = (srcMs - (a.offset_ms || 0)) / 1000;
  applyVolume();
  pa.playbackRate = Math.max(0.25, Math.min(4, clip?.speed || 1)); pa.preservesPitch = true;
  // Above 4x the browser cannot keep up with the sound, so it is skipped.
  if ((clip?.speed || 1) > 4) { pa.pause(); return; }
  const speed = Math.max(0.25, Math.min(4, clip?.speed || 1));
  const fix = start ? { seek: true, rate: speed } : AvSync.audioCorrection(want, pa.currentTime, speed);
  if (fix.seek) pa.currentTime = Math.max(0, want);
  pa.playbackRate = Math.max(0.0625, Math.min(4.25, fix.rate));
  if (pa.paused) pa.play().then(() => { audioPrimed = true; watchAudio(); }).catch(err => {
    if (err?.name === 'NotAllowedError') { audioPrimed = false; setAudioState('blocked', 'Sound blocked: press Play again'); status('The system blocked the sound until you press Play. Press Play once more.', 'info'); }
    else if (err?.name !== 'AbortError') { setAudioState('error', 'Sound error'); status('Could not play the sound: ' + (err?.message || err), 'error'); }
  });
}
// Resolves once the sound is playing (or there is none, or it takes too long).
function audioStarted() {
  if (nat.on) return Promise.race([nat.starting || Promise.resolve(), new Promise(r => setTimeout(r, 700))]);
  if (audioState !== 'ready' || !project || (audioClip(tl.outToSrc(tl.playhead))?.speed || 1) > 4) return Promise.resolve();
  return new Promise(res => {
    const done = () => { pa.removeEventListener('playing', done); clearTimeout(t); res(); };
    const t = setTimeout(done, 700);
    pa.addEventListener('playing', done);
  });
}
const _setPlaying = setPlaying;
setPlaying = v => { _setPlaying(v); if (v) syncAudio(true); else { pa.pause(); if (nat.on) { nat.playing = false; invoke('audio_pause').catch(() => { }); } } };
const _refresh = refresh;
refresh = async function () { if (playing) syncAudio(false); return _refresh(); };

// ---------- AI status ----------
let aiState = null;
const STATE_LABEL = { ready: ['Ready', 'ok'], needs_model: ['Needs model', 'missing'], cloud: ['Using cloud', 'cloud'], needs_cloud: ['Needs cloud provider', 'missing'], error: ['Error', 'err'] };
async function loadAiStatus() {
  try { aiState = await invoke('ai_status'); } catch { return; }
  renderAiStatus();
}
function renderAiStatus() {
  const s = aiState; if (!s) return;
  $('ai-status').innerHTML = s.features.map(f => {
    const [label, cls] = STATE_LABEL[f.state] || [f.state, ''];
    const dl = f.needs.length ? `<button class="btn sm" data-dl="${f.needs.map(n => n.id).join(',')}">${f.state === 'error' ? 'Fix' : 'Download'} ${esc(f.needs.reduce((a, n) => a + n.size, 0) > 1e6 ? (f.needs.reduce((a, n) => a + n.size, 0) / 1e6).toFixed(0) + ' MB' : '')}</button>` : '';
    const test = f.can_test && f.state !== 'error' ? `<button class="btn ghost sm" data-test="${f.id}" title="Run a quick check with the installed model">Test</button>` : '';
    return `<div class="feat"><div class="feat-h"><b>${esc(f.title)}</b><span class="pill ${cls}">${esc(f.provider && f.state === 'cloud' ? 'Using ' + f.provider : label)}</span></div>
      <small>${esc(f.detail)}</small><div class="row feat-a">${dl}${test}<small class="muted" data-res="${f.id}"></small></div></div>`;
  }).join('');
  $('ai-dl-all').hidden = !s.recommended_missing.length;
  const total = s.recommended_missing.reduce((a, m) => a + m.size, 0);
  $('ai-dl-all').querySelector('span').textContent = `Download recommended models (${(total / 1e6).toFixed(0)} MB)`;
  document.querySelectorAll('#ai-status [data-dl]').forEach(b => (b.onclick = async () => {
    b.disabled = true; b.textContent = 'Downloading...';
    try { for (const id of b.dataset.dl.split(',')) await invoke('model_download', { id }); status('Model downloaded. It is ready to use.', 'success'); } catch (e) { fail(e); }
    loadAiStatus(); refreshModels();
  }));
  document.querySelectorAll('#ai-status [data-test]').forEach(b => (b.onclick = async () => {
    const out = document.querySelector(`[data-res="${b.dataset.test}"]`); b.disabled = true; out.textContent = 'Testing...';
    try { const r = await invoke('ai_self_test', { id: b.dataset.test }); out.textContent = (r.ok ? 'Passed: ' : 'Failed: ') + r.message; out.className = r.ok ? 'ok-text' : 'err-text'; }
    catch (e) { out.textContent = 'Failed: ' + humanError(e); out.className = 'err-text'; }
    b.disabled = false;
  }));
}
$('ai-dl-all').onclick = async () => {
  $('ai-dl-all').disabled = true; $('ai-dl-wrap').hidden = false; $('ai-dl-bar').style.width = '0%';
  try { const n = await invoke('models_download_recommended'); status(`Downloaded ${n} model${n === 1 ? '' : 's'}. Captions and secret blurring are ready.`, 'success'); }
  catch (e) { fail(e); }
  $('ai-dl-all').disabled = false; setTimeout(() => ($('ai-dl-wrap').hidden = true), 1200); loadAiStatus(); refreshModels();
};
$('ai-cloud-link').onclick = () => showSettings('cloud');
window.__TAURI__.event.listen('ai-progress', e => {
  const p = e.payload;
  if (p.task === 'download:all') { $('ai-dl-bar').style.width = (p.progress * 100) + '%'; $('ai-dl-text').textContent = p.message; }
});

// ---------- Disk usage (Settings) ----------
const gb = n => n >= 1e9 ? (n / 1e9).toFixed(1) + ' GB' : n >= 1e6 ? (n / 1e6).toFixed(0) + ' MB' : Math.max(0, Math.round(n / 1e3)) + ' KB';
async function renderDisk() {
  const box = $('disk-body'); if (!box) return;
  let r; try { r = await invoke('storage_report'); } catch (e) { box.innerHTML = '<p class="err-text">' + esc(humanError(e)) + '</p>'; return; }
  const bad = r.recordings.filter(x => x.health !== 'ok');
  box.innerHTML = `<p>Recordings use <b>${gb(r.total_bytes)}</b> in <code>${esc(r.root)}</code>. Editing copies use ${gb(r.proxy_bytes)} (rebuilt when needed).</p>
    <div class="row">${bad.length ? `<button class="btn" id="dk-clean">Clean up ${bad.length} empty or broken (${gb(r.reclaimable_bytes)})...</button>` : '<span class="ok-text">' + icon('i-check') + ' No empty or broken recordings.</span>'}
    <button class="btn ghost" id="dk-proxy">Clear editing copies</button></div>
    <div class="list">${r.recordings.map(x => `<div class="item"><span>${esc(x.name)} <small class="muted">${gb(x.bytes)}${x.health !== 'ok' ? ` &middot; <span class="err-text">${x.health}: ${esc(x.reason)}</span>` : ''}</small></span><button class="btn ghost sm" data-rm="${esc(x.path)}">Delete</button></div>`).join('') || '<p class="muted small">No recordings.</p>'}</div>`;
  const del = async (paths, title, text, yes) => {
    if (!(await confirmBox(title, text, yes))) return;
    try { const d = await invoke('storage_delete', { paths }); status(`Deleted ${d.deleted} folder${d.deleted === 1 ? '' : 's'}, freed ${gb(d.bytes)}.` + (d.failed.length ? ' Some could not be deleted: ' + d.failed[0] : ''), d.failed.length ? 'error' : 'success'); } catch (e) { fail(e); }
    renderDisk();
  };
  if ($('dk-clean')) $('dk-clean').onclick = () => del(bad.map(x => x.path), 'Clean up recordings?', `<p>These folders cannot be opened and will be deleted for good:</p><ul>${bad.map(x => `<li>${esc(x.name)}: ${gb(x.bytes)}, ${esc(x.reason)}</li>`).join('')}</ul><p>Working recordings are never touched.</p>`, 'Delete ' + bad.length);
  $('dk-proxy').onclick = async () => { const n = await invoke('storage_clear_proxies').catch(() => 0); status('Freed ' + gb(n), 'success'); renderDisk(); };
  box.querySelectorAll('[data-rm]').forEach(b => (b.onclick = () => del([b.dataset.rm], 'Delete this recording?', `<p>${esc(baseName(b.dataset.rm))} will be deleted for good.</p>`, 'Delete')));
}

// ---------- Start ----------
enableControls(); refreshModels(); loadTemplates().catch(fail);
invoke('initial_project').then(r => { if (r) { setProject(r.project, r.path); status('Opened ' + baseName(r.path || r.project.source), 'success'); } }).catch(fail);

// ---------- Privacy, cloud and storage ----------
let cloudState = { providers: [], choice: {} }, storageReady = false;
const TASKS = { stt: 'Speech to text', llm: 'Writing docs', tts: 'Text to speech' };
const kb = n => n > 1e6 ? (n / 1e6).toFixed(1) + ' MB' : Math.max(1, Math.round(n / 1e3)) + ' KB';
async function loadCloud() {
  try { cloudState = await invoke('cloud_status'); } catch { }
  try { const s = await invoke('s3_get'); storageReady = !!(s.config.bucket && s.secret_set); } catch { }
  const on = Object.keys(cloudState.choice || {});
  $('ai-badge').textContent = on.length ? 'cloud: ' + on.join(', ') : 'offline';
  $('ai-badge').title = on.length ? 'Some tasks use a cloud provider you turned on in Settings. You will see what is sent first.' : 'Models run on this computer. Nothing is uploaded.';
}

// Resolves true/false; the modal's close button counts as no.
function confirmBox(title, html, yes = 'Continue', no = 'Cancel') {
  return new Promise(res => {
    modal(title, html + `<div class="row end"><button class="btn ghost" id="cb-no">${esc(no)}</button><button class="btn primary" id="cb-yes">${esc(yes)}</button></div>`);
    let done = false; const fin = v => { if (done) return; done = true; $('modal').close(); res(v); };
    $('cb-yes').onclick = () => fin(true); $('cb-no').onclick = () => fin(false);
    $('modal').addEventListener('close', () => fin(false), { once: true });
  });
}

// Shows exactly what would be sent. Returns true (send), false (no cloud
// provider for this task, run locally) or null (the user cancelled).
async function confirmCloud(task) {
  const pv = await invoke('cloud_preview', { task, project, text: null });
  if (!pv) return false;
  const ok = await confirmBox('Send to ' + pv.provider + '?', `<p class="warn-note">This leaves your device.</p>
    <dl class="kv"><dt>Provider</dt><dd>${esc(pv.provider)} (${esc(pv.model)})</dd><dt>Endpoint</dt><dd><code>${esc(pv.endpoint)}</code></dd>
    <dt>Data</dt><dd>${esc(pv.data)}</dd><dt>Size</dt><dd>about ${kb(pv.bytes)}</dd>
    <dt>Their terms</dt><dd><a href="${esc(pv.terms)}" target="_blank" rel="noreferrer">Terms</a> and <a href="${esc(pv.privacy)}" target="_blank" rel="noreferrer">privacy policy</a></dd>
    <dt>You turned this on</dt><dd>${esc(pv.consented_on || 'unknown')}</dd></dl>
    <p class="muted small">Switch back to local any time in Settings > Cloud &amp; API keys.</p>`, 'Send');
  return ok ? true : null;
}

function reviewWarning() {
  if (sessionStorage.getItem('reviewed')) return Promise.resolve(true);
  return confirmBox('Before you share', `<p><b>Auto blur may miss text. Watch the export before sharing.</b></p>
    <p class="muted">Blur boxes come from text recognition on sampled frames. Small, moving or low contrast text can slip through.</p>`, 'Export').then(v => { if (v) sessionStorage.setItem('reviewed', '1'); return v; });
}

async function cloudVoiceover(docs) {
  const text = [docs.summary, ...docs.steps.map(s => s.said || s.title || '')].filter(Boolean).join(' ');
  const pv = await invoke('cloud_preview', { task: 'tts', project, text });
  if (!pv) return;
  if (!(await confirmBox('Send to ' + pv.provider + '?', `<p class="warn-note">This leaves your device.</p><p>${esc(pv.data)} to <code>${esc(pv.endpoint)}</code>.</p>`, 'Send'))) return;
  const p = await dialog.save({ defaultPath: 'voiceover.mp3', filters: [{ name: 'MP3 audio', extensions: ['mp3'] }] });
  if (!p) return;
  status('Generating the voiceover...', 'busy');
  try { await invoke('cloud_tts', { text, path: p }); status('Saved ' + baseName(p), 'success'); } catch (e) { fail(e); }
}

$('ex-upload').onclick = () => uploadExport(lastExport);
async function uploadExport(file) {
  if (!file) return;
  if (!(await confirmBox('Upload this file?', `<p class="warn-note">This leaves your device.</p><p>${esc(baseName(file))} goes to your own bucket as a private object. You get a signed link that expires. The event log and project file are never uploaded.</p>`, 'Upload'))) return;
  status('Uploading...', 'busy');
  try {
    const u = await invoke('s3_upload', { path: file });
    await navigator.clipboard.writeText(u.url).catch(() => { });
    status(`Uploaded${u.encrypted ? ' (encrypted at rest)' : ''}. Link copied; it expires in ${Math.round(u.expires_in_s / 3600)} h.`, 'success');
  } catch (e) { fail(e); }
}

async function showSettings(tab = 'privacy') {
  await loadCloud();
  const st = await invoke('get_settings'), recs = await invoke('list_recordings').catch(() => []);
  const s3 = await invoke('s3_get').catch(() => ({ config: {}, secret_set: false }));
  const tabs = ['privacy', 'permissions', 'appearance', 'shortcuts', 'cloud', 'storage', 'disk', 'audio', 'about'].map(t => `<button class="btn sm ${t === tab ? 'primary' : 'ghost'}" data-st="${t}">${{ privacy: 'Privacy', appearance: 'Appearance', shortcuts: 'Shortcuts', cloud: 'Cloud &amp; API keys', storage: 'Upload (S3)', permissions: 'Permissions', audio: 'Audio diagnostics', disk: 'Disk usage', about: 'About and updates' }[t]}</button>`).join('');
  let body = '';
  if (tab === 'privacy') body = `<p>Recordings stay on this computer in private folders. Key capture is off unless you turn it on for a recording, and then only shortcut names are kept.</p>
    <label class="inline">Delete recordings older than <select id="set-ret" class="field sm">${[0, 7, 30, 90].map(d => `<option value="${d}" ${(st.retention_days || 0) === d ? 'selected' : ''}>${d ? d + ' days' : 'never'}</option>`).join('')}</select></label>
    <h3 class="section">Recordings (${recs.length})</h3><div class="list">${recs.map(r => `<div class="item"><span>${esc(baseName(r.path))} <small class="muted">${kb(r.bytes)}</small></span><button class="btn ghost sm" data-del="${esc(r.path)}">Delete</button></div>`).join('') || '<p class="muted small">None.</p>'}</div>
    <div class="row"><button class="btn" id="set-wipe">Delete all DemoLens data...</button></div>
    <p class="muted small">See PRIVACY.md for exactly what is stored and sent.</p>`;
  if (tab === 'cloud') body = `<p>Cloud providers are off. Turn one on per task only if you want to; <b>the audio or text for that task then leaves your device</b> and goes to the provider, under their terms. Keys are kept in your system keychain.</p>` +
    cloudState.providers.map(p => `<div class="model"><div><b>${esc(p.info.name)}</b><small><a href="${esc(p.info.terms)}" target="_blank" rel="noreferrer">Terms</a> &middot; <a href="${esc(p.info.privacy)}" target="_blank" rel="noreferrer">Privacy</a>${p.info.key_url ? ` &middot; <a href="${esc(p.info.key_url)}" target="_blank" rel="noreferrer">Get a key</a>` : ''}</small>
      <div class="row">${p.info.tasks.map(t => `<label><input type="checkbox" data-p="${p.id}" data-t="${t}" ${cloudState.choice[t] === p.id ? 'checked' : ''}/> ${TASKS[t]}${p.consents[t] ? ` <small class="muted">since ${esc(p.consents[t])}</small>` : ''}</label>`).join('')}</div></div>
      <div class="row">${p.key_set ? `<code>${esc(p.key_masked)}</code><button class="btn ghost sm" data-rm="${p.id}">Remove key</button>` : `<input type="password" class="field sm" data-key="${p.id}" placeholder="API key" autocomplete="off" /><button class="btn sm" data-save="${p.id}">Save</button>`}</div></div>`).join('') +
    `<label class="stack">OpenAI compatible endpoint (for "Custom")<input id="set-base" class="field" value="${esc(cloudState.custom_base || '')}" placeholder="https://example.com/v1" /></label><div class="row"><button class="btn sm" id="set-base-save">Save endpoint</button></div>`;
  if (tab === 'appearance') {
    const g = await applyGlassUi();
    body = `<label class="switch"><input type="checkbox" id="set-glass" ${st.glass_ui ? 'checked' : ''} /> Glass look</label>
      <p class="muted small">A frosted, translucent look for the toolbar, sidebar and dialogs. ${IS_MAC ? 'On a Mac the window itself is see-through and macOS blurs what is behind it.' : 'Here it is a blur drawn in the page, so it only shows over other content, such as behind dialogs.'}</p>
      ${g.reduce ? '<p class="warn-note">Reduce Transparency is on in your system settings, so the glass look is paused. Turn that off in the system accessibility settings to see it.</p>' : ''}
      ${IS_MAC || /Win/i.test(navigator.platform) ? `<label class="switch"><input type="checkbox" id="set-tray" ${st.menu_bar !== false ? 'checked' : ''} /> Show DemoLens in the ${IS_MAC ? 'menu bar' : 'system tray'} for quick recording</label>` : ''}
      <p class="muted small">The exported video has its own glass frame: open the Style tab and turn on Glass.</p>`;
  }
  if (tab === 'shortcuts') body = '<div id="sc-body"><p class="muted small">Loading...</p></div>';
  if (tab === 'audio') body = `<p class="muted small">What the editor's sound is doing right now. Copy this into a bug report. It is also written to a log file.</p><pre id="ad-body" class="diag">Reading...</pre><div class="row"><button class="btn sm" id="ad-refresh">Refresh</button><button class="btn sm ghost" id="ad-copy">Copy</button><button class="btn sm ghost" id="ad-test">Play 2 seconds</button></div>`;
  if (tab === 'permissions') body = '<div id="perm-body"></div>';
  if (tab === 'about') body = '<div id="about-body"><p class="muted small">Loading...</p></div>';
  if (tab === 'disk') body = '<div id="disk-body"><p class="muted small">Measuring...</p></div>';
  if (tab === 'storage') { const c = s3.config || {}; body = `<p>Upload exports to your own S3 or Cloudflare R2 bucket. Objects are private, encrypted at rest where supported, and shared with a signed link (24 hours by default, 7 days at most). Event logs are never uploaded.</p>
    ${['endpoint', 'region', 'bucket', 'prefix', 'access_key_id'].map(k => `<label class="stack">${k.replace(/_/g, ' ')}<input class="field" data-s3="${k}" value="${esc(c[k] || '')}" /></label>`).join('')}
    <label class="stack">Link lifetime (hours, max 168)<input class="field" id="s3-exp" type="number" min="1" max="168" value="${Math.round((c.expiry_s || 86400) / 3600)}" /></label>
    <label class="stack">Secret access key ${s3.secret_set ? '(saved in the keychain; leave blank to keep)' : ''}<input class="field" id="s3-secret" type="password" autocomplete="off" /></label>
    <div class="row"><button class="btn primary" id="s3-save">Save storage</button></div>`; }
  modal('Settings', `<div class="row">${tabs}</div>${body}`);
  document.querySelectorAll('[data-st]').forEach(b => (b.onclick = () => showSettings(b.dataset.st)));
  if (tab === 'disk') renderDisk();
  if (tab === 'about') renderAboutTab();
  if (tab === 'audio') wireAudioDiag();
  if (tab === 'permissions') Permissions.mount($('perm-body'), { embedded: true });
  if (tab === 'shortcuts') renderShortcutsPage();
  if ($('set-glass')) $('set-glass').onchange = async () => { glassPref = $('set-glass').checked; await invoke('set_glass_ui', { enabled: glassPref }).catch(fail); await applyGlassUi(); status(glassPref ? 'Glass look on.' : 'Glass look off.', 'success'); };
  if ($('set-tray')) $('set-tray').onchange = () => invoke('set_menu_bar', { enabled: $('set-tray').checked }).then(() => status('Saved.', 'success'), fail);
  const again = () => showSettings(tab);
  if ($('set-ret')) $('set-ret').onchange = async () => { const n = await invoke('set_retention', { days: +$('set-ret').value || null }); status(n ? `Deleted ${n} old recording(s).` : 'Saved.', 'success'); again(); };
  document.querySelectorAll('[data-del]').forEach(b => (b.onclick = async () => { await invoke('delete_recording', { path: b.dataset.del }).catch(fail); again(); }));
  if ($('set-wipe')) $('set-wipe').onclick = async () => {
    const ok = await confirmBox('Delete all DemoLens data?', '<p>This deletes every recording in Videos/DemoLens, the transcript cache and the built in sample. Templates and models are kept.</p><label><input type="checkbox" id="wipe-keys" /> Also remove saved API keys</label>', 'Delete everything');
    if (ok) { const n = await invoke('delete_all_data', { includeKeys: !!document.getElementById('wipe-keys')?.checked }).catch(fail); status(`Deleted ${n || 0} item(s).`, 'success'); }
  };
  document.querySelectorAll('[data-t]').forEach(c => (c.onchange = async () => {
    if (c.checked) {
      const name = cloudState.providers.find(p => p.id === c.dataset.p).info.name;
      const ok = await confirmBox(`Use ${name} for ${TASKS[c.dataset.t].toLowerCase()}?`, `<p class="warn-note">Data leaves your device.</p><p>When you run this task, the ${c.dataset.t === 'stt' ? 'audio track' : 'text'} is sent to ${esc(name)} and handled under their terms. You will see exactly what is sent each time. Today's date is recorded as your consent.</p>`, 'Turn on');
      if (!ok) return showSettings('cloud');
    }
    await invoke('cloud_consent', { provider: c.dataset.p, task: c.dataset.t, accepted: c.checked }).catch(fail);
    showSettings('cloud');
  }));
  document.querySelectorAll('[data-save]').forEach(b => (b.onclick = async () => { const i = document.querySelector(`[data-key="${b.dataset.save}"]`); await invoke('cloud_set_key', { provider: b.dataset.save, key: i.value }).catch(fail); i.value = ''; showSettings('cloud'); }));
  document.querySelectorAll('[data-rm]').forEach(b => (b.onclick = async () => { await invoke('cloud_remove_key', { provider: b.dataset.rm }).catch(fail); showSettings('cloud'); }));
  if ($('set-base-save')) $('set-base-save').onclick = () => invoke('cloud_configure', { customBase: $('set-base').value, models: null, voice: cloudState.voice || null }).then(() => status('Saved.', 'success'), fail);
  if ($('s3-save')) $('s3-save').onclick = async () => {
    const config = { expiry_s: Math.min(168, Math.max(1, +$('s3-exp').value || 24)) * 3600 };
    document.querySelectorAll('[data-s3]').forEach(i => (config[i.dataset.s3] = i.value.trim()));
    try { await invoke('s3_set', { config, secret: $('s3-secret').value || null }); status('Storage saved.', 'success'); showSettings('storage'); } catch (e) { fail(e); }
  };
}
// Settings > Shortcuts: one row per action. Click the keys, press the new ones.
async function renderShortcutsPage(note) {
  const host = $('sc-body'); if (!host) return;
  await loadShortcuts();
  const caps = t => t ? Shortcuts.parts(t).map(p => `<kbd>${esc(p)}</kbd>`).join('') : '<span class="muted small">Not set</span>';
  host.innerHTML = `<p class="muted small">These work from any app while DemoLens is running. Click the keys, then press the new combination. Esc cancels. A combination needs Ctrl${IS_MAC ? ', Cmd' : ''} or Alt, so it does not take over normal typing.</p>
    <div class="sc-list">${shortcutState.actions.map(a => `<div class="sc-row" data-id="${a.id}">
      <div class="sc-name"><b>${esc(a.label)}</b><small>${esc(a.hint)}</small>${a.error ? `<small class="sc-err">Could not be registered: another app probably has these keys. Choose different keys.</small>` : ''}<small class="sc-err" data-err="${a.id}"></small></div>
      <button class="sc-keys${a.error ? ' bad' : ''}" data-rec="${a.id}" title="Click, then press the new keys">${caps(a.text)}</button>
      <button class="btn ghost sm" data-clear="${a.id}" ${a.combo ? '' : 'disabled'} title="Remove the keys">Clear</button>
      <button class="btn ghost sm" data-reset="${a.id}" ${a.custom ? '' : 'disabled'} title="Back to ${esc(a.default_text || 'no keys')}">Reset</button>
    </div>`).join('')}</div>
    <div class="row end"><button class="btn" id="sc-reset-all">Reset all to defaults</button></div>`;
  if (note) status(note, 'success');
  const fail1 = (id, msg) => { const el = host.querySelector(`[data-err="${id}"]`); if (el) el.textContent = String(msg).replace(/^Error: /, ''); };
  const set = async (id, combo) => {
    try { shortcutState = await invoke('set_shortcut', { action: id, combo }); applyShortcutHints(); await renderShortcutsPage('Shortcut saved.'); }
    catch (e) { fail1(id, e); }
  };
  host.querySelectorAll('[data-rec]').forEach(b => (b.onclick = () => {
    const id = b.dataset.rec, old = b.innerHTML;
    b.classList.add('listening'); b.innerHTML = '<span class="muted small">Press the new keys...</span>'; fail1(id, '');
    // Release the OS shortcuts while listening, or the system would swallow the combo.
    invoke('shortcuts_pause', { paused: true }).catch(() => { });
    let done = false;
    const stop = () => {
      if (done) return; done = true;
      window.removeEventListener('keydown', onKey, true); b.removeEventListener('blur', onBlur); $('modal').removeEventListener('close', onBlur);
      b.classList.remove('listening'); invoke('shortcuts_pause', { paused: false }).catch(() => { });
    };
    const onBlur = () => { stop(); if (b.isConnected) b.innerHTML = old; };
    const onKey = ev => {
      ev.preventDefault(); ev.stopPropagation();
      if (ev.code === 'Escape') { onBlur(); return; }
      const combo = Shortcuts.comboFromEvent(ev, IS_MAC);
      if (!combo) return; // only a modifier so far
      stop(); b.innerHTML = old; set(id, combo);
    };
    window.addEventListener('keydown', onKey, true); b.addEventListener('blur', onBlur); $('modal').addEventListener('close', onBlur);
  }));
  host.querySelectorAll('[data-clear]').forEach(b => (b.onclick = () => set(b.dataset.clear, '')));
  host.querySelectorAll('[data-reset]').forEach(b => (b.onclick = () => { const a = shortcutState.actions.find(x => x.id === b.dataset.reset); set(a.id, a.default); }));
  $('sc-reset-all').onclick = async () => { try { shortcutState = await invoke('reset_shortcuts'); applyShortcutHints(); await renderShortcutsPage('All shortcuts are back to their defaults.'); } catch (e) { fail(e); } };
}
$('settings-btn').onclick = () => showSettings();

// ---------- Drawing rectangles on a frame (area picker, blur boxes) ----------
// Shows `url` (an image of w x h source pixels) and lets the user drag
// rectangles. Resolves with rects in source pixels.
function drawRects(title, url, w, h, multi, hint) {
  return new Promise(res => {
    modal(title, `<p class="muted small">${esc(hint)}</p><div class="draw" id="draw"><img src="${url}" alt="" draggable="false" /></div>
      <div class="row end"><button class="btn ghost" id="dr-clear">Clear</button><button class="btn primary" id="dr-ok">Done</button></div>`);
    const box = $('draw'), rects = []; let cur = null, start = null;
    const at = e => { const b = box.getBoundingClientRect(); return [Math.max(0, Math.min(1, (e.clientX - b.left) / b.width)), Math.max(0, Math.min(1, (e.clientY - b.top) / b.height))]; };
    const paint = () => { box.querySelectorAll('.rect').forEach(x => x.remove()); [...rects, cur].filter(Boolean).forEach(r => { const d = document.createElement('div'); d.className = 'rect'; Object.assign(d.style, { left: r[0] * 100 + '%', top: r[1] * 100 + '%', width: r[2] * 100 + '%', height: r[3] * 100 + '%' }); box.appendChild(d); }); };
    box.onpointerdown = e => { box.setPointerCapture(e.pointerId); start = at(e); if (!multi) rects.length = 0; };
    box.onpointermove = e => { if (!start) return; const p = at(e); cur = [Math.min(p[0], start[0]), Math.min(p[1], start[1]), Math.abs(p[0] - start[0]), Math.abs(p[1] - start[1])]; paint(); };
    box.onpointerup = () => { if (cur && cur[2] > 0.005 && cur[3] > 0.005) rects.push(cur); cur = null; start = null; paint(); };
    box.onpointercancel = () => { cur = null; start = null; paint(); };
    let done = false; const fin = v => { if (done) return; done = true; $('modal').close(); res(v.map(r => ({ x: Math.round(r[0] * w), y: Math.round(r[1] * h), w: Math.round(r[2] * w) & ~1, h: Math.round(r[3] * h) & ~1 }))); };
    $('dr-clear').onclick = () => { rects.length = 0; paint(); };
    $('dr-ok').onclick = () => fin(rects);
    $('modal').addEventListener('close', () => fin([]), { once: true });
  });
}
$('blur-draw').onclick = async () => {
  if (!project) return;
  const src = tl.outToSrc(tl.playhead);
  try {
    const f = await invoke('raw_frame', { project, srcMs: src });
    const rs = await drawRects('Draw blur boxes', f.url, f.w, f.h, true, 'Drag over anything to hide. Each box blurs from the playhead to the end; trim it later in the list.');
    rs.forEach(r => project.blur.push({ ...r, start_ms: src, end_ms: project.duration_ms, radius: 18 }));
    if (rs.length) { renderBlurList(); refresh(); checkpoint(); status(`Added ${rs.length} blur box(es).`, 'success'); }
  } catch (e) { fail(e); }
};
['card-len', 'in-title', 'in-sub', 'out-title', 'out-sub', 'zc-steady', 'wm-text', 'wm-corner', 'wm-opacity', 'cam-visible', 'cam-circle', 'cam-size', 'cam-corner', 'cam-layout', 'cam-mirror', 'lay-trans', 'lay-follow'].forEach(id => ($(id).oninput = $(id).onchange = () => { readPanel(); refresh(); }));

// ---------- First run ----------
// The permissions screen comes first (a Mac's first launch, or a permission that went off), then the privacy note.
function firstRunNote() {
  if (localStorage.getItem('privacy-ack')) return;
  modal('Before you start', `<p>DemoLens records and edits on this computer. By default nothing leaves it, except model downloads you start from the Models dialog.</p>
    <ul><li>A small pill shows while recording (it is not in the video). Its buttons pause, drop a marker, stop or discard; the keys are in Settings > Shortcuts.</li><li>Key capture is off unless you turn it on, and never stores what you type.</li><li>Cloud providers and uploads are off until you turn them on in Settings, and show what is sent first.</li></ul>
    <div class="row end"><button class="btn primary" id="fr-ok">Got it</button></div>`);
  $('fr-ok').onclick = () => { localStorage.setItem('privacy-ack', '1'); $('modal').close(); };
}
Permissions.maybeShow().catch(() => { }).then(firstRunNote);
loadCloud();
