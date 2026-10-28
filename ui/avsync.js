// Keeps the editor preview's picture and sound together.
//
// The sound is a normal audio element playing the proxy. The picture is drawn
// by the backend, one frame per request, so every frame arrives a little
// after the moment it was asked for. Without a correction the picture is
// that much behind the sound (and more when the machine is busy). These
// pure functions hold the three rules the preview follows:
//   1. ask for the frame the clock will reach when the frame is on screen
//      (the playhead plus the measured round trip),
//   2. keep the sound on the clock: nudge its speed for small gaps, jump for
//      large ones,
//   3. start the clock only once the sound is really playing.
(function (root) {
  'use strict';

  const MAX_LEAD_MS = 400;

  // Smoothed round trip of a frame request, ms.
  function nextLatency(prev, sample) {
    if (!(sample >= 0) || !isFinite(sample)) return prev;
    const s = Math.min(sample, 1500);
    return prev == null ? s : prev * 0.7 + s * 0.3;
  }

  // How far ahead of the playhead the next frame is requested, ms.
  function frameLead(latencyMs) {
    if (latencyMs == null || !(latencyMs > 0)) return 0;
    return Math.min(MAX_LEAD_MS, latencyMs);
  }

  // The output time to request a frame for.
  function frameTime(playheadMs, latencyMs, durMs, playing) {
    const t = playing ? playheadMs + frameLead(latencyMs) : playheadMs;
    return Math.max(0, Math.min(durMs, t));
  }

  // What to do with the audio element, given where it should be (want) and
  // where it is (have), both in seconds, and the clip's speed.
  //   gap > 250 ms: jump. gap > 25 ms: change speed a little for a moment
  //   (up to 6%, unheard with pitch preserved). Otherwise play straight.
  function audioCorrection(wantSec, haveSec, speed) {
    const gap = wantSec - haveSec; // positive: the sound is behind
    const abs = Math.abs(gap);
    if (abs > 0.25) return { seek: true, rate: speed };
    if (abs > 0.025) {
      const nudge = Math.max(-0.06, Math.min(0.06, gap * 0.5));
      return { seek: false, rate: speed * (1 + nudge) };
    }
    return { seek: false, rate: speed };
  }

  // The natively played sound (see playback.rs) has no speed nudging: it is
  // put back on the clock with a seek once it is more than 60 ms away.
  const NATIVE_TOLERANCE_S = 0.06;
  function nativeCorrection(wantSec, haveSec) {
    return Math.abs(wantSec - haveSec) > NATIVE_TOLERANCE_S;
  }

  const api = { nextLatency, frameLead, frameTime, audioCorrection, nativeCorrection, NATIVE_TOLERANCE_S, MAX_LEAD_MS };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.AvSync = api;
})(typeof window !== 'undefined' ? window : globalThis);
