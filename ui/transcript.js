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
  $('tr-hint').textContent = caps.length ? 'Click a word to fix it (Enter saves, Tab moves on). Split, merge and nudge the timing under each line.' : 'No transcript yet. Use Transcribe in the Auto edit tab.';
  box.innerHTML = '';
  const redo = () => { renderTranscript(); refresh(); };
  caps.forEach((c, i) => {
    CaptionEdit.ensureWords(c);
    const row = document.createElement('div'); row.className = 'tr-row';
    const t = document.createElement('button'); t.className = 'tr-time'; t.textContent = fmt(c.start_ms); t.title = 'Jump here';
    t.onclick = () => { tl.playhead = tl.srcToOut(c.start_ms + 1); tl.draw(); refresh(); };
    const words = document.createElement('div'); words.className = 'tr-words'; words.setAttribute('role', 'group'); words.setAttribute('aria-label', 'Caption ' + (i + 1));
    let picked = -1; // the word split will start a new line at
    const edit = (k) => {
      const w = c.words[k]; if (!w) return;
      const chip = words.children[k];
      const inp = document.createElement('input'); inp.className = 'field tr-word-edit'; inp.value = w.text; inp.size = Math.max(3, w.text.length + 1); inp.spellcheck = true;
      inp.setAttribute('aria-label', 'Word ' + (k + 1));
      let done = false;
      const commit = (next) => {
        if (done) return; done = true;
        if (inp.value !== w.text) CaptionEdit.setWord(c, k, inp.value);
        picked = k;
        redo();
        if (next != null) { const n = $('tr-list').querySelectorAll('.tr-row')[i]; const chips = n && n.querySelector('.tr-words'); if (chips && chips.children[next]) chips.children[next].click(); }
      };
      inp.onkeydown = e => {
        e.stopPropagation();
        if (e.key === 'Enter') { e.preventDefault(); commit(); }
        else if (e.key === 'Escape') { e.preventDefault(); done = true; renderTranscript(); }
        else if (e.key === 'Tab') { e.preventDefault(); commit(e.shiftKey ? k - 1 : k + 1); }
      };
      inp.onblur = () => commit();
      chip.replaceWith(inp); inp.focus(); inp.select();
    };
    c.words.forEach((w, k) => {
      const chip = document.createElement('button'); chip.className = 'tr-word' + (k === picked ? ' picked' : ''); chip.textContent = w.text; chip.title = 'Click to fix this word';
      chip.onclick = () => { picked = k; tl.playhead = tl.srcToOut(w.start_ms + 1); tl.draw(); edit(k); };
      words.appendChild(chip);
    });
    const tools = document.createElement('div'); tools.className = 'tr-tools';
    const btn = (label, title, fn) => { const b = document.createElement('button'); b.className = 'btn ghost sm'; b.textContent = label; b.title = title; b.onclick = fn; tools.appendChild(b); return b; };
    btn('Split', 'Start a new line at the word you last clicked', () => {
      const k = picked > 0 ? picked : Math.floor(c.words.length / 2);
      const next = CaptionEdit.splitAt(project.captions, i, k);
      if (next === project.captions) return status('Click the word the new line should start with, then Split.', 'info');
      project.captions = next; redo();
    });
    btn('Merge next', 'Join this line with the next one', () => { project.captions = CaptionEdit.mergeNext(project.captions, i); redo(); });
    btn('Start -0.1s', 'Show this line a little earlier', () => { CaptionEdit.nudge(project.captions, i, 'start', -100); redo(); });
    btn('Start +0.1s', 'Show this line a little later', () => { CaptionEdit.nudge(project.captions, i, 'start', 100); redo(); });
    btn('End -0.1s', 'Hide this line a little earlier', () => { CaptionEdit.nudge(project.captions, i, 'end', -100); redo(); });
    btn('End +0.1s', 'Keep this line a little longer', () => { CaptionEdit.nudge(project.captions, i, 'end', 100); redo(); });
    btn('Move -0.1s', 'Shift the whole line and its words earlier', () => { CaptionEdit.nudge(project.captions, i, 'both', -100); redo(); });
    btn('Move +0.1s', 'Shift the whole line and its words later', () => { CaptionEdit.nudge(project.captions, i, 'both', 100); redo(); });
    const cut = btn('Cut line', 'Remove this part from the video', () => {
      const clips = cutSourceRange(project.clips, c.start_ms, c.end_ms);
      if (!clips.length) return status('A project needs at least one clip.', 'error');
      project.clips = clips; project.captions.splice(i, 1); tl.selectedClip = 0; tl.playhead = Math.min(tl.playhead, tl.outDur());
      tl.draw(); syncSelection(); renderTranscript(); refresh(); status('Line cut from the video.', 'success');
    });
    cut.classList.add('danger');
    const main = document.createElement('div'); main.append(words, tools);
    row.append(t, main); box.appendChild(row);
  });
}
$('tr-replace').onclick = () => {
  const f = $('tr-find').value; if (!project || !f) return status('Type the word to find first.', 'error');
  const re = new RegExp(f.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'); let n = 0;
  (project.captions || []).forEach(c => { const m = c.text.match(re); if (m) { n += m.length; rewordCaption(c, c.text.replace(re, $('tr-repl').value)); } });
  renderTranscript(); refresh(); status(n ? `Replaced ${n} ${n === 1 ? 'match' : 'matches'}.` : 'No matches.', n ? 'success' : 'info');
};
(function () { const sp = syncPanel; syncPanel = function () { sp(); renderTranscript(); }; })();
