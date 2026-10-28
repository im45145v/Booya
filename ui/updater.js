// Updates: look for a newer release at launch (unless turned off), show a
// banner with the notes, install and restart on request. Also the About tab in
// Settings. Loaded after main.js and shares its globals.

const AUTO_UPDATE_KEY = 'auto-update';
const autoUpdateOn = () => localStorage.getItem(AUTO_UPDATE_KEY) !== '0';
let lastUpdate = null;

// "Release notes" arrive as Markdown. Show them as plain readable text.
function plainNotes(md) {
  return String(md || '').replace(/\r/g, '').replace(/^#{1,6}\s*/gm, '').replace(/\*\*(.+?)\*\*/g, '$1').replace(/`([^`]+)`/g, '$1').trim();
}

async function checkForUpdate(quiet) {
  let u;
  try { u = await invoke('check_update'); }
  catch (e) {
    if (!quiet) status('Could not check for updates: ' + humanError(e), 'error');
    return null;
  }
  if (!u) return null;
  lastUpdate = u;
  if (u.available) showUpdateBanner(u);
  else if (!quiet) status(`DemoLens ${u.current} is the latest version.`, 'success');
  return u;
}

function showUpdateBanner(u) {
  $('update-text').textContent = `DemoLens ${u.version} is available (you have ${u.current}).`;
  $('update-banner').hidden = false;
  $('update-install').textContent = u.can_install ? 'Install and restart' : 'Open download page';
  $('update-notes').onclick = () => modal(`DemoLens ${u.version}`, `<div class="about-notes">${esc(plainNotes(u.notes) || 'No notes for this release.')}</div>${updateCaveat()}`);
  $('update-later').onclick = () => { $('update-banner').hidden = true; };
  $('update-install').onclick = () => installUpdate(u);
}

const updateCaveat = () => IS_MAC ? '<p class="hint">After an update macOS may ask for Screen Recording, Microphone and Input Monitoring again, because DemoLens is signed ad hoc and each new build looks like a new app to the system. Allow them again, then reopen DemoLens.</p>' : '';

async function installUpdate(u) {
  if (!u.can_install) return modal('Update DemoLens', '<p>This copy was installed from a .deb or .rpm package, so it cannot replace itself. Update it with your package manager, or get the new version from <a href="https://github.com/im45145v/DemoLens/releases/latest" target="_blank">github.com/im45145v/DemoLens/releases/latest</a>. The AppImage updates itself.</p>');
  if (typeof recording !== 'undefined' && recording) return status('Finish the recording before updating.', 'error');
  if (saved !== null && history[hIdx] !== saved) {
    const ok = await confirmBox('Update with unsaved changes?', '<p>You have unsaved changes. Save first, or they will be lost when DemoLens restarts.</p>');
    if (!ok) return;
  }
  const btn = $('update-install'); btn.disabled = true;
  const off = await window.__TAURI__.event.listen('update-progress', ev => {
    const { got, total } = ev.payload || {};
    $('update-text').textContent = total ? `Downloading ${(100 * got / total).toFixed(0)}% of ${u.version}...` : `Downloading ${u.version}...`;
  });
  $('update-text').textContent = `Downloading ${u.version}...`;
  try { await invoke('install_update'); status('Installed. Restarting...', 'success'); }
  catch (e) { off(); btn.disabled = false; $('update-text').textContent = `DemoLens ${u.version} is available (you have ${u.current}).`; fail(e); }
}

async function renderAboutTab() {
  const host = $('about-body'); if (!host) return;
  let st = {}; try { st = await invoke('ffmpeg_status'); } catch { }
  host.innerHTML = `<p><b>DemoLens ${esc(st.app_version || '')}</b>. Free and open source under the MIT license. <a href="https://github.com/im45145v/DemoLens" target="_blank">github.com/im45145v/DemoLens</a></p>
    <h3 class="section">Updates</h3>
    <div class="row"><button class="btn primary" id="upd-check">Check for updates</button><span id="upd-result" class="muted small"></span></div>
    <label class="switch"><input type="checkbox" id="upd-auto" ${autoUpdateOn() ? 'checked' : ''} /> Check for updates when DemoLens starts</label>
    <p class="muted small">Checking asks GitHub for a small file (latest.json) and sends nothing else. Updates are signed and checked against a key built into the app before they are installed.</p>
    ${updateCaveat()}
    <p><b>ffmpeg:</b> ${st.found ? esc(st.version) + (st.bundled ? ' (bundled)' : '') + '<br><span class="hint">' + esc(st.path) + '</span>' : 'not found'}</p>
    <p class="hint">This app bundles FFmpeg (https://ffmpeg.org), licensed under the GNU GPL version 2 or later (the bundled builds include GPL components). FFmpeg is a trademark of Fabrice Bellard. Source code for the bundled builds is available from the builders: BtbN/FFmpeg-Builds (Linux, Windows) and ffmpeg.martin-riedl.de (macOS). See THIRD_PARTY_NOTICES.md in the repository.</p>
    <p class="hint">Models are downloaded separately and keep their own licenses.</p>`;
  $('upd-auto').onchange = () => localStorage.setItem(AUTO_UPDATE_KEY, $('upd-auto').checked ? '1' : '0');
  $('upd-check').onclick = async () => {
    const out = $('upd-result'); out.textContent = 'Checking...';
    const u = await checkForUpdate(true);
    if (!u) { out.textContent = 'Could not reach GitHub. Check your connection and try again.'; return; }
    out.textContent = u.available ? `Version ${u.version} is available.` : `You have the latest version (${u.current}).`;
  };
}

// Look a few seconds after launch so start up stays quick, and never while recording.
setTimeout(() => { if (autoUpdateOn() && !(typeof recording !== 'undefined' && recording)) checkForUpdate(true); }, 6000);
