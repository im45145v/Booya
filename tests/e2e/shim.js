// A stand in for the Tauri shell, so the real editor UI can be driven in a
// browser. Every backend command goes to the test harness (window.__py).
(() => {
  const listeners = {};
  window.__calls = [];
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        window.__calls.push(cmd);
        const raw = await window.__py(cmd, JSON.stringify(args || {}));
        const v = raw === undefined || raw === null ? null : JSON.parse(raw);
        if (v && v.__error) throw v.__error;
        if (v && v.__bytes) return (await fetch(v.__bytes)).arrayBuffer();
        return v;
      },
      convertFileSrc: p => window.__ASSET_BASE + '/asset?path=' + encodeURIComponent(p),
    },
    event: {
      listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => {}; },
    },
  };
  window.__emit = (name, payload) => (listeners[name] || []).forEach(cb => cb({ payload }));
})();
