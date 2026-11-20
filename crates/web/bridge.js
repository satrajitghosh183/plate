// Small browser helpers the Rust app calls. Kept tiny on purpose.

function dec(s) {
  const b = atob(s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "="));
  const out = new Uint8Array(b.length);
  for (let i = 0; i < b.length; i++) out[i] = b.charCodeAt(i);
  return out.buffer;
}

function enc(buf) {
  if (!buf) return null;
  const bytes = new Uint8Array(buf);
  let s = "";
  for (const x of bytes) s += String.fromCharCode(x);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function passkeysSupported() {
  return !!(window.PublicKeyCredential && navigator.credentials);
}

// Takes the server's options JSON, returns the credential JSON the server expects.
export async function passkeyCreate(optionsJson) {
  const o = JSON.parse(optionsJson).publicKey;
  o.challenge = dec(o.challenge);
  o.user.id = dec(o.user.id);
  (o.excludeCredentials || []).forEach((c) => { c.id = dec(c.id); });
  const cred = await navigator.credentials.create({ publicKey: o });
  return JSON.stringify({
    id: cred.id,
    rawId: enc(cred.rawId),
    type: cred.type,
    extensions: (cred.getClientExtensionResults && cred.getClientExtensionResults()) || {},
    response: {
      attestationObject: enc(cred.response.attestationObject),
      clientDataJSON: enc(cred.response.clientDataJSON),
      transports: (cred.response.getTransports && cred.response.getTransports()) || [],
    },
  });
}

export async function passkeyGet(optionsJson) {
  const o = JSON.parse(optionsJson).publicKey;
  o.challenge = dec(o.challenge);
  (o.allowCredentials || []).forEach((c) => { c.id = dec(c.id); });
  const cred = await navigator.credentials.get({ publicKey: o });
  return JSON.stringify({
    id: cred.id,
    rawId: enc(cred.rawId),
    type: cred.type,
    extensions: (cred.getClientExtensionResults && cred.getClientExtensionResults()) || {},
    response: {
      authenticatorData: enc(cred.response.authenticatorData),
      clientDataJSON: enc(cred.response.clientDataJSON),
      signature: enc(cred.response.signature),
      userHandle: enc(cred.response.userHandle),
    },
  });
}

// Keep the screen on during a session (iOS 18.4+ in an installed app).
let lock = null;
export async function keepAwake(on) {
  try {
    if (on && !lock && navigator.wakeLock) {
      lock = await navigator.wakeLock.request("screen");
      lock.addEventListener("release", () => { lock = null; });
    } else if (!on && lock) {
      await lock.release();
      lock = null;
    }
  } catch (_) { lock = null; }
}
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible" && window.__gymAwake) keepAwake(true);
});
export function setAwakeWanted(on) { window.__gymAwake = !!on; keepAwake(!!on); }

export function registerWorker() {
  if ("serviceWorker" in navigator && location.protocol !== "file:") {
    navigator.serviceWorker.register("/sw.js").catch(() => {});
  }
}

export function reducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

// FLIP: remember where tagged elements are, then animate them from there after a change.
const flip = new Map();
export function flipFirst(selector) {
  flip.clear();
  document.querySelectorAll(selector).forEach((el) => flip.set(el.dataset.flip, el.getBoundingClientRect()));
}
export function flipPlay(selector, duration, stagger) {
  if (reducedMotion()) return;
  let i = 0;
  document.querySelectorAll(selector).forEach((el) => {
    const a = flip.get(el.dataset.flip);
    if (!a) return;
    const b = el.getBoundingClientRect();
    const dx = a.left - b.left, dy = a.top - b.top;
    if (dx || dy) {
      el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "translate(0, 0)" }],
        { duration, delay: i * stagger, easing: "cubic-bezier(.3,1,.3,1)", fill: "backwards" });
    }
    i++;
  });
}

export async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch (_) { return false; }
}

// A short, soft tone when rest ends. Audio needs one tap first on iOS.
let audio = null;
export function unlockAudio() {
  try {
    if (!audio) audio = new (window.AudioContext || window.webkitAudioContext)();
    if (audio.state === "suspended") audio.resume();
  } catch (_) {}
}
export function chime() {
  try {
    if (!audio) return;
    const t = audio.currentTime;
    [880, 1318.5].forEach((f, i) => {
      const o = audio.createOscillator(), g = audio.createGain();
      o.type = "sine"; o.frequency.value = f;
      g.gain.setValueAtTime(0.0001, t + i * 0.12);
      g.gain.exponentialRampToValueAtTime(0.18, t + i * 0.12 + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, t + i * 0.12 + 0.6);
      o.connect(g).connect(audio.destination);
      o.start(t + i * 0.12); o.stop(t + i * 0.12 + 0.65);
    });
  } catch (_) {}
}

export function timezoneOffset() { return new Date().getTimezoneOffset(); }
export function isStandalone() { return window.matchMedia("(display-mode: standalone)").matches || window.navigator.standalone === true; }
export function hideBoot() { const b = document.getElementById("boot"); if (b) b.remove(); }
export function openExternal(url) { window.location.href = url; }
export function downloadUrl(url) { const a = document.createElement("a"); a.href = url; a.download = ""; document.body.appendChild(a); a.click(); a.remove(); }
