// VisualSource offline cache: keeps this page available with no network.
// The page itself is fully self-contained (engine included), so caching one
// document is all that is needed.
// NETWORK-FIRST: when online, the latest deployed page is always fetched and
// the cache is refreshed, so every site redeploy shows up on the next visit.
// The cache is only used as the offline fallback.
const CACHE = 'vs4-offline-v12';
self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(['./'])));
  self.skipWaiting();
});
self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((keys) =>
    Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))));
  self.clients.claim();
});
self.addEventListener('fetch', (e) => {
  e.respondWith(
    fetch(e.request).then((res) => {
      const copy = res.clone();
      caches.open(CACHE).then((c) => c.put(e.request, copy));
      return res;
    }).catch(() => caches.match(e.request).then((hit) => hit || caches.match('./')))
  );
});
