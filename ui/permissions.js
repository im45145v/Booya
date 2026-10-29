// The permissions screen: shown on first launch on a Mac, whenever a
// permission that was on has been switched off (this happens after an update
// of an app that is not notarised), and always in Settings > Permissions.
// Each row's state is read from the system every second, so it turns green
// as soon as the person flips the switch in System Settings.
(function (root) {
  'use strict';
  const invoke = (...a) => root.__TAURI__.core.invoke(...a);
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const LABEL = { screen: 'Screen Recording', mic: 'Microphone', input: 'Input Monitoring', 'system-audio': 'System audio' };
  const clicked = new Set();
  const CHECK = '<svg class="ico" viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12.5 4.5 4.5L19 7.5"/></svg>';

  // The first time a button is pressed the system's own prompt appears;
  // after that the system will not ask again, so the button opens the page.
  function action(i) {
    if (i.status === 'granted') return `<div class="perm-ok" role="status">${CHECK}<span>${esc(LABEL[i.id] || i.title)} enabled</span></div>`;
    if (i.status === 'not-needed') return '<div class="perm-na">Nothing to allow here</div>';
    const open = i.status === 'denied' && (i.id !== 'screen' || clicked.has(i.id)) || clicked.has(i.id);
    return `<button class="perm-btn" data-perm="${i.id}">${open ? 'Open System Settings' : 'Allow ' + esc(LABEL[i.id] || i.title)}</button>`;
  }

  function rows(d) {
    return d.items.map(i => `<div class="perm-row" data-state="${i.status}">
      <div class="perm-text"><div class="perm-title">${esc(i.title)} <span class="perm-tag">${i.required ? 'Required' : 'Optional'}</span></div>
        <div class="perm-detail">${esc(i.detail)}${i.note && i.status !== 'granted' ? ' ' + esc(i.note) : ''}</div></div>
      <div class="perm-act">${action(i)}</div></div>`).join('');
  }

  function note(d) {
    if (d.platform === 'macos') return '<p class="perm-fine">Booya does not need the Accessibility permission. Clicks, the cursor and scrolling are read without it.</p>';
    if (d.platform === 'windows') return '<p class="perm-fine">If the microphone stays silent, open Settings > Privacy > Microphone and switch on "Let desktop apps access your microphone".</p>';
    return '';
  }

  function bind(host, onChange) {
    host.querySelectorAll('[data-perm]').forEach(b => (b.onclick = async () => {
      const id = b.dataset.perm; clicked.add(id);
      try { await invoke('permissions_request', { id }); } catch (e) { console.warn(e); }
      onChange();
    }));
  }

  // Remember what was on, so a permission that later goes off brings this screen back.
  function remember(d) { localStorage.setItem('perm-granted', JSON.stringify(d.items.filter(i => i.status === 'granted').map(i => i.id))); }
  function regressed(d) {
    let was = []; try { was = JSON.parse(localStorage.getItem('perm-granted') || '[]'); } catch { }
    return d.items.filter(i => was.includes(i.id) && i.status !== 'granted' && i.status !== 'not-needed');
  }

  // Inside Settings: just the rows.
  function mount(host, opts = {}) {
    if (!host) return;
    let stop = false;
    const draw = async () => {
      if (stop || !host.isConnected) { stop = true; return; }
      const d = await invoke('permissions_list').catch(() => null); if (!d || stop) return;
      remember(d);
      host.innerHTML = `<p class="muted small">What the system lets Booya do. These update by themselves when you change them.</p><div class="perm-rows">${rows(d)}</div>${note(d)}<p class="perm-fine">Booya never sends your recordings anywhere.</p>`;
      bind(host, draw);
    };
    draw();
    const t = setInterval(() => { if (stop || !host.isConnected) return clearInterval(t); draw(); }, 1000);
  }

  // The full screen. Resolves when the person continues.
  function overlay(d, why) {
    return new Promise(res => {
      const el = document.createElement('div');
      el.id = 'perm-overlay'; el.setAttribute('role', 'dialog'); el.setAttribute('aria-modal', 'true'); el.setAttribute('aria-labelledby', 'perm-h');
      document.body.appendChild(el);
      let stop = false;
      const canGo = x => x.items.filter(i => i.required).every(i => i.status === 'granted');
      const paint = x => {
        const ok = canGo(x);
        el.innerHTML = `<div class="perm-card">
          <img class="perm-logo" src="logo.svg" alt="" width="64" height="64" />
          <h1 id="perm-h">Welcome to Booya</h1>
          <p class="perm-lede">${why ? esc(why) : 'Before you can start recording, Booya needs a few permissions from your system.'}</p>
          <div class="perm-rows">${rows(x)}</div>
          ${note(x)}
          <div class="perm-foot"><span class="perm-private">Booya never sends your recordings anywhere.</span>
            <button class="btn primary perm-go" id="perm-go" ${ok ? '' : 'disabled'}>Continue</button></div>
          ${ok ? '' : '<p class="perm-fine">Continue unlocks once Screen Recording is on.</p>'}</div>`;
        bind(el, tick);
        el.querySelector('#perm-go').onclick = () => { stop = true; localStorage.setItem('perm-seen', '1'); remember(x); el.remove(); res(); };
      };
      const tick = async () => { if (stop) return; const x = await invoke('permissions_list').catch(() => null); if (x && !stop) paint(x); };
      paint(d);
      const t = setInterval(() => (stop ? clearInterval(t) : tick()), 1000);
      el.querySelector('#perm-go')?.focus();
    });
  }

  // On start: the first time on a Mac, or when something that was on is now off.
  async function maybeShow() {
    const d = await invoke('permissions_list').catch(() => null);
    if (!d) return;
    const lost = regressed(d);
    const first = d.platform === 'macos' && !localStorage.getItem('perm-seen');
    if (!(d.needs_attention || first || lost.length)) { remember(d); return; }
    const why = lost.length && !first ? `${lost.map(i => LABEL[i.id] || i.title).join(' and ')} ${lost.length > 1 ? 'were' : 'was'} switched off. This can happen after an update. Switch ${lost.length > 1 ? 'them' : 'it'} back on to keep recording.` : '';
    await overlay(d, why);
  }

  root.Permissions = { mount, maybeShow, overlay, rows };
})(window);
