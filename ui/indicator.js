// The recording pill: time, microphone level, pause, stop and discard.
// It stays out of the recording (the window is content protected) and fades
// when the mouse is elsewhere.
const T = window.__TAURI__;
const $ = id => document.getElementById(id);
const invoke = (c, a) => T.core.invoke(c, a);
if (/Mac/i.test(navigator.platform)) document.body.classList.add('mac');

let paused = false, baseMs = 0, baseAt = performance.now(), pausedMs = 0, quietSince = null, lastLevel = 0;
const fmt = ms => { const s = Math.max(0, Math.floor(ms / 1000)); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };

function render() {
  $('pill').classList.toggle('paused', paused);
  $('state').textContent = paused ? 'Paused' : '';
  $('pause').innerHTML = paused
    ? '<svg viewBox="0 0 24 24"><path d="M8 5.5v13l11-6.5z"/></svg>'
    : '<svg viewBox="0 0 24 24"><rect x="6.5" y="5" width="4" height="14" rx="1.2"/><rect x="13.5" y="5" width="4" height="14" rx="1.2"/></svg>';
  $('pause').title = paused ? 'Resume (Shift+Cmd/Ctrl+P)' : 'Pause: stops cursor and key logging and cuts this part (Shift+Cmd/Ctrl+P)';
  $('pause').setAttribute('aria-label', paused ? 'Resume' : 'Pause');
}

async function poll() {
  try {
    const s = await invoke('rec_status');
    if (s.recording) {
      baseMs = s.elapsed_ms; pausedMs = s.paused_ms; baseAt = performance.now(); paused = s.paused; render();
    }
  } catch { }
}
setInterval(() => {
  const wall = baseMs + (paused ? 0 : performance.now() - baseAt);
  // While paused the clock holds, and paused time never counts.
  $('time').textContent = fmt(wall - pausedMs);
}, 250);
setInterval(poll, 2000); poll();

$('pause').onclick = async () => { try { paused = await invoke('pause_recording', { paused: !paused }); await poll(); render(); } catch { } };
$('stop').onclick = () => invoke('request_stop').catch(() => { });
$('trash').onclick = () => { $('confirm').hidden = false; $('no').focus(); setTimeout(() => ($('confirm').hidden = true), 6000); };
$('no').onclick = () => ($('confirm').hidden = true);
$('yes').onclick = () => invoke('discard_recording').catch(() => { });
T.event.listen('rec-paused', e => { paused = !!e.payload; poll(); });

// Microphone level: five bars from the peak, amber when the mic is very quiet.
T.event.listen('mic-level', e => {
  const { rms, peak } = e.payload; lastLevel = rms;
  $('meter').hidden = false;
  const lvl = Math.max(0, Math.min(1, (peak + 60) / 60));
  [...$('meter').children].forEach((b, i) => { const on = lvl > i / 5 + 0.02; b.classList.toggle('on', on); b.style.height = (4 + (on ? (i + 1) * 2.4 : 0)) + 'px'; });
  const quiet = rms < -55;
  if (quiet) quietSince ??= performance.now(); else quietSince = null;
  const isQuiet = quietSince && performance.now() - quietSince > 4000;
  $('meter').classList.toggle('quiet', !!isQuiet);
  $('meter').title = isQuiet ? 'The microphone is very quiet. Check the input volume in System Settings > Sound.' : 'Microphone level';
});

// Fade when the mouse is elsewhere.
let dimTimer = null;
const wake = () => { $('pill').classList.remove('dim'); clearTimeout(dimTimer); dimTimer = setTimeout(() => $('pill').classList.add('dim'), 3500); };
document.addEventListener('mousemove', wake); document.addEventListener('mouseenter', wake);
document.addEventListener('mouseleave', () => { clearTimeout(dimTimer); dimTimer = setTimeout(() => $('pill').classList.add('dim'), 1200); });
wake(); render();
