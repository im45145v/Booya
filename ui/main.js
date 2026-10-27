// DemoLens editor UI. All pixels come from the Rust renderer via `preview`.
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = id => document.getElementById(id);
let project = null, playing = false, pending = false, dirty = false, recording = false;

const status = msg => ($('status').textContent = msg);
const hex = rgb => '#' + rgb.map(v => v.toString(16).padStart(2, '0')).join('');
const rgb = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));

const tl = new Timeline($('timeline'), () => refresh(), (t) => { tl.playhead = t; refresh(); });

function setProject(p) {
  project = p; tl.project = p; tl.playhead = 0; tl.selectedZoom = -1; tl.selectedClip = 0;
  $('empty').style.display = 'none';
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

document.querySelectorAll('#panel input, #panel select').forEach(el => el.addEventListener('input', () => { if (project) { readPanel(); refresh(); } }));

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

$('open-btn').onclick = async () => { try { setProject(await invoke('open_recording', { video: $('open-video').value, events: null })); } catch (e) { status(String(e)); } };
$('load-btn').onclick = async () => { try { setProject(await invoke('load_project', { path: $('project-path').value })); } catch (e) { status(String(e)); } };
$('save-btn').onclick = async () => { try { await invoke('save_project', { path: $('project-path').value, project }); status('Saved'); } catch (e) { status(String(e)); } };

async function loadPresets() {
  const names = await invoke('list_presets');
  $('preset-list').innerHTML = names.map(n => `<option>${n}</option>`).join('');
}
$('preset-save').onclick = async () => { await invoke('save_preset', { name: $('preset-name').value, project }); loadPresets(); status('Preset saved'); };
$('preset-apply').onclick = async () => { setProjectKeepTime(await invoke('apply_preset', { name: $('preset-list').value, project })); };
function setProjectKeepTime(p) { const t = tl.playhead; project = p; tl.project = p; tl.playhead = t; syncPanel(); refresh(); }

$('ex-btn').onclick = async () => {
  readPanel(); status('Exporting...');
  try { status('Exported ' + await invoke('export_video', { project, out: $('ex-path').value })); } catch (e) { status(String(e)); }
};
window.__TAURI__.event.listen('export-progress', e => ($('ex-progress').value = e.payload));

$('rec-btn').onclick = async () => {
  try {
    if (!recording) {
      const kind = $('rec-target').value, arg = $('rec-arg').value;
      let target = { type: 'screen', display: 0 };
      if (kind === 'window') target = { type: 'window', id: arg };
      if (kind === 'area') { const [x, y, w, h] = arg.split(',').map(Number); target = { type: 'area', x, y, w, h }; }
      const outDir = `${Date.now()}`;
      const warn = await invoke('start_recording', { target, mic: $('rec-mic').checked, systemAudio: $('rec-sys').checked, outDir: 'DemoLens-' + outDir });
      recording = true; $('rec-btn').textContent = 'Stop'; $('rec-btn').classList.add('recording');
      status(warn.join('\n') || 'Recording...');
    } else {
      recording = false; $('rec-btn').textContent = 'Record'; $('rec-btn').classList.remove('recording');
      setProject(await invoke('stop_recording')); status('Recording loaded');
    }
  } catch (e) { recording = false; $('rec-btn').textContent = 'Record'; $('rec-btn').classList.remove('recording'); status(String(e)); }
};

loadPresets();
invoke('initial_project').then(p => { if (p) setProject(p); }).catch(e => status(String(e)));
