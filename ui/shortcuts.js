// Pure helpers for the shortcut recorder (Settings > Shortcuts). The Rust side
// (booya_core::shortcuts) is the authority on what a binding may be; this
// only turns a key press into the combo text it understands, so it can be
// tested without a browser.
const Shortcuts = (() => {
  // Pressing only a modifier is not a combo yet: wait for the real key.
  const MODIFIER_CODES = new Set(['ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight', 'OSLeft', 'OSRight', 'CapsLock', 'Fn', 'AltGraph']);

  // e: anything with code, ctrlKey, metaKey, altKey, shiftKey (a KeyboardEvent).
  // mac: Cmd (meta) is the main modifier; elsewhere it is Ctrl.
  // Returns "CmdOrCtrl+Shift+KeyR" style text, or null while only modifiers are down.
  function comboFromEvent(e, mac) {
    if (!e || !e.code || MODIFIER_CODES.has(e.code)) return null;
    const mods = [];
    if (mac) {
      if (e.metaKey) mods.push('CmdOrCtrl');
      if (e.ctrlKey) mods.push('Ctrl');
    } else {
      if (e.ctrlKey) mods.push('CmdOrCtrl');
      if (e.metaKey) mods.push('Super');
    }
    if (e.altKey) mods.push('Alt');
    if (e.shiftKey) mods.push('Shift');
    return [...mods, e.code].join('+');
  }

  // "Shift+Cmd+R" -> ["Shift", "Cmd", "R"] for drawing one key cap per part.
  function parts(text) { return text ? text.split('+').filter(Boolean) : []; }

  return { comboFromEvent, parts, MODIFIER_CODES };
})();
if (typeof module !== 'undefined') module.exports = Shortcuts;
