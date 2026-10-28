// Run with: node tests/js/shortcuts.test.js
const assert = require('assert');
const S = require('../../ui/shortcuts.js');
const ev = (code, o = {}) => ({ code, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...o });
let n = 0;
const t = (name, f) => { f(); n++; console.log('ok', name); };

t('a lone modifier is not a combo yet', () => {
  for (const c of ['ShiftLeft', 'ControlRight', 'AltLeft', 'MetaLeft', 'CapsLock']) assert.strictEqual(S.comboFromEvent(ev(c, { shiftKey: true }), true), null);
});
t('on a Mac Cmd is the main modifier and Ctrl stays Ctrl', () => {
  assert.strictEqual(S.comboFromEvent(ev('KeyR', { metaKey: true, shiftKey: true }), true), 'CmdOrCtrl+Shift+KeyR');
  assert.strictEqual(S.comboFromEvent(ev('Digit1', { metaKey: true, altKey: true }), true), 'CmdOrCtrl+Alt+Digit1');
  assert.strictEqual(S.comboFromEvent(ev('KeyK', { ctrlKey: true, metaKey: true }), true), 'CmdOrCtrl+Ctrl+KeyK');
});
t('elsewhere Ctrl is the main modifier and the Windows key is Super', () => {
  assert.strictEqual(S.comboFromEvent(ev('KeyR', { ctrlKey: true, shiftKey: true }), false), 'CmdOrCtrl+Shift+KeyR');
  assert.strictEqual(S.comboFromEvent(ev('KeyM', { metaKey: true, altKey: true }), false), 'Super+Alt+KeyM');
});
t('modifier order matches what the Rust side writes', () => {
  assert.strictEqual(S.comboFromEvent(ev('KeyA', { metaKey: true, ctrlKey: true, altKey: true, shiftKey: true }), true), 'CmdOrCtrl+Ctrl+Alt+Shift+KeyA');
  assert.strictEqual(S.comboFromEvent(ev('KeyA', { ctrlKey: true, metaKey: true, altKey: true, shiftKey: true }), false), 'CmdOrCtrl+Super+Alt+Shift+KeyA');
});
t('function keys work without a modifier', () => {
  assert.strictEqual(S.comboFromEvent(ev('F9'), false), 'F9');
});
t('the physical key is used, so the layout does not matter', () => {
  assert.strictEqual(S.comboFromEvent({ ...ev('KeyQ', { ctrlKey: true }), key: 'a' }, false), 'CmdOrCtrl+KeyQ');
});
t('parts splits a label into key caps', () => {
  assert.deepStrictEqual(S.parts('Shift+Cmd+R'), ['Shift', 'Cmd', 'R']);
  assert.deepStrictEqual(S.parts(''), []);
});
console.log(n + ' passed');
