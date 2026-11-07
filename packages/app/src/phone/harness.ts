/* Test harness for the phone app: a scripted fake PC behind an injected fetch. Not a test file itself. */
// @ts-ignore plain JS module, not type-checked
import { mountPhoneApp } from '../../phone/app.js';
import { webcrypto, createHmac, createHash } from 'node:crypto';

(globalThis as any).__POTATO_PHONE_NO_AUTOMOUNT = true;

export const CODE = 'ABCD2345';
export const TOKEN = Buffer.from('0123456789abcdef0123456789abcdef').toString('base64');
export const LAN = 'http://192.168.1.20:1420';
export const LAN2 = 'http://10.0.0.5:1420';
export const TUNNEL = 'https://quiet-fox.trycloudflare.com';

export type Behavior = 'ok' | 'timeout' | 'wrong' | 'down';
export interface Call { origin: string; method: string; path: string; headers: Record<string, string>; body: any }

export function bot(over: any = {}) {
  return {
    id: 'b1', name: 'Scout', state: 'idle', stateText: 'Idle', lastLine: 'All done', lastAt: 1_700_000_000_000 - 5 * 60000,
    unread: 0, openCards: 0, typing: false, ...over
  };
}
export function chat(over: any = {}) {
  return {
    rev: 5, bot: bot(), messages: [], cards: [], job: null, work: null, typing: false, ...over
  };
}

export class FakeServer {
  behaviors: Record<string, Behavior> = {};
  calls: Call[] = [];
  rev = 5;
  paused = false;
  revoked = false;
  deviceId = 'dev-1';
  pcName = 'Home PC';
  me: any = { deviceId: 'dev-1', name: 'iPhone', pcName: 'Home PC', caps: ['watch', 'answer', 'type', 'control'], groups: false, paused: false, candidates: [LAN] };
  bots: any[] = [bot()];
  rooms: any[] = [];
  chats: Record<string, any> = { b1: chat() };
  roomData: Record<string, any> = {};
  errors: Record<string, { status: number; error: string; message: string }> = {};
  waiters: Array<() => void> = [];
  pairError: any = null;
  pairCandidates: string[] = [LAN];
  pushKey = Buffer.from('push-public-key-bytes-1234567890').toString('base64url');

  constructor(origins: string[] = [LAN]) { for (const o of origins) this.behaviors[o] = 'ok'; }

  bump() { this.rev++; const w = this.waiters; this.waiters = []; w.forEach((f) => f()); }
  callsTo(path: string, method?: string) {
    return this.calls.filter((c) => c.path.split('?')[0] === path && (!method || c.method === method));
  }
  hellos() { return this.calls.filter((c) => c.path === '/api/hello'); }

  proof(kind: string, nonce: string) {
    const key = kind === 'pair'
      ? createHash('sha256').update(CODE.toUpperCase()).digest()
      : createHash('sha256').update(Buffer.from(TOKEN, 'base64')).digest();
    return createHmac('sha256', key).update('potato-phone-hello|' + nonce).digest('hex');
  }

  reply(status: number, body: any) {
    return { ok: status >= 200 && status < 300, status, json: async () => body };
  }

  fetch = (url: string, init: any = {}): Promise<any> => {
    const u = new URL(url);
    const origin = u.origin;
    const path = u.pathname + u.search;
    const headers: Record<string, string> = {};
    for (const [k, v] of Object.entries(init.headers || {})) headers[k.toLowerCase()] = String(v);
    const body = init.body ? JSON.parse(init.body) : undefined;
    this.calls.push({ origin, method: init.method || 'GET', path, headers, body });
    const beh = this.behaviors[origin] || 'down';
    const signal: AbortSignal | undefined = init.signal;
    const aborted = () => { const e: any = new Error('aborted'); e.name = 'AbortError'; return e; };
    if (beh === 'down') return Promise.reject(new TypeError('Failed to fetch'));
    if (beh === 'timeout') {
      return new Promise((_, rej) => {
        if (signal?.aborted) return rej(aborted());
        signal?.addEventListener('abort', () => rej(aborted()));
      });
    }
    if (u.pathname === '/api/hello') {
      const proof = beh === 'wrong' ? 'f'.repeat(64) : this.proof(body.kind, body.nonce);
      return Promise.resolve(this.reply(200, { proof, pcName: this.pcName }));
    }
    if (beh === 'wrong') return Promise.resolve(this.reply(500, { error: 'nope', message: 'wrong machine' }));
    if (u.pathname === '/api/changes') {
      const since = Number(u.searchParams.get('since') || 0);
      if (this.rev > since) return Promise.resolve(this.reply(200, { rev: this.rev }));
      return new Promise((res, rej) => {
        this.waiters.push(() => res(this.reply(200, { rev: this.rev })));
        signal?.addEventListener('abort', () => rej(aborted()));
      });
    }
    return Promise.resolve(this.handle(u, init.method || 'GET', headers, body));
  };

  handle(u: URL, method: string, headers: Record<string, string>, body: any) {
    const p = u.pathname;
    if (p === '/api/health') return this.reply(200, { ok: true });
    if (p === '/api/pair') {
      if (this.pairError) return this.reply(this.pairError.status, this.pairError);
      if (body.code !== CODE) return this.reply(400, { error: 'bad_code', message: 'That code is not right.' });
      return this.reply(200, { deviceId: this.deviceId, token: TOKEN, pcName: this.pcName, candidates: this.pairCandidates });
    }
    if (headers.authorization !== 'Bearer ' + TOKEN || this.revoked) {
      return this.reply(401, { error: 'unauthorized', message: 'This phone was removed on your PC.' });
    }
    const forced = this.errors[method + ' ' + p];
    if (forced) return this.reply(forced.status, forced);
    if (p === '/api/me') {
      if (method === 'DELETE') { this.revoked = true; return this.reply(200, { ok: true }); }
      return this.reply(200, { ...this.me, paused: this.paused });
    }
    if (p === '/api/bots' && method === 'GET') {
      return this.reply(200, { rev: this.rev, paused: this.paused, bots: this.bots, rooms: this.rooms });
    }
    let m = /^\/api\/bots\/([^/]+)$/.exec(p);
    if (m && method === 'GET') {
      const c = this.chats[decodeURIComponent(m[1])];
      return c ? this.reply(200, { ...c, rev: this.rev }) : this.reply(404, { error: 'not_found', message: 'No such bot.' });
    }
    m = /^\/api\/bots\/([^/]+)\/send$/.exec(p);
    if (m) {
      const c = this.chats[decodeURIComponent(m[1])];
      const id = 'm' + (c.messages.length + 100);
      c.messages.push({ id, at: 1_700_000_000_000, from: { kind: 'you' }, kind: 'text', text: body.text });
      return this.reply(200, { id, delivered: true, state: 'working' });
    }
    m = /^\/api\/bots\/([^/]+)\/cards\/([^/]+)\/answer$/.exec(p);
    if (m) {
      const c = this.chats[decodeURIComponent(m[1])];
      if (c) c.cards = c.cards.filter((x: any) => x.id !== decodeURIComponent(m![2]));
      return this.reply(200, { ok: true });
    }
    if (/^\/api\/bots\/[^/]+\/stop$/.test(p)) return this.reply(200, { ok: true });
    if (p === '/api/pause-all') { this.paused = true; return this.reply(200, { paused: true }); }
    if (p === '/api/resume-all') { this.paused = false; return this.reply(200, { paused: false }); }
    m = /^\/api\/rooms\/([^/]+)$/.exec(p);
    if (m) return this.reply(200, { rev: this.rev, ...this.roomData[decodeURIComponent(m[1])] });
    if (/^\/api\/rooms\/[^/]+\/send$/.test(p)) return this.reply(200, { ok: true });
    if (p === '/api/push/key') return this.reply(200, { publicKey: this.pushKey });
    if (p === '/api/push/subscribe' || p === '/api/push/presence') return this.reply(200, { ok: true });
    return this.reply(404, { error: 'not_found', message: 'Unknown.' });
  }
}

export function memoryStorage(initial?: Record<string, string>) {
  const data = new Map<string, string>(Object.entries(initial || {}));
  return {
    data,
    getItem: (k: string) => (data.has(k) ? data.get(k)! : null),
    setItem: (k: string, v: string) => { data.set(k, String(v)); },
    removeItem: (k: string) => { data.delete(k); }
  };
}

export const STORE_KEY = 'potato.phone.v1';
export function pairedStore(candidates: string[] = [LAN]) {
  return { [STORE_KEY]: JSON.stringify({ token: TOKEN, deviceId: 'dev-1', candidates }) };
}

export interface MountOpts {
  server?: FakeServer;
  hash?: string;
  stored?: Record<string, string> | null;
  lang?: string;
  origin?: string;
  nav?: any;
}

export function mount(opts: MountOpts = {}) {
  const server = opts.server || new FakeServer();
  const storage = memoryStorage(opts.stored === undefined ? pairedStore() : opts.stored || {});
  document.body.innerHTML = '';
  const root = document.createElement('div');
  document.body.append(root);
  const navigator: any = { language: opts.lang || 'en-US', userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0)', ...(opts.nav || {}) };
  const location = { origin: opts.origin || LAN, hash: opts.hash || '' };
  const app = mountPhoneApp(root, {
    fetch: server.fetch,
    storage,
    location,
    navigator,
    now: () => 1_700_000_000_000,
    setTimeout: (f: () => void, ms: number) => globalThis.setTimeout(f, ms === 2500 ? 30 : ms),
    crypto: webcrypto
  });
  return { root: root as HTMLElement, server, storage, app, navigator, location };
}

export const sleep = (ms = 0) => new Promise((r) => setTimeout(r, ms));

export function btn(root: HTMLElement, text: string): HTMLButtonElement {
  const b = Array.from(root.querySelectorAll('button')).find((x) => (x.textContent || '').trim() === text);
  if (!b) throw new Error('no button "' + text + '"; buttons: ' + Array.from(root.querySelectorAll('button')).map((x) => x.textContent).join('|'));
  return b as HTMLButtonElement;
}
export function hasText(root: HTMLElement, text: string) { return (root.textContent || '').includes(text); }

/** Mount an already paired app and wait for the bot list. */
export async function mountList(opts: MountOpts = {}) {
  const m = mount(opts);
  await waitFor(() => !!m.root.querySelector('.row, .empty'));
  return m;
}
export async function openChat(m: ReturnType<typeof mount>, id = 'b1') {
  (m.root.querySelector(`[data-bot="${id}"]`) as HTMLElement).click();
  await waitFor(() => !!m.root.querySelector('.composer'));
  await waitFor(() => !!m.root.querySelector('.bar-title h1'));
}

export async function waitFor(fn: () => boolean | void, timeout = 2000) {
  const t0 = Date.now();
  for (;;) {
    try { if (fn() !== false) return; } catch (e) { if (Date.now() - t0 > timeout) throw e; }
    if (Date.now() - t0 > timeout) throw new Error('waitFor timed out');
    await sleep(5);
  }
}
