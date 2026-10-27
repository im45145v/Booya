// DemoLens editor UI. All pixels come from the Rust renderer via `preview`.
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = id => document.getElementById(id);
let project = null, playing = false, pending = false, dirty = false, recording = false, projectPath = null, busy = false;

// Native file dialogs (tauri-plugin-dialog).
const dialog = {
  open: o => (window.__TAURI__.dialog ? window.__TAURI__.dialog.open(o) : invoke('plugin:dialog|open', { options: o })),
  save: o => (window.__TAURI__.dialog ? window.__TAURI__.dialog.save(o) : invoke('plugin:dialog|save', { options: o })),
};
const VIDEO_EXT = ['mkv', 'mp4', 'mov', 'webm', 'avi'];
const baseName = p => String(p).split(/[\\/]/).pop();

// Undo / redo: snapshots of the whole project, taken shortly after edits settle.
let history = [], hIdx = -1, cpTimer = null;
function updUndo() { $('undo-btn').disabled = hIdx <= 0; $('redo-btn').disabled = hIdx >= history.length - 1; }
function checkpoint() {
  if (!project) return;
  const s = JSON.stringify(project);
  if (history[hIdx] === s) return;
  history = history.slice(0, hIdx + 1); history.push(s);
  if (history.length > 200) history.shift();
  hIdx = history.length - 1; updUndo();
}
function scheduleCheckpoint() { clearTimeout(cpTimer); cpTimer = setTimeout(checkpoint, 350); }
function restore() { setProjectKeepTime(JSON.parse(history[hIdx])); tl.draw(); updUndo(); }
function undo() { clearTimeout(cpTimer); checkpoint(); if (hIdx > 0) { hIdx--; restore(); status('Undo'); } }
function redo() { if (hIdx < history.length - 1) { hIdx++; restore(); status('Redo'); } }

const status = msg => ($('status').textContent = msg);
const hex = rgb => '#' + rgb.map(v => v.toString(16).padStart(2, '0')).join('');
const rgb = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));

const tl = new Timeline($('timeline'), () => refresh(), (t) => { tl.playhead = t; refresh(); });

function setProject(p) {
  project = p; tl.project = p; tl.playhead = 0; tl.selectedZoom = -1; tl.selectedClip = 0;
  $('empty').style.display = 'none';
  history = [JSON.stringify(p)]; hIdx = 0; updUndo();
  syncPanel(); tl.draw(); refresh();
}

function syncPanel() {
  const s = project.style, b = s.background;
  $('bg-type').value = b.type;
  if (b.type === 'gradient') { $('bg-from').value = hex(b.from); $('bg-to').value = hex(b.to); $('bg-angle').value = b.angle_deg; }
  if (b.type === 'solid') $('bg-from').value = hex(b.color);
  if (b.type === 'image') $('bg-image').value = b.path;
  $('padding').value = s.padding; $('radius').value = s.corner_radius; $('shadow').value = s.shadow;
  const c = project.cursor;
  $('cur-visible').checked = c.visible; $('cur-size').value = c.size; $('cur-smooth').value = c.smoothing_ms; $('cur-ripple').checked = c.click_ripples;
  $('keys-on').checked = project.keys.enabled; $('keys-short').checked = project.keys.shortcuts_only;
  $('cap-burn').checked = project.caption_style.burn_in; $('cap-size').value = project.caption_style.size;
  $('aud-denoise').checked = project.audio.denoise; $('aud-mute').checked = project.audio.mute;
  $('ex-aspect').value = project.export.aspect; $('ex-quality').value = project.export.quality; $('ex-fps').value = project.export.fps;
  renderBlurList();
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
  Object.assign(project.export, { aspect: $('ex-aspect').value, quality: +$('ex-quality').value, fps: +$('ex-fps').value });
}

function renderBlurList() {
  $('blur-list').innerHTML = '';
  project.blur.forEach((b, i) => {
    const d = document.createElement('div'); d.className = 'blur-item';
    d.innerHTML = `<span>${b.x},${b.y} ${b.w}x${b.h} (${(b.start_ms / 1000).toFixed(1)}-${(b.end_ms / 1000).toFixed(1)}s)</span>`;
    const x = document.createElement('button'); x.textContent = 'x'; x.onclick = () => { project.blur.splice(i, 1); renderBlurList(); refresh(); };
    d.appendChild(x); $('blur-list').appendChild(d);
  });
}

// Coalesce preview requests so slow frames never queue up.
async function refresh() {
  if (!project) return;
  if (!playing) scheduleCheckpoint();
  $('time').textContent = `${(tl.playhead / 1000).toFixed(1)} / ${(tl.outDur() / 1000).toFixed(1)} s`;
  if (pending) { dirty = true; return; }
  pending = true;
  try {
    $('preview').src = await invoke('preview', { project, outMs: tl.playhead, shortSide: 540 });
  } catch (e) { status(String(e)); }
  pending = false;
  if (dirty) { dirty = false; refresh(); }
}

async function playLoop() {
  let last = performance.now();
  while (playing) {
    const now = performance.now();
    tl.playhead += now - last; last = now;
    if (tl.playhead >= tl.outDur()) { tl.playhead = 0; playing = false; $('play').textContent = 'Play'; }
    tl.draw();
    pending = false; await refresh();
  }
}

document.querySelectorAll('#panel input:not([id^=cut-]):not(#docs-llm), #panel select:not(#cut-min)').forEach(el => el.addEventListener('input', () => { if (project) { readPanel(); refresh(); } }));

$('play').onclick = () => { if (!project) return; playing = !playing; $('play').textContent = playing ? 'Pause' : 'Play'; if (playing) playLoop(); };
$('split').onclick = () => {
  const at = tl.outToSrc(tl.playhead), i = project.clips.findIndex(c => at > c.src_start_ms && at < c.src_end_ms);
  if (i < 0) return;
  const c = project.clips[i];
  project.clips.splice(i + 1, 0, { src_start_ms: at, src_end_ms: c.src_end_ms, speed: c.speed });
  c.src_end_ms = at; tl.draw(); refresh();
};
$('del-clip').onclick = () => { if (project.clips.length > 1) { project.clips.splice(tl.selectedClip, 1); tl.selectedClip = 0; tl.draw(); refresh(); } };
$('speed').onchange = () => { const c = project.clips[tl.selectedClip]; if (c) { c.speed = +$('speed').value; tl.draw(); refresh(); } };
$('add-zoom').onclick = () => {
  const at = tl.outToSrc(tl.playhead);
  project.zoom.push({ start_ms: at, end_ms: at + 2000, scale: +$('zoom-scale').value, follow_cursor: true, focus: [project.source_width / 2, project.source_height / 2], reason: 'manual' });
  project.zoom.sort((a, b) => a.start_ms - b.start_ms); tl.draw(); refresh();
};
$('del-zoom').onclick = () => { if (tl.selectedZoom >= 0) { project.zoom.splice(tl.selectedZoom, 1); tl.selectedZoom = -1; tl.draw(); refresh(); } };
$('zoom-scale').oninput = () => { const z = project.zoom[tl.selectedZoom]; if (z) { z.scale = +$('zoom-scale').value; tl.draw(); refresh(); } };
$('auto-zoom').onclick = async () => { project.zoom = await invoke('regenerate_zoom', { project }); tl.draw(); refresh(); };
$('blur-add').onclick = () => {
  const v = $('blur-rect').value.split(',').map(Number);
  if (v.length !== 4 || v.some(isNaN)) return status('Blur box must be x,y,w,h');
  const at = tl.outToSrc(tl.playhead);
  project.blur.push({ x: v[0], y: v[1], w: v[2], h: v[3], start_ms: at, end_ms: project.duration_ms, radius: 18 });
  renderBlurList(); refresh();
};

async function openAny(path) {
  try {
    if (!path) path = await dialog.open({ multiple: false, filters: [{ name: 'Video or project', extensions: [...VIDEO_EXT, 'json'] }] });
    if (!path) return;
    if (path.endsWith('.json')) { setProject(await invoke('load_project', { path })); projectPath = path; }
    else {
      // Pick up the event log recorded next to the video, if there is one.
      const events = path.replace(/\.[^.\\/]+$/, '.events.jsonl');
      try { setProject(await invoke('open_recording', { video: path, events })); } catch { setProject(await invoke('open_recording', { video: path, events: null })); }
      projectPath = null;
    }
    $('project-name').textContent = baseName(path); status('Opened ' + baseName(path));
  } catch (e) { status(String(e)); }
}
async function save(as) {
  if (!project) return;
  try {
    if (as || !projectPath) {
      const p = await dialog.save({ defaultPath: projectPath || baseName(project.source).replace(/\.[^.]+$/, '') + '.demolens.json', filters: [{ name: 'DemoLens project', extensions: ['json'] }] });
      if (!p) return;
      projectPath = p;
    }
    await invoke('save_project', { path: projectPath, project });
    $('project-name').textContent = baseName(projectPath); status('Saved ' + baseName(projectPath));
  } catch (e) { status(String(e)); }
}
$('open-btn').onclick = () => openAny();
$('save-btn').onclick = () => save(false);
$('save-as-btn').onclick = () => save(true);
$('undo-btn').onclick = undo;
$('redo-btn').onclick = redo;
window.addEventListener('keydown', e => {
  const mod = e.ctrlKey || e.metaKey;
  if (!mod || ['INPUT', 'SELECT'].includes(document.activeElement.tagName) && document.activeElement.type !== 'range' && document.activeElement.type !== 'checkbox') return;
  const k = e.key.toLowerCase();
  if (k === 'z' && !e.shiftKey) { e.preventDefault(); undo(); }
  else if ((k === 'z' && e.shiftKey) || k === 'y') { e.preventDefault(); redo(); }
  else if (k === 's') { e.preventDefault(); save(e.shiftKey); }
  else if (k === 'o') { e.preventDefault(); openAny(); }
});

async function loadPresets() {
  const names = await invoke('list_presets');
  $('preset-list').innerHTML = names.map(n => `<option>${n}</option>`).join('');
}
$('preset-save').onclick = async () => { await invoke('save_preset', { name: $('preset-name').value, project }); // ---------- Local AI ----------
const esc = t => String(t).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]);
function modal(title, html) { $('modal-title').textContent = title; $('modal-body').innerHTML = html; if (!$('modal').open) $('modal').showModal(); }
$('modal-close').onclick = () => $('modal').close();

async function aiTask(label, fn) {
  if (!project || busy) return;
  busy = true; readPanel(); checkpoint();
  document.querySelectorAll('[id^=ai-]').forEach(b => (b.disabled = true));
  $('ai-progress').hidden = false; $('ai-progress').removeAttribute('value'); status(label + '...');
  try { await fn(); } catch (e) {
    const msg = String(e);
    status(msg);
    if (/model|install|download/i.test(msg)) showModels(msg);
  }
  busy = false; $('ai-progress').hidden = true;
  document.querySelectorAll('[id^=ai-]').forEach(b => (b.disabled = false));
}
window.__TAURI__.event.listen('ai-progress', e => {
  const p = e.payload;
  if (p.task.startsWith('download:')) {
    const bar = document.querySelector(`progress[data-model="${p.task.slice(9)}"]`);
    if (bar) { bar.hidden = false; bar.value = p.progress; bar.title = p.message; }
    const lbl = document.querySelector(`small[data-model="${p.task.slice(9)}"]`);
    if (lbl) lbl.textContent = p.message;
  } else { $('ai-progress').value = p.progress; status(p.message); }
});

function applyAi(r) { setProjectKeepTime(r.project); tl.draw(); checkpoint(); }

$('ai-transcribe').onclick = () => aiTask('Transcribing', async () => {
  const r = await invoke('ai_transcribe', { project, model: null });
  if (!r.project.caption_style.burn_in) r.project.caption_style.burn_in = true;
  applyAi(r);
  status(`${r.report.words} words, ${r.report.fillers} filler words, ${r.report.lines} caption lines. Captions are on; untick "Burn in captions" to keep them as a separate SRT only.`);
});
$('ai-autocut').onclick = () => aiTask('Finding pauses and filler words', async () => {
  const r = await invoke('ai_autocut', { project, fillers: $('cut-fillers').checked, silences: $('cut-silences').checked, minSilenceMs: +$('cut-min').value });
  applyAi(r);
  const n = r.report.cuts.length, f = r.report.cuts.filter(c => c[2] === 'filler').length;
  status(`${n} cuts (${f} filler words, ${n - f} pauses). ${(r.report.before_ms / 1000).toFixed(1)}s -> ${(r.report.after_ms / 1000).toFixed(1)}s. Ctrl+Z to undo.` + (!r.report.had_transcript && $('cut-fillers').checked ? ' Transcribe first to also cut filler words.' : ''));
});
$('ai-redact').onclick = () => aiTask('Scanning for secrets', async () => {
  const r = await invoke('ai_redact', { project });
  applyAi(r);
  status(`Blurred ${r.report.added} regions.` + (r.report.found.length ? '\nFound: ' + r.report.found.map(f => `${f[0]} ${f[1]}`).join(', ') : ' Nothing sensitive found.'));
});
$('ai-docs').onclick = () => aiTask('Writing docs', async () => {
  const r = await invoke('ai_docs', { project, useLlm: $('docs-llm').checked });
  status('Docs written by the ' + r.docs.generated_by);
  modal('Demo docs', `<p class="muted">${esc(r.docs.generated_by)}</p>
    <div class="row"><button id="doc-save">Save guide.md...</button><button id="doc-copy-ch">Copy chapters</button><button id="doc-copy-post">Copy post</button></div>
    <pre class="doc">${esc(r.markdown)}</pre>`);
  $('doc-save').onclick = async () => { const p = await dialog.save({ defaultPath: 'guide.md', filters: [{ name: 'Markdown', extensions: ['md'] }] }); if (p) { await invoke('write_text', { path: p, text: r.markdown }); status('Saved ' + baseName(p)); } };
  $('doc-copy-ch').onclick = () => navigator.clipboard.writeText(r.chapters).then(() => status('Chapters copied'));
  $('doc-copy-post').onclick = () => navigator.clipboard.writeText(r.docs.social_post).then(() => status('Post copied'));
});
$('cap-export').onclick = async () => {
  if (!project) return;
  if (!project.captions.length) return status('Transcribe first to get captions.');
  const p = await dialog.save({ defaultPath: baseName(project.source).replace(/\.[^.]+$/, '') + '.srt', filters: [{ name: 'Subtitles', extensions: ['srt', 'vtt'] }] });
  if (p) { try { await invoke('export_captions', { project, path: p }); status('Wrote ' + baseName(p)); } catch (e) { status(String(e)); } }
};

async function showModels(note) {
  const m = await invoke('models_list');
  const kinds = { speech: 'Speech to text', ocr: 'Text recognition (auto blur)', llm: 'Writing (docs)', runtime: 'LLM runtime' };
  modal('Local models', `<p>${note ? `<span class="found">${esc(note)}</span><br>` : ''}Models run on this computer and are stored in <code>${esc(m.dir)}</code>. Nothing is uploaded. Ollama or any local OpenAI compatible server is used for docs automatically when it is running.</p>` +
    m.models.map(x => `<div class="model"><div><b>${esc(x.title)}</b> <small data-model="${x.id}">${kinds[x.kind] || x.kind} · ${x.size_human} · ${esc(x.license)}</small></div>
      <span>${x.installed ? 'Installed' : ''}</span>
      <button data-id="${x.id}" data-act="${x.installed ? 'delete' : 'download'}">${x.installed ? 'Delete' : 'Download ' + x.size_human}</button>
      <progress data-model="${x.id}" max="1" value="0" hidden></progress></div>`).join(''));
  document.querySelectorAll('#modal-body button[data-id]').forEach(b => (b.onclick = async () => {
    b.disabled = true;
    try {
      if (b.dataset.act === 'delete') await invoke('model_delete', { id: b.dataset.id });
      else await invoke('model_download', { id: b.dataset.id });
    } catch (e) { status(String(e)); }
    showModels();
  }));
}
$('ai-models').onclick = () => showModels();

loadPresets(); status('Preset saved'); };
$('preset-apply').onclick = async () => { setProjectKeepTime(await invoke('apply_preset', { name: $('preset-list').value, project })); };
function setProjectKeepTime(p) { const t = tl.playhead; project = p; tl.project = p; tl.playhead = t; syncPanel(); refresh(); }

$('ex-btn').onclick = async () => {
  if (!project) return;
  readPanel();
  const out = await dialog.save({ defaultPath: baseName(project.source).replace(/\.[^.]+$/, '') + '-demo.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }, { name: 'GIF', extensions: ['gif'] }] });
  if (!out) return;
  project.export.format = out.toLowerCase().endsWith('.gif') ? 'gif' : 'mp4';
  status('Exporting...');
  try { status('Exported ' + await invoke('export_video', { project, out })); } catch (e) { status(String(e)); }
};
window.__TAURI__.event.listen('export-progress', e => ($('ex-progress').value = e.payload));

$('rec-btn').onclick = async () => {
  try {
    if (!recording) {
      const kind = $('rec-target').value, arg = $('rec-arg').value;
      let target = { type: 'screen', display: 0 };
      if (kind === 'window') target = { type: 'window', id: arg };
      if (kind === 'area') { const [x, y, w, h] = arg.split(',').map(Number); target = { type: 'area', x, y, w, h }; }
      const warn = await invoke('start_recording', { target, mic: $('rec-mic').checked, systemAudio: $('rec-sys').checked });
      recording = true; $('rec-btn').textContent = 'Stop'; $('rec-btn').classList.add('recording');
      status(warn.join('\n') || 'Recording...');
    } else {
      recording = false; $('rec-btn').textContent = 'Record'; $('rec-btn').classList.remove('recording');
      setProject(await invoke('stop_recording')); status('Recording loaded');
    }
  } catch (e) { recording = false; $('rec-btn').textContent = 'Record'; $('rec-btn').classList.remove('recording'); status(String(e)); }
};

// ---------- Local AI ----------
const esc = t => String(t).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]);
function modal(title, html) { $('modal-title').textContent = title; $('modal-body').innerHTML = html; if (!$('modal').open) $('modal').showModal(); }
$('modal-close').onclick = () => $('modal').close();

async function aiTask(label, fn) {
  if (!project || busy) return;
  busy = true; readPanel(); checkpoint();
  document.querySelectorAll('[id^=ai-]').forEach(b => (b.disabled = true));
  $('ai-progress').hidden = false; $('ai-progress').removeAttribute('value'); status(label + '...');
  try { await fn(); } catch (e) {
    const msg = String(e);
    status(msg);
    if (/model|install|download/i.test(msg)) showModels(msg);
  }
  busy = false; $('ai-progress').hidden = true;
  document.querySelectorAll('[id^=ai-]').forEach(b => (b.disabled = false));
}
window.__TAURI__.event.listen('ai-progress', e => {
  const p = e.payload;
  if (p.task.startsWith('download:')) {
    const bar = document.querySelector(`progress[data-model="${p.task.slice(9)}"]`);
    if (bar) { bar.hidden = false; bar.value = p.progress; bar.title = p.message; }
    const lbl = document.querySelector(`small[data-model="${p.task.slice(9)}"]`);
    if (lbl) lbl.textContent = p.message;
  } else { $('ai-progress').value = p.progress; status(p.message); }
});

function applyAi(r) { setProjectKeepTime(r.project); tl.draw(); checkpoint(); }

$('ai-transcribe').onclick = () => aiTask('Transcribing', async () => {
  const r = await invoke('ai_transcribe', { project, model: null });
  if (!r.project.caption_style.burn_in) r.project.caption_style.burn_in = true;
  applyAi(r);
  status(`${r.report.words} words, ${r.report.fillers} filler words, ${r.report.lines} caption lines. Captions are on; untick "Burn in captions" to keep them as a separate SRT only.`);
});
$('ai-autocut').onclick = () => aiTask('Finding pauses and filler words', async () => {
  const r = await invoke('ai_autocut', { project, fillers: $('cut-fillers').checked, silences: $('cut-silences').checked, minSilenceMs: +$('cut-min').value });
  applyAi(r);
  const n = r.report.cuts.length, f = r.report.cuts.filter(c => c[2] === 'filler').length;
  status(`${n} cuts (${f} filler words, ${n - f} pauses). ${(r.report.before_ms / 1000).toFixed(1)}s -> ${(r.report.after_ms / 1000).toFixed(1)}s. Ctrl+Z to undo.` + (!r.report.had_transcript && $('cut-fillers').checked ? ' Transcribe first to also cut filler words.' : ''));
});
$('ai-redact').onclick = () => aiTask('Scanning for secrets', async () => {
  const r = await invoke('ai_redact', { project });
  applyAi(r);
  status(`Blurred ${r.report.added} regions.` + (r.report.found.length ? '\nFound: ' + r.report.found.map(f => `${f[0]} ${f[1]}`).join(', ') : ' Nothing sensitive found.'));
});
$('ai-docs').onclick = () => aiTask('Writing docs', async () => {
  const r = await invoke('ai_docs', { project, useLlm: $('docs-llm').checked });
  status('Docs written by the ' + r.docs.generated_by);
  modal('Demo docs', `<p class="muted">${esc(r.docs.generated_by)}</p>
    <div class="row"><button id="doc-save">Save guide.md...</button><button id="doc-copy-ch">Copy chapters</button><button id="doc-copy-post">Copy post</button></div>
    <pre class="doc">${esc(r.markdown)}</pre>`);
  $('doc-save').onclick = async () => { const p = await dialog.save({ defaultPath: 'guide.md', filters: [{ name: 'Markdown', extensions: ['md'] }] }); if (p) { await invoke('write_text', { path: p, text: r.markdown }); status('Saved ' + baseName(p)); } };
  $('doc-copy-ch').onclick = () => navigator.clipboard.writeText(r.chapters).then(() => status('Chapters copied'));
  $('doc-copy-post').onclick = () => navigator.clipboard.writeText(r.docs.social_post).then(() => status('Post copied'));
});
$('cap-export').onclick = async () => {
  if (!project) return;
  if (!project.captions.length) return status('Transcribe first to get captions.');
  const p = await dialog.save({ defaultPath: baseName(project.source).replace(/\.[^.]+$/, '') + '.srt', filters: [{ name: 'Subtitles', extensions: ['srt', 'vtt'] }] });
  if (p) { try { await invoke('export_captions', { project, path: p }); status('Wrote ' + baseName(p)); } catch (e) { status(String(e)); } }
};

async function showModels(note) {
  const m = await invoke('models_list');
  const kinds = { speech: 'Speech to text', ocr: 'Text recognition (auto blur)', llm: 'Writing (docs)', runtime: 'LLM runtime' };
  modal('Local models', `<p>${note ? `<span class="found">${esc(note)}</span><br>` : ''}Models run on this computer and are stored in <code>${esc(m.dir)}</code>. Nothing is uploaded. Ollama or any local OpenAI compatible server is used for docs automatically when it is running.</p>` +
    m.models.map(x => `<div class="model"><div><b>${esc(x.title)}</b> <small data-model="${x.id}">${kinds[x.kind] || x.kind} · ${x.size_human} · ${esc(x.license)}</small></div>
      <span>${x.installed ? 'Installed' : ''}</span>
      <button data-id="${x.id}" data-act="${x.installed ? 'delete' : 'download'}">${x.installed ? 'Delete' : 'Download ' + x.size_human}</button>
      <progress data-model="${x.id}" max="1" value="0" hidden></progress></div>`).join(''));
  document.querySelectorAll('#modal-body button[data-id]').forEach(b => (b.onclick = async () => {
    b.disabled = true;
    try {
      if (b.dataset.act === 'delete') await invoke('model_delete', { id: b.dataset.id });
      else await invoke('model_download', { id: b.dataset.id });
    } catch (e) { status(String(e)); }
    showModels();
  }));
}
$('ai-models').onclick = () => showModels();

loadPresets();
invoke('initial_project').then(r => { if (r) { setProject(r.project); projectPath = r.path; $('project-name').textContent = baseName(r.path || r.project.source); } }).catch(e => status(String(e)));
