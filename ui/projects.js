// Projects and takes: project folders (open, save as, duplicate, find missing
// media), the Recent list on the welcome screen, autosave, and putting a new
// recording or a video file into the project you are editing.
// Loaded after main.js and shares its globals.

// ---------- Open ----------
async function openProjectPath(path) {
  const r = await invoke('open_project', { path });
  let p = r.project;
  if (r.recovered) {
    const use = confirm(`DemoLens found edits to "${baseName(r.path)}" that were never saved (the app was closed first).\n\nOK restores them. Cancel keeps the last saved version.`);
    if (use) p = r.recovered; else invoke('discard_autosave', { path: r.path }).catch(() => { });
  }
  setProject(p, r.path);
  if (r.recovered && p === r.recovered) { saved = null; status('Restored the unsaved edits. Press Save to keep them.', 'info'); updTitle(); }
  if (r.missing.length) await relocateDialog(r.missing);
  renderRecent();
  return r;
}

async function openProjectFolder() {
  try {
    const dir = await invoke('pick_folder');
    if (!dir) return;
    showBusy('Opening ' + baseName(dir) + '...');
    await openProjectPath(dir);
    status('Opened ' + baseName(dir), 'success');
  } catch (e) { fail(e); }
  hideBusy();
}

// ---------- Missing media ----------
const MISSING_KIND = {
  source: ['the recording', 'Video', VIDEO_EXT],
  events: ['the mouse and key log', 'Event log', ['jsonl']],
  webcam: ['the camera video', 'Video', VIDEO_EXT],
  background: ['the background photo', 'Photo', ['png', 'jpg', 'jpeg', 'webp', 'heic', 'bmp', 'gif']],
};
async function relocateDialog(missing) {
  const draw = () => {
    modal('Some files are missing', `<p>This project points at files that are no longer where it expects them. Find them to continue, or carry on without.</p>` +
      missing.map((m, i) => `<div class="item"><span>${esc(MISSING_KIND[m.field]?.[0] || m.field)}<br><small class="muted">${esc(m.path)}</small></span><button class="btn sm" data-loc="${i}">Locate...</button></div>`).join('') +
      `<div class="row"><button class="btn primary" id="loc-done">${missing.length ? 'Continue without' : 'Done'}</button></div>`);
    $('modal-body').querySelectorAll('[data-loc]').forEach(b => b.onclick = async () => {
      const m = missing[+b.dataset.loc], k = MISSING_KIND[m.field];
      try {
        const path = await invoke('pick_open', { name: k?.[1] || 'File', extensions: k?.[2] || ['mp4'] });
        if (!path) return;
        const r = await invoke('relocate_media', { project, field: m.field, path });
        project = r.project; tl.project = project; missing = r.missing;
        if (m.field === 'source') { tl.setProject(project); prepareProxy(); }
        syncPanel(); tl.draw(); refresh(); checkpoint();
        if (missing.length) draw(); else { $('modal').close(); status('Found it. Save the project to keep the new location.', 'success'); }
      } catch (e) { fail(e); }
    });
    $('loc-done').onclick = () => $('modal').close();
  };
  draw();
}

// ---------- Recent projects (welcome screen) ----------
async function renderRecent() {
  const box = $('w-recent'); if (!box) return;
  let list = []; try { list = await invoke('recent_projects'); } catch { }
  box.hidden = !list.length;
  box.innerHTML = list.length ? '<h3 class="section">Recent projects</h3>' + list.map(r => `<div class="item"><button class="link" data-open="${esc(r.path)}" ${r.exists ? '' : 'disabled'}>${esc(r.name)}</button><small class="muted">${esc(r.exists ? r.folder : 'moved or deleted')}</small><button class="btn ghost sm" data-forget="${esc(r.path)}" aria-label="Remove from the list">Remove</button></div>`).join('') : '';
  box.querySelectorAll('[data-open]').forEach(b => b.onclick = async () => {
    showBusy('Opening ' + baseName(b.dataset.open) + '...');
    try { await openProjectPath(b.dataset.open); status('Opened ' + baseName(b.dataset.open), 'success'); } catch (e) { fail(e); }
    hideBusy();
  });
  box.querySelectorAll('[data-forget]').forEach(b => b.onclick = async () => { await invoke('forget_recent', { path: b.dataset.forget }); renderRecent(); });
}
$('w-open-folder').onclick = () => openProjectFolder();
renderRecent();

// ---------- Duplicate ----------
async function duplicateProject() {
  if (!project) return;
  try {
    const target = await dialog.save({ defaultPath: stem(project.source) + ' copy.demolens', filters: [{ name: 'DemoLens project', extensions: ['demolens'] }] });
    if (!target) return;
    readPanel(); checkpoint();
    showBusy('Copying the project...');
    try { await invoke('duplicate_project', { path: target, project }); } finally { hideBusy(); }
    status('Copied to ' + baseName(target) + '. You are still editing the original.', 'success');
  } catch (e) { fail(e); }
}

// ---------- Project menu ----------
$('proj-btn').onclick = () => {
  modal('Project', `<p class="muted small">A project is a folder ending in .demolens. It holds the edit and a copy of the recording, so you can move it, back it up or send it.</p>
    <div class="list">
      <button class="btn" data-pm="save-as">Save as...</button>
      <button class="btn" data-pm="duplicate">Duplicate...</button>
      <button class="btn" data-pm="open-folder">Open a project folder...</button>
      <button class="btn" data-pm="import-clip">Import a video clip at the playhead...</button>
    </div>`);
  $('modal-body').querySelectorAll('[data-pm]').forEach(b => b.onclick = () => { $('modal').close(); act(b.dataset.pm); });
};

// ---------- Autosave ----------
let lastAutosave = null;
async function autosaveTick() {
  if (!project || busy || recording) return;
  const s = JSON.stringify(project);
  if (s === lastAutosave) return;
  try { await invoke('autosave_project', { path: projectPath, project }); lastAutosave = s; } catch { }
}
setInterval(autosaveTick, 15000);
window.addEventListener('blur', () => autosaveTick());
window.addEventListener('beforeunload', () => { autosaveTick(); });

// ---------- Takes: record here, re-record, import ----------
let takePlan = null;
function clearTakePlan() { takePlan = null; }
function takeInsertArgs() {
  const t = takePlan; takePlan = null;
  if (!t || !project) return null;
  readPanel();
  return { project, at_ms: t.at, until_ms: t.until };
}
function fmtT(ms) { const s = ms / 1000; return Math.floor(s / 60) + ':' + (s % 60).toFixed(1).padStart(4, '0'); }
async function recordHere(over) {
  if (!project || recording || busy) return;
  let at = tl.outToSrc(tl.playhead), until = null, what;
  if (over) {
    const c = project.clips[tl.selectedClip];
    if (!c) return status('Select a clip first, or split the part you want to redo.', 'error');
    at = c.src_start_ms; until = c.src_end_ms;
    what = `Recording over the selected clip (${fmtT(at)} to ${fmtT(until)}). Undo brings the old one back.`;
  } else {
    if (at == null || at > project.duration_ms) at = project.duration_ms;
    what = `Recording a new take to put in at ${fmtT(at)}.`;
  }
  takePlan = { at, until };
  if (playing) setPlaying(false);
  $('rec-btn').click();
  // The 3, 2, 1 countdown follows; the status bar line is replaced by the recorder's own.
  setTimeout(() => { if (recording && takePlan) status(what + ' Press Stop when you are done.', 'busy'); }, 3600);
}
$('rec-here').onclick = () => recordHere(false);
$('rec-over').onclick = () => recordHere(true);

function applySplice(p, ins) {
  const hadCam = !!project.webcam && project.webcam.visible;
  project = p; tl.project = p; tl.setProject(p);
  tl.playhead = Math.min(tl.outDur(), tl.srcToOut ? (tl.srcToOut(ins.at_ms) ?? 0) : 0);
  syncPanel(); syncSelection(); tl.draw(); refresh(); checkpoint(); prepareProxy();
  status('New take added at ' + fmtT(ins.at_ms) + '.' + (hadCam ? ' The camera track was hidden: it cannot line up with a recording that now has a new piece in it.' : '') + ' Undo removes it.', 'success');
}

async function importClip() {
  if (!project || busy) return;
  try {
    const path = await dialog.open({ filters: [{ name: 'Video', extensions: VIDEO_EXT }] });
    if (!path) return;
    readPanel(); checkpoint();
    let at = tl.outToSrc(tl.playhead); if (at == null || at > project.duration_ms) at = project.duration_ms;
    if (playing) setPlaying(false);
    showBusy('Adding ' + baseName(path) + ' to the timeline...');
    const p = await invoke('import_clip', { project, path, atMs: at, untilMs: null });
    applySplice(p, { at_ms: at });
  } catch (e) { fail(e); }
  hideBusy();
}
