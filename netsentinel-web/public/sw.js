// Bump the suffix on every deploy that changes static assets. The
// activate handler below drops every non-matching cache, so users pick
// up the new bundle on next nav. Version ladder: netmonitor-v1 (pre-rename)
// → netsentinel-v1 (post-rename, stale JS/CSS hazard fix) → future bumps.
const CACHE_NAME = "netsentinel-v1";

self.addEventListener("install", () => {
  // Skip pre-caching — Cloudflare Access may intercept and redirect
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE_NAME).map((k) => caches.delete(k)))
    )
  );
  self.clients.claim();
});

self.addEventListener("fetch", (event) => {
  const { request } = event;

  // Only handle same-origin GET requests for pages/assets
  if (
    request.method !== "GET" ||
    request.url.includes("/api/") ||
    !request.url.startsWith(self.location.origin)
  ) {
    return;
  }

  // Network-first strategy: try network, fall back to cache
  event.respondWith(
    fetch(request)
      .then((response) => {
        // Only cache successful same-origin responses
        if (response.ok && response.type === "basic") {
          const clone = response.clone();
          caches.open(CACHE_NAME).then((cache) => cache.put(request, clone));
        }
        return response;
      })
      .catch(() =>
        caches.match(request).then((cached) => {
          // Return cached response, or a minimal offline fallback
          return cached || new Response("Offline", { status: 503, statusText: "Service Unavailable" });
        })
      )
  );
});
