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

const dialog = {
  open: o => (window.__TAURI__.dialog ? window.__TAURI__.dialog.open(o) : invoke('plugin:dialog|open', { options: o })),
  save: o => (window.__TAURI__.dialog ? window.__TAURI__.dialog.save(o) : invoke('plugin:dialog|save', { options: o })),
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
function restore() { const t = tl.playhead; project = JSON.parse(history[hIdx]); tl.project = project; tl.playhead = Math.min(t, tl.outDur()); syncPanel(); tl.draw(); refresh(); updUndo(); }
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
  onSelect: () => syncSelection(),
});

function setProject(p, path = null) {
  project = p; projectPath = path; tl.selectedZoom = -1; tl.selectedClip = 0; tl.playhead = 0; tl.setProject(p);
  history = [JSON.stringify(p)]; hIdx = 0; saved = path ? history[0] : null; updUndo();
  $('welcome').hidden = true; $('preview').hidden = false; $('tl-empty').hidden = true;
  $('ready-report').innerHTML = '';
  document.body.classList.add('has-project');
  enableControls(); syncPanel(); syncSelection(); refresh();
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
  $('padding').value = s.padding; $('radius').value = s.corner_radius; $('shadow').value = s.shadow;
  const c = project.cursor;
  $('cur-visible').checked = c.visible; $('cur-size').value = c.size; $('cur-smooth').value = c.smoothing_ms; $('cur-ripple').checked = c.click_ripples;
  $('keys-on').checked = project.keys.enabled; $('keys-short').checked = project.keys.shortcuts_only;
  $('cap-burn').checked = project.caption_style.burn_in; $('cap-size').value = project.caption_style.size;
  $('aud-denoise').checked = project.audio.denoise; $('aud-mute').checked = project.audio.mute;
  $('ex-aspect').value = project.export.aspect; $('ex-quality').value = project.export.quality; $('ex-fps').value = project.export.fps; $('ex-format').value = project.export.format;
  renderBlurList(); markTemplate(); markDestination();
}
function readPanel() {
  const t = $('bg-type').value, s = project.style;
  s.background = t === 'gradient' ? { type: t, from: rgb($('bg-from').value), to: rgb($('bg-to').value), angle_deg: +$('bg-angle').value }
    : t === 'solid' ? { type: t, color: rgb($('bg-from').value) } : { type: t, path: $('bg-image').value };
  s.padding = +$('padding').value; s.corner_radius = +$('radius').value; s.shadow = +$('shadow').value;
  Object.assign(project.cursor, { visible: $('cur-visible').checked, size: +$('cur-size').value, smoothing_ms: +$('cur-smooth').value, click_ripples: $('cur-ripple').checked });
  Object.assign(project.keys, { enabled: $('keys-on').checked, shortcuts_only: $('keys-short').checked });
  Object.assign(project.caption_style, { burn_in: $('cap-burn').checked, size: +$('cap-size').value });
  Object.assign(project.audio, { denoise: $('aud-denoise').checked, mute: $('aud-mute').checked });
  Object.assign(project.export, { aspect: $('ex-aspect').value, quality: +$('ex-quality').value, fps: +$('ex-fps').value, format: $('ex-format').value });
}
document.querySelectorAll('#pane-style input, #pane-export select').forEach(el => el.addEventListener('input', () => { if (project) { readPanel(); markTemplate(); markDestination(); refresh(); } }));

function setBgType(t) {
  $('bg-type').value = t;
  document.querySelectorAll('.seg button').forEach(b => b.setAttribute('aria-checked', b.dataset.bg === t));
  $('bg-to-wrap').hidden = $('bg-angle-wrap').hidden = t !== 'gradient';
  $('bg-colors').hidden = t === 'image'; $('bg-image-row').hidden = t !== 'image';
}
document.querySelectorAll('.seg button').forEach(b => (b.onclick = () => { setBgType(b.dataset.bg); if (project) { readPanel(); refresh(); } }));
$('bg-pick').onclick = async () => {
  const p = await dialog.open({ multiple: false, filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] });
  if (p) { $('bg-image').value = p; readPanel(); refresh(); }
};

function renderBlurList() {
  $('blur-list').innerHTML = project.blur.length ? '' : '<p class="muted small">No blur boxes yet. Blur secrets in the Magic tab finds them for you.</p>';
  project.blur.forEach((b, i) => {
    const d = document.createElement('div'); d.className = 'item';
    d.innerHTML = `<span>${b.x},${b.y} ${b.w}x${b.h} &middot; ${(b.start_ms / 1000).toFixed(1)}-${(b.end_ms / 1000).toFixed(1)}s</span>`;
    const x = document.createElement('button'); x.className = 'btn icon ghost sm'; x.innerHTML = icon('i-x'); x.setAttribute('aria-label', 'Remove blur box ' + (i + 1));
    x.onclick = () => { project.blur.splice(i, 1); renderBlurList(); refresh(); };
    d.appendChild(x); $('blur-list').appendChild(d);
  });
}

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
}

// ---------- Preview and playback ----------
async function refresh() {
  if (!project) return;
  if (!playing) scheduleCheckpoint();
  $('time').textContent = `${fmt(tl.playhead)} / ${fmt(tl.outDur())}`;
  if (pending) { again = true; return; }
  pending = true;
  try { $('preview').src = await invoke('preview', { project, outMs: tl.playhead, shortSide: 540 }); }
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
  if (tl.selectedZoom >= 0) { project.zoom.splice(tl.selectedZoom, 1); tl.selectedZoom = -1; status('Zoom deleted'); }
  else if (project.clips.length > 1) { project.clips.splice(tl.selectedClip, 1); tl.selectedClip = Math.max(0, tl.selectedClip - 1); tl.playhead = Math.min(tl.playhead, tl.outDur()); status('Clip deleted'); }
  else return status('A project needs at least one clip.');
  tl.draw(); syncSelection(); refresh();
}
function addZoom() {
  const at = tl.outToSrc(tl.playhead);
  project.zoom.push({ start_ms: at, end_ms: Math.min(project.duration_ms, at + 2000), scale: 2, follow_cursor: true, focus: [project.source_width / 2, project.source_height / 2], reason: 'manual' });
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
$('zoom-follow').onchange = () => { const z = project?.zoom[tl.selectedZoom]; if (z) { z.follow_cursor = $('zoom-follow').checked; refresh(); } };
$('auto-zoom').onclick = async () => {
  try { project.zoom = await invoke('regenerate_zoom', { project }); tl.selectedZoom = -1; tl.draw(); syncSelection(); refresh(); status(project.zoom.length ? `${project.zoom.length} zooms from clicks and typing` : 'No input log for this video, so there is nothing to zoom on.'); } catch (e) { fail(e); }
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
    if (path.endsWith('.json')) setProject(await invoke('load_project', { path }), path);
    else {
      const events = path.replace(/\.[^.\\/]+$/, '.events.jsonl');
      let p; try { p = await invoke('open_recording', { video: path, events }); } catch { p = await invoke('open_recording', { video: path, events: null }); }
      setProject(p, null);
    }
    status('Opened ' + baseName(path), 'success');
  } catch (e) { fail(e); }
  hideBusy();
}
async function save(as) {
  if (!project) return;
  try {
    if (as || !projectPath) {
      const p = await dialog.save({ defaultPath: projectPath || stem(project.source) + '.demolens.json', filters: [{ name: 'DemoLens project', extensions: ['json'] }] });
      if (!p) return;
      projectPath = p;
    }
    readPanel(); checkpoint();
    await invoke('save_project', { path: projectPath, project });
    saved = history[hIdx]; updTitle(); status('Saved ' + baseName(projectPath), 'success');
  } catch (e) { fail(e); }
}
async function trySample() {
  showBusy('Building the sample recording...');
  try { const r = await invoke('make_sample'); setProject(r.project, r.path); status('Sample loaded. Press Make it demo-ready to see the AI at work.', 'success'); selectTab('magic'); }
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
$('rec-target').onchange = () => { const k = $('rec-target').value; $('rec-arg').hidden = k === 'screen'; $('rec-arg').placeholder = k === 'window' ? (navigator.userAgent.includes('Windows') ? 'Window title' : 'Window id') : 'x,y,w,h'; };
document.querySelectorAll('.toggle').forEach(b => (b.onclick = () => b.setAttribute('aria-pressed', b.getAttribute('aria-pressed') !== 'true')));
function setRecUi(on) {
  recording = on;
  $('rec-btn').classList.toggle('recording', on);
  $('rec-btn').innerHTML = icon(on ? 'i-stop' : 'i-record') + `<span>${on ? 'Stop' : 'Record'}</span>`;
  $('rec-btn').title = on ? 'Stop recording (Ctrl+R)' : 'Start recording (Ctrl+R)';
}
$('rec-btn').onclick = async () => {
  try {
    if (!recording) {
      const kind = $('rec-target').value, arg = $('rec-arg').value.trim();
      let target = { type: 'screen', display: 0 };
      if (kind === 'window') { if (!arg) return status('Type the window id (Linux) or title (Windows) first.', 'error'); target = { type: 'window', id: arg }; }
      if (kind === 'area') { const v = arg.split(',').map(Number); if (v.length !== 4 || v.some(isNaN)) return status('Type the area as x,y,w,h, for example 0,0,1280,720', 'error'); const [x, y, w, h] = v; target = { type: 'area', x, y, w, h }; }
      const warn = await invoke('start_recording', { target, mic: $('rec-mic').getAttribute('aria-pressed') === 'true', systemAudio: $('rec-sys').getAttribute('aria-pressed') === 'true' });
      setRecUi(true);
      const w = warn.filter(x => x !== 'recording without audio');
      status(w.length ? 'Recording. ' + w.join(' ') : 'Recording. Press Stop or Ctrl+R when you are done.', w.length ? 'error' : 'busy');
    } else {
      setRecUi(false); showBusy('Finishing the recording...');
      setProject(await invoke('stop_recording'), null); status('Recording loaded. Try Make it demo-ready.', 'success');
      hideBusy();
    }
  } catch (e) { setRecUi(false); hideBusy(); fail(e); }
};

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
async function doExport() {
  if (!project || busy) return;
  selectTab('export'); readPanel();
  const gif = project.export.format === 'gif';
  const out = await dialog.save({ defaultPath: stem(projectPath || project.source) + '-demo.' + (gif ? 'gif' : 'mp4'), filters: [gif ? { name: 'GIF', extensions: ['gif'] } : { name: 'MP4 video', extensions: ['mp4'] }] });
  if (!out) return;
  busy = true; enableControls();
  $('ex-state').hidden = false; $('ex-reveal').hidden = true; $('ex-bar').style.width = '0%'; $('ex-text').textContent = 'Starting...';
  status('Exporting ' + baseName(out) + '...', 'busy');
  const t0 = performance.now();
  try {
    const r = await invoke('export_video', { project, out });
    lastExport = out; $('ex-bar').style.width = '100%';
    $('ex-text').textContent = `Done in ${((performance.now() - t0) / 1000).toFixed(0)} s: ${baseName(out)}`; $('ex-reveal').hidden = false;
    status('Exported ' + r, 'success');
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
  try { await fn(); } catch (e) {
    fail(e);
    if (/model|install|download/i.test(String(e))) showModels(humanError(e));
  }
  busy = false; $('ai-bar-wrap').hidden = true; enableControls(); refreshModels();
}
window.__TAURI__.event.listen('ai-progress', e => {
  const p = e.payload;
  if (p.task.startsWith('download:')) {
    const id = p.task.slice(9), bar = document.querySelector(`.bar[data-model="${id}"]`);
    if (bar) { bar.hidden = false; bar.firstElementChild.style.width = (p.progress * 100) + '%'; }
    const lbl = document.querySelector(`small[data-model="${id}"]`); if (lbl) lbl.textContent = 'Downloading ' + p.message;
  } else { $('ai-bar-wrap').classList.remove('indet'); $('ai-bar').style.width = (p.progress * 100) + '%'; status(p.message, 'busy'); }
});

$('ready-btn').onclick = () => aiTask('Making it demo-ready', async () => {
  const r = await invoke('ai_demo_ready', { project, zoom: $('r-zoom').checked, captions: $('r-captions').checked, cut: $('r-cut').checked, blur: $('r-blur').checked, denoise: $('r-denoise').checked });
  applyResult(r.project);
  const ic = { done: 'i-check', skipped: 'i-skip', failed: 'i-alert' };
  $('ready-report').innerHTML = r.report.steps.map(s => `<li class="${s.state}">${icon(ic[s.state])}<span><b>${esc(s.name)}</b><small>${esc(s.detail)}</small></span></li>`).join('') +
    `<li class="done">${icon('i-check')}<span><b>Length</b><small>${fmt(r.report.before_ms)} to ${fmt(r.report.after_ms)}. Ctrl+Z undoes everything at once.</small></span></li>`;
  const skipped = r.report.steps.filter(s => s.state !== 'done').length;
  status(skipped ? `Demo-ready, ${skipped} step(s) skipped. See the Magic tab for why.` : 'Demo-ready. Press Space to watch it, then Export.', 'success');
});
$('ai-transcribe').onclick = () => aiTask('Transcribing', async () => {
  const r = await invoke('ai_transcribe', { project, model: null });
  r.project.caption_style.burn_in = true; applyResult(r.project);
  status(`${r.report.words} words and ${r.report.fillers} filler words. Captions are burned in; switch that off in Style.`, 'success');
});
$('ai-autocut').onclick = () => aiTask('Finding pauses and filler words', async () => {
  const r = await invoke('ai_autocut', { project, fillers: $('cut-fillers').checked, silences: $('cut-silences').checked, minSilenceMs: +$('cut-min').value });
  applyResult(r.project);
  const n = r.report.cuts.length, f = r.report.cuts.filter(c => c[2] === 'filler').length;
  status(`${n} cuts (${f} filler words, ${n - f} pauses). ${fmt(r.report.before_ms)} to ${fmt(r.report.after_ms)}.` + (!r.report.had_transcript && $('cut-fillers').checked ? ' Transcribe first to also cut filler words.' : ''), 'success');
});
$('ai-redact').onclick = () => aiTask('Scanning for secrets', async () => {
  const r = await invoke('ai_redact', { project });
  applyResult(r.project);
  status(r.report.found.length ? `Blurred ${r.report.added} regions: ${r.report.found.map(f => f[0]).join(', ')}` : 'Nothing sensitive found.', 'success');
});
$('ai-docs').onclick = () => aiTask('Writing docs', async () => {
  const r = await invoke('ai_docs', { project, useLlm: $('docs-llm').checked });
  status('Docs written by the ' + r.docs.generated_by, 'success');
  modal('Demo docs', `<p>Written by ${esc(r.docs.generated_by)}.</p>
    <div class="row"><button class="btn" id="doc-save">${icon('i-save')}<span>Save guide.md...</span></button><button class="btn" id="doc-copy-ch">Copy chapters</button><button class="btn" id="doc-copy-post">Copy post</button></div>
    <pre class="doc">${esc(r.markdown)}</pre>`);
  $('doc-save').onclick = async () => { const p = await dialog.save({ defaultPath: 'guide.md', filters: [{ name: 'Markdown', extensions: ['md'] }] }); if (p) { await invoke('write_text', { path: p, text: r.markdown }); status('Saved ' + baseName(p), 'success'); } };
  $('doc-copy-ch').onclick = () => navigator.clipboard.writeText(r.chapters).then(() => status('Chapters copied', 'success'));
  $('doc-copy-post').onclick = () => navigator.clipboard.writeText(r.docs.social_post).then(() => status('Post copied', 'success'));
});
$('cap-export').onclick = async () => {
  if (!project) return;
  if (!project.captions.length) return status('Transcribe first to get subtitles.', 'error');
  const p = await dialog.save({ defaultPath: stem(project.source) + '.srt', filters: [{ name: 'Subtitles', extensions: ['srt', 'vtt'] }] });
  if (p) { try { await invoke('export_captions', { project, path: p }); status('Wrote ' + baseName(p), 'success'); } catch (e) { fail(e); } }
};

async function refreshModels() {
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
  ['Playback', [['Space', 'Play or pause'], ['Left / Right', 'Step 0.1 s (Shift: 1 s)'], ['Home / End', 'Go to start or end'], ['J / L', 'Jump 5 s back or forward']]],
  ['Editing', [['S', 'Split clip at playhead'], ['Z', 'Add zoom at playhead'], ['Delete', 'Delete selected zoom or clip'], ['Ctrl Z', 'Undo'], ['Ctrl Shift Z', 'Redo'], ['D', 'Make it demo-ready']]],
  ['Timeline', [['+ / -', 'Zoom timeline in or out'], ['0', 'Fit timeline'], ['Ctrl Wheel', 'Zoom around the mouse'], ['Wheel', 'Pan when zoomed in'], ['Double-click', 'Add a zoom on the zoom lane']]],
  ['File', [['Ctrl O', 'Open'], ['Ctrl S', 'Save'], ['Ctrl Shift S', 'Save as'], ['Ctrl E', 'Export'], ['Ctrl R', 'Start or stop recording'], ['?', 'This list']]],
];
function showShortcuts() {
  modal('Keyboard shortcuts', `<div class="shortcuts">${SHORTCUTS.map(([g, list]) => `<h4>${g}</h4>` + list.map(([k, d]) => `<div><span>${d}</span><span>${k.split(' / ').map(x => x.split(' ').map(y => `<kbd>${esc(y)}</kbd>`).join('')).join(' / ')}</span></div>`).join('')).join('')}</div>`);
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
  <p class="hint">Optional AI models are downloaded separately and keep their own licenses.</p>`);
}
$('about-btn').onclick = showAbout;
setTimeout(() => checkFfmpeg(false), 400);

window.addEventListener('keydown', e => {
  const el = document.activeElement, typing = (el.tagName === 'INPUT' && !['range', 'checkbox', 'color'].includes(el.type)) || el.tagName === 'SELECT' || el.tagName === 'TEXTAREA';
  const mod = e.ctrlKey || e.metaKey, k = e.key.toLowerCase();
  if ($('modal').open && !mod) return;
  if (mod) {
    if (k === 'z' && !e.shiftKey) { if (typing) return; e.preventDefault(); undo(); }
    else if ((k === 'z' && e.shiftKey) || k === 'y') { if (typing) return; e.preventDefault(); redo(); }
    else if (k === 's') { e.preventDefault(); save(e.shiftKey); }
    else if (k === 'o') { e.preventDefault(); openAny(); }
    else if (k === 'e') { e.preventDefault(); doExport(); }
    else if (k === 'r') { e.preventDefault(); $('rec-btn').click(); }
    return;
  }
  if (typing || e.altKey) return;
  if (e.key === '?') { e.preventDefault(); return showShortcuts(); }
  if (!project) return;
  const step = e.shiftKey ? 1000 : 100;
  if (e.key === ' ' && el.tagName !== 'BUTTON') { e.preventDefault(); setPlaying(!playing); }
  else if (e.key === 'ArrowLeft' && !el.closest('.tabs')) { e.preventDefault(); seek(tl.playhead - step); }
  else if (e.key === 'ArrowRight' && !el.closest('.tabs')) { e.preventDefault(); seek(tl.playhead + step); }
  else if (e.key === 'Home') { e.preventDefault(); seek(0); }
  else if (e.key === 'End') { e.preventDefault(); seek(tl.outDur()); }
  else if (k === 'j') seek(tl.playhead - 5000);
  else if (k === 'l') seek(tl.playhead + 5000);
  else if (k === 's') split();
  else if (k === 'z') addZoom();
  else if (k === 'd' && !busy) $('ready-btn').click();
  else if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); deleteSelected(); }
  else if (e.key === '+' || e.key === '=') tl.zoomBy(1.6);
  else if (e.key === '-') tl.zoomBy(1 / 1.6);
  else if (e.key === '0') tl.fit();
});

// ---------- Start ----------
enableControls(); refreshModels(); loadTemplates().catch(fail);
invoke('initial_project').then(r => { if (r) { setProject(r.project, r.path); status('Opened ' + baseName(r.path || r.project.source), 'success'); } }).catch(fail);
