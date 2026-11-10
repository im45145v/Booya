// Booya site: pick the visitor's OS and point the download buttons at the
// latest GitHub release. If the API is slow, rate limited or blocked, every
// link already goes to the releases page, so nothing breaks.
(function () {
  "use strict";
  var REPO = "im45145v/Booya";
  var API = "https://api.github.com/repos/" + REPO + "/releases/latest";
  var CACHE_KEY = "booya-latest-release";
  var CACHE_MS = 10 * 60 * 1000;

  var PATTERNS = {
    "mac-dmg": /\.dmg$/i,
    "win-exe": /-setup\.exe$/i,
    "win-msi": /\.msi$/i,
    "linux-appimage": /\.AppImage$/,
    "linux-deb": /\.deb$/i,
    "linux-rpm": /\.rpm$/i
  };
  var PRIMARY = { mac: "mac-dmg", win: "win-exe", linux: "linux-appimage" };
  var NAMES = { mac: "macOS", win: "Windows", linux: "Linux" };

  function detectOS() {
    var p = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "").toLowerCase();
    var ua = navigator.userAgent.toLowerCase();
    if (/android|iphone|ipad|ipod/.test(ua) || (navigator.maxTouchPoints > 1 && /mac/.test(p) && !/macintosh/.test(ua))) return null;
    if (/mac/.test(p) || /mac os x/.test(ua)) return "mac";
    if (/win/.test(p) || /windows/.test(ua)) return "win";
    if (/linux|x11|cros/.test(p) || /linux|x11/.test(ua)) return "linux";
    return null;
  }

  function mb(bytes) { return Math.round(bytes / 1048576) + " MB"; }

  function $(sel) { return document.querySelector(sel); }
  function $$(sel) { return Array.prototype.slice.call(document.querySelectorAll(sel)); }

  function apply(release, os) {
    var assets = release.assets || [];
    Object.keys(PATTERNS).forEach(function (key) {
      var asset = assets.filter(function (a) { return PATTERNS[key].test(a.name); })[0];
      if (!asset) return;
      $$('[data-asset="' + key + '"]').forEach(function (a) {
        a.href = asset.browser_download_url;
        a.setAttribute("download", "");
      });
      $$('[data-size="' + key + '"]').forEach(function (s) { s.textContent = mb(asset.size); });
      if (os && PRIMARY[os] === key) {
        var hero = $("#hero-dl");
        if (hero) { hero.href = asset.browser_download_url; hero.setAttribute("download", ""); }
      }
    });
    var v = release.tag_name || "";
    if (v) {
      var dv = $("#dl-version");
      if (dv) dv.textContent = v;
      var fine = $("#hero-fine");
      if (fine && os) fine.textContent = "Booya " + v + " for " + NAMES[os] + ", " + mb(sizeOf(assets, PRIMARY[os])) + ". Free and MIT licensed.";
    }
  }

  function sizeOf(assets, key) {
    var a = assets.filter(function (x) { return PATTERNS[key].test(x.name); })[0];
    return a ? a.size : 0;
  }

  function load() {
    try {
      var c = JSON.parse(sessionStorage.getItem(CACHE_KEY) || "null");
      if (c && Date.now() - c.t < CACHE_MS) return Promise.resolve(c.r);
    } catch (e) { /* private mode */ }
    var ctrl = "AbortController" in window ? new AbortController() : null;
    var timer = setTimeout(function () { if (ctrl) ctrl.abort(); }, 6000);
    return fetch(API, { headers: { Accept: "application/vnd.github+json" }, signal: ctrl ? ctrl.signal : undefined })
      .then(function (r) { clearTimeout(timer); if (!r.ok) throw new Error(r.status); return r.json(); })
      .then(function (r) {
        var slim = { tag_name: r.tag_name, assets: (r.assets || []).map(function (a) { return { name: a.name, size: a.size, browser_download_url: a.browser_download_url }; }) };
        try { sessionStorage.setItem(CACHE_KEY, JSON.stringify({ t: Date.now(), r: slim })); } catch (e) { /* ignore */ }
        return slim;
      });
  }

  function init() {
    var os = detectOS();
    if (os) {
      var card = $('.os[data-os="' + os + '"]');
      if (card) card.classList.add("mine");
      var label = $("#hero-dl-label");
      if (label) label.textContent = "Download for " + NAMES[os];
      var hero = $("#hero-dl");
      if (hero) hero.href = "#download";
    }
    load().then(function (r) { apply(r, os); }).catch(function () {
      var s = $("#dl-status");
      if (s) s.textContent = "Couldn't reach GitHub. The buttons open the releases page instead.";
      var hero = $("#hero-dl");
      if (hero && os) hero.href = "https://github.com/" + REPO + "/releases/latest";
    });

    var top = $("#top");
    if (top && "IntersectionObserver" in window) {
      var sentinel = document.createElement("div");
      sentinel.style.cssText = "position:absolute;top:0;height:1px;width:1px";
      document.body.prepend(sentinel);
      new IntersectionObserver(function (e) { top.classList.toggle("scrolled", !e[0].isIntersecting); }).observe(sentinel);
    }
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
