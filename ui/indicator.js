// Always on top recording badge. Shows that DemoLens is recording, for how
// long, and whether input capture is paused.
const T = window.__TAURI__;
const t0 = Date.now();
let paused = false;
const $ = id => document.getElementById(id);
function show() {
  $('label').textContent = paused ? 'PAUSED' : 'REC';
  $('dot').classList.toggle('paused', paused);
  $('pause').textContent = paused ? 'Resume' : 'Pause';
}
setInterval(() => {
  const s = Math.floor((Date.now() - t0) / 1000);
  $('time').textContent = Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0');
}, 500);
$('pause').onclick = async () => { try { paused = await T.core.invoke('pause_recording', { paused: !paused }); show(); } catch {} };
T.event.listen('rec-paused', e => { paused = !!e.payload; show(); });
show();
