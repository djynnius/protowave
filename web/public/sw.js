// ProtoWave service worker: offline shell (PRD §10).
//
// Strategy: hashed /assets/ are immutable → cache-first; navigations and
// index.html → network-first with cached fallback (so deploys land on the
// next online load, and the app still opens offline); API and WebSocket
// traffic passes through untouched — document state offline is the CRDT's
// job (yjs), not the HTTP cache's.

const CACHE = 'protowave-shell-v1'

self.addEventListener('install', (event) => {
  event.waitUntil(self.skipWaiting())
})

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim())
})

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url)
  if (event.request.method !== 'GET' || url.origin !== location.origin) return
  if (url.pathname.startsWith('/api/') || url.pathname === '/ws') return

  // Immutable hashed assets: cache-first.
  if (url.pathname.startsWith('/assets/') || url.pathname.endsWith('.svg')) {
    event.respondWith(
      caches.open(CACHE).then(async (cache) => {
        const hit = await cache.match(event.request)
        if (hit) return hit
        const res = await fetch(event.request)
        if (res.ok) cache.put(event.request, res.clone())
        return res
      }),
    )
    return
  }

  // Navigations / index: network-first, cached shell as offline fallback.
  if (event.request.mode === 'navigate' || url.pathname === '/index.html') {
    event.respondWith(
      fetch(event.request)
        .then((res) => {
          if (res.ok) {
            const copy = res.clone()
            caches.open(CACHE).then((cache) => cache.put('/index.html', copy))
          }
          return res
        })
        .catch(() => caches.match('/index.html')),
    )
  }
})
