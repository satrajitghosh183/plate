// Offline shell for the gym. The app's data lives on the phone (localStorage) and
// syncs when the server answers; this worker only keeps the app itself loadable.
const VERSION = "__VERSION__";
const SHELL = `shell-${VERSION}`;
const FONTS = "fonts-v1";
const FILES = [
  "/",
  "/index.html",
  `/app.css?v=${VERSION}`,
  `/pkg/gym_web.js?v=${VERSION}`,
  `/pkg/gym_web_bg.wasm?v=${VERSION}`,
  "/body.jpg",
  "/body-pull.jpg",
  "/body-legs.jpg",
  "/manifest.webmanifest",
  "/icons/icon-192.png",
  "/icons/apple-touch-icon.png",
  "/favicon.svg",
  "/favicon.ico",
  ...__SNIPPETS__,
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(SHELL).then((c) => c.addAll(FILES)).then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== SHELL && k !== FONTS).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

function timeout(ms) {
  return new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), ms));
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);

  // Never cache data or anything with a key in it.
  if (url.origin === self.location.origin && /^\/(api|mcp|ingest|healthz)/.test(url.pathname)) return;

  if (url.hostname === "fonts.googleapis.com" || url.hostname === "fonts.gstatic.com") {
    event.respondWith(
      caches.open(FONTS).then(async (c) => {
        const hit = await c.match(req);
        const net = fetch(req).then((res) => { if (res.ok || res.type === "opaque") c.put(req, res.clone()); return res; }).catch(() => hit);
        return hit || net;
      })
    );
    return;
  }

  if (url.origin !== self.location.origin) return;

  // Pages: try the network briefly so updates arrive, fall back to the cached app.
  if (req.mode === "navigate") {
    event.respondWith(
      Promise.race([fetch(req), timeout(3500)])
        .then((res) => { if (res.ok) caches.open(SHELL).then((c) => c.put("/index.html", res.clone())); return res; })
        .catch(() => caches.match("/index.html").then((r) => r || caches.match("/")))
    );
    return;
  }

  // Versioned files: cache first.
  event.respondWith(
    caches.match(req).then((hit) => hit || fetch(req).then((res) => {
      if (res.ok) { const copy = res.clone(); caches.open(SHELL).then((c) => c.put(req, copy)); }
      return res;
    }))
  );
});
