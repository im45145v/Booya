// Transcript panel: edit caption text, find and replace, cut a line from the
// video. Loaded after main.js and shares its globals.

// Words keep their timing when the count is unchanged, otherwise the new
// words share the line's time in proportion to their length.
function rewordCaption(c, text) {
  c.text = text.trim();
  const toks = c.text.split(/\s+/).filter(Boolean);
  if (!c.words || !c.words.length) return;
  if (toks.length === c.words.length) { toks.forEach((w, i) => (c.words[i].text = w)); return; }
  if (!toks.length) { c.words = []; return; }
  const total = toks.reduce((a, w) => a + w.length + 1, 0);
  let at = c.start_ms;
  c.words = toks.map(w => { const len = (c.end_ms - c.start_ms) * ((w.length + 1) / total); const o = { start_ms: at, end_ms: at + len, text: w }; at += len; return o; });
}

// Remove [a, b] (source ms) from the clips, keeping each clip's speed.
function cutSourceRange(clips, a, b) {
  const out = [];
  for (const c of clips) {
    if (b <= c.src_start_ms || a >= c.src_end_ms) { out.push({ ...c }); continue; }
    if (a - c.src_start_ms > 50) out.push({ ...c, src_end_ms: a });
    if (c.src_end_ms - b > 50) out.push({ ...c, src_start_ms: b });
  }
  return out;
}

function renderTranscript() {
  const box = $('tr-list'); if (!box || !project) return;
  const caps = project.captions || [];
  $('tr-hint').textContent = caps.length ? 'Fix a word by editing it. Cut line removes that part of the video.' : 'No transcript yet. Use Transcribe in the Magic tab.';
  box.innerHTML = '';
  caps.forEach((c, i) => {
    const row = document.createElement('div'); row.className = 'tr-row';
    const t = document.createElement('button'); t.className = 'tr-time'; t.textContent = fmt(c.start_ms); t.title = 'Jump here';
    t.onclick = () => { tl.playhead = tl.srcToOut(c.start_ms + 1); tl.draw(); refresh(); };
    const inp = document.createElement('textarea'); inp.className = 'field tr-text'; inp.rows = Math.max(2, Math.ceil(c.text.length / 30)); inp.value = c.text; inp.spellcheck = true; inp.setAttribute('aria-label', 'Caption ' + (i + 1));
    inp.onchange = () => { rewordCaption(c, inp.value); refresh(); };
    inp.onkeydown = e => { if (e.key === 'Enter') { e.preventDefault(); inp.blur(); } e.stopPropagation(); };
    const cut = document.createElement('button'); cut.className = 'btn ghost sm'; cut.textContent = 'Cut line'; cut.title = 'Remove this part from the video';
    cut.onclick = () => {
      const clips = cutSourceRange(project.clips, c.start_ms, c.end_ms);
      if (!clips.length) return status('A project needs at least one clip.', 'error');
      project.clips = clips; project.captions.splice(i, 1); tl.selectedClip = 0; tl.playhead = Math.min(tl.playhead, tl.outDur());
      tl.draw(); syncSelection(); renderTranscript(); refresh(); status('Line cut from the video.', 'success');
    };
    row.append(t, inp, cut); box.appendChild(row);
  });
}
$('tr-replace').onclick = () => {
  const f = $('tr-find').value; if (!project || !f) return status('Type the word to find first.', 'error');
  const re = new RegExp(f.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'); let n = 0;
  (project.captions || []).forEach(c => { const m = c.text.match(re); if (m) { n += m.length; rewordCaption(c, c.text.replace(re, $('tr-repl').value)); } });
  renderTranscript(); refresh(); status(n ? `Replaced ${n} ${n === 1 ? 'match' : 'matches'}.` : 'No matches.', n ? 'success' : 'info');
};
(function () { const sp = syncPanel; syncPanel = function () { sp(); renderTranscript(); }; })();
