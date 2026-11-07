// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
// @ts-ignore plain JS module
import { en, zh, pickLang, setLang, t, fmtMoney, fmtTime } from '../../phone/i18n.js';

const dir = join(__dirname, '..', '..', 'phone');
const read = (f: string) => readFileSync(join(dir, f), 'utf8');

describe('i18n', () => {
  it('both tables have exactly the same keys', () => {
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
  });
  it('both tables keep the same {placeholders}', () => {
    const ph = (s: string) => (s.match(/\{\w+\}/g) || []).sort().join(',');
    for (const k of Object.keys(en)) expect(ph(zh[k]), k).toBe(ph(en[k]));
  });
  it('no empty strings', () => {
    for (const k of Object.keys(en)) { expect(en[k].length).toBeGreaterThan(0); expect(zh[k].length).toBeGreaterThan(0); }
  });
  it('every zh-* locale picks Traditional Chinese, the rest English', () => {
    for (const l of ['zh-TW', 'zh-Hant', 'zh-HK', 'zh-CN', 'zh', 'ZH-tw', 'zh_TW']) expect(pickLang(l), l).toBe('zh');
    for (const l of ['en-US', 'ja', 'de-DE', '', undefined, 'zhx']) expect(pickLang(l), String(l)).toBe('en');
  });
  it('t() substitutes params and falls back to the key', () => {
    setLang('en');
    expect(t('pc.status', { name: 'Home' })).toBe('PC: Home');
    expect(t('no.such.key')).toBe('no.such.key');
    setLang('zh');
    expect(t('pc.status', { name: 'Home' })).toBe('電腦：Home');
    setLang('en');
  });
  it('formats money and time', () => {
    setLang('en');
    expect(fmtMoney(1_230_000_000)).toBe('$1.23');
    expect(fmtMoney(1000)).toBe('<$0.01');
    const n = 1_700_000_000_000;
    expect(fmtTime(n - 5000, n)).toBe('now');
    expect(fmtTime(n - 5 * 60000, n)).toBe('5m');
    expect(fmtTime(n - 3 * 3600000, n)).toBe('3h');
  });
});

describe('manifest, shell and icons', () => {
  it('manifest.webmanifest is valid JSON with the PWA basics', () => {
    const m = JSON.parse(read('manifest.webmanifest'));
    expect(m.name).toBeTruthy();
    expect(m.start_url).toBe('.');
    expect(m.display).toBe('standalone');
    expect(m.icons.length).toBeGreaterThanOrEqual(2);
    for (const i of m.icons) expect(existsSync(join(dir, i.src)), i.src).toBe(true);
  });
  it('icons are real PNGs of the right size', () => {
    for (const [f, size] of [['icon-192.png', 192], ['icon-512.png', 512], ['apple-touch-icon.png', 180]] as const) {
      const b = readFileSync(join(dir, f));
      expect(b.subarray(1, 4).toString()).toBe('PNG');
      expect(b.readUInt32BE(16)).toBe(size);
      expect(b.readUInt32BE(20)).toBe(size);
    }
  });
  it('index.html is phone ready', () => {
    const html = read('index.html');
    expect(html).toContain('viewport-fit=cover');
    expect(html).toContain('rel="manifest"');
    expect(html).toContain('apple-touch-icon');
    expect(html).toContain('type="module" src="app.js"');
  });
  it('style.css uses safe-area insets, dark mode and no colours', () => {
    const css = read('style.css');
    expect(css).toContain('env(safe-area-inset-bottom');
    expect(css).toContain('prefers-color-scheme: dark');
    // only grayscale hex colours: r=g=b
    for (const m of css.matchAll(/#([0-9a-fA-F]{6})\b/g)) {
      const [r, g, b] = [m[1].slice(0, 2), m[1].slice(2, 4), m[1].slice(4, 6)];
      expect(r === g && g === b, '#' + m[1]).toBe(true);
    }
    expect(css).not.toMatch(/hsl\(|rgb\(\s*\d+\s*,\s*\d+\s*,\s*\d+/);
  });
  it('app source never uses innerHTML / outerHTML / insertAdjacentHTML / document.write', () => {
    for (const f of ['app.js', 'net.js', 'i18n.js']) {
      expect(read(f), f).not.toMatch(/innerHTML|outerHTML|insertAdjacentHTML|document\.write|eval\(/);
    }
  });
  it('app source has no password or username inputs', () => {
    const src = read('app.js');
    expect(src).not.toMatch(/type:\s*'password'|type="password"|autocomplete:\s*'(username|current-password)/);
  });
});

describe('service worker', () => {
  function load() {
    const listeners: Record<string, (e: any) => void> = {};
    const cachePut = vi.fn(async () => {});
    const caches = {
      open: vi.fn(async () => ({ addAll: async () => {}, put: cachePut })),
      match: vi.fn(async () => undefined),
      keys: async () => [],
      delete: async () => true
    };
    const self: any = {
      addEventListener: (n: string, f: any) => { listeners[n] = f; },
      location: { origin: 'http://pc.test' },
      skipWaiting: () => {},
      clients: { claim: async () => {}, matchAll: vi.fn(async () => []), openWindow: vi.fn(async () => {}) },
      registration: { showNotification: vi.fn(async () => {}) }
    };
    new Function('self', 'caches', 'fetch', read('sw.js'))(self, caches, vi.fn(async () => ({ ok: true, clone: () => ({}) })));
    return { listeners, caches, cachePut, self };
  }
  it('never answers or caches /api/* requests', () => {
    const { listeners, cachePut } = load();
    for (const p of ['/api/me', '/api/bots/b1', '/x/api/changes?since=1']) {
      const respondWith = vi.fn();
      listeners.fetch({ request: { method: 'GET', url: 'http://pc.test' + p }, respondWith });
      expect(respondWith, p).not.toHaveBeenCalled();
    }
    expect(cachePut).not.toHaveBeenCalled();
  });
  it('serves the app shell through the cache path', () => {
    const { listeners } = load();
    const respondWith = vi.fn();
    listeners.fetch({ request: { method: 'GET', url: 'http://pc.test/app.js' }, respondWith });
    expect(respondWith).toHaveBeenCalledTimes(1);
  });
  it('ignores non-GET and cross-origin requests', () => {
    const { listeners } = load();
    const a = vi.fn(); const b = vi.fn();
    listeners.fetch({ request: { method: 'POST', url: 'http://pc.test/app.js' }, respondWith: a });
    listeners.fetch({ request: { method: 'GET', url: 'http://other.test/app.js' }, respondWith: b });
    expect(a).not.toHaveBeenCalled();
    expect(b).not.toHaveBeenCalled();
  });
  it('shows a notification on push and focuses the app on click', async () => {
    const { listeners, self } = load();
    const waits: Promise<any>[] = [];
    listeners.push({ data: { json: () => ({ title: 'Scout', body: 'Needs you', botId: 'b1' }) }, waitUntil: (p: any) => waits.push(p) });
    await Promise.all(waits);
    expect(self.registration.showNotification).toHaveBeenCalledWith('Scout', expect.objectContaining({ body: 'Needs you', data: { botId: 'b1' } }));
    const close = vi.fn();
    const w2: Promise<any>[] = [];
    listeners.notificationclick({ notification: { close, data: { botId: 'b1' } }, waitUntil: (p: any) => w2.push(p) });
    await Promise.all(w2);
    expect(close).toHaveBeenCalled();
    expect(self.clients.openWindow).toHaveBeenCalledWith('./#/bot/b1');
  });
  it('survives a push with no data', async () => {
    const { listeners, self } = load();
    const waits: Promise<any>[] = [];
    listeners.push({ data: null, waitUntil: (p: any) => waits.push(p) });
    await Promise.all(waits);
    expect(self.registration.showNotification).toHaveBeenCalledWith('Potato Bot', expect.anything());
  });
});
