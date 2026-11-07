/* Potato Bot service worker.
   - Caches the app shell only (network first, so updates land fast).
   - NEVER touches /api/*: those requests go straight to the network, never cached.
   - Handles web push and notification clicks. */
const CACHE = 'potato-phone-shell-v1';
const SHELL = ['./', 'index.html', 'app.js', 'net.js', 'i18n.js', 'style.css', 'manifest.webmanifest', 'icon.svg', 'icon-192.png', 'apple-touch-icon.png'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE).then((c) => c.addAll(SHELL)).catch(() => {}).then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname.indexOf('/api/') !== -1) return; // never cache the API
  event.respondWith(
    fetch(req)
      .then((res) => {
        if (res && res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
        }
        return res;
      })
      .catch(() => caches.match(req).then((hit) => hit || caches.match('index.html')))
  );
});

self.addEventListener('push', (event) => {
  let data = {};
  try { data = event.data ? event.data.json() : {}; } catch (e) { data = {}; }
  const title = typeof data.title === 'string' && data.title ? data.title : 'Potato Bot';
  const body = typeof data.body === 'string' ? data.body : '';
  const options = {
    body,
    icon: 'icon-192.png',
    badge: 'icon-192.png',
    tag: typeof data.tag === 'string' ? data.tag : (data.botId ? 'bot-' + data.botId : 'potato'),
    data: { botId: data.botId || null }
  };
  event.waitUntil(self.registration.showNotification(title, options));
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const botId = event.notification.data && event.notification.data.botId;
  const target = botId ? './#/bot/' + encodeURIComponent(botId) : './';
  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((list) => {
      for (const c of list) {
        if ('focus' in c) {
          if (botId && 'navigate' in c) { try { c.navigate(target); } catch (e) { /* ignore */ } }
          return c.focus();
        }
      }
      return self.clients.openWindow ? self.clients.openWindow(target) : undefined;
    })
  );
});
