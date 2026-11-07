// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { webcrypto, createHash, createHmac } from 'node:crypto';
// @ts-ignore plain JS module
import { sha256, hmacSha256, helloKey, helloProof, toHex } from '../../phone/net.js';
// @ts-ignore plain JS module
import { mountPhoneApp } from '../../phone/app.js';
import { FakeServer, memoryStorage, pairedStore, waitFor, hasText, LAN, CODE, TOKEN } from './harness';

const enc = new TextEncoder();
const noSubtle = { getRandomValues: (a: Uint8Array) => webcrypto.getRandomValues(a) };

describe('pure JS SHA-256 and HMAC', () => {
  it('matches the known vectors', () => {
    expect(toHex(sha256(enc.encode('abc')))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    expect(toHex(hmacSha256(enc.encode('Jefe'), enc.encode('what do ya want for nothing?')))).toBe('5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843');
  });
  it('matches node for many lengths and long keys', () => {
    for (const n of [0, 1, 55, 56, 63, 64, 65, 119, 120, 1000]) {
      const d = new Uint8Array(n).map((_, i) => (i * 7 + n) & 255);
      expect(toHex(sha256(d)), String(n)).toBe(createHash('sha256').update(d).digest('hex'));
      expect(toHex(hmacSha256(d, enc.encode('x'))), 'h' + n).toBe(createHmac('sha256', d).update('x').digest('hex'));
    }
  });
  it('WebCrypto and the fallback give the same key and proof', async () => {
    for (const [kind, secret] of [['pair', 'abcd2345'], ['device', TOKEN]]) {
      const k1 = await helloKey(webcrypto, kind, secret);
      const k2 = await helloKey(noSubtle, kind, secret);
      expect(toHex(k2)).toBe(toHex(k1));
      expect(await helloProof(noSubtle, k2, 'ab12')).toBe(await helloProof(webcrypto, k1, 'ab12'));
    }
  });
});

describe('plain http (no crypto.subtle, not a secure context)', () => {
  function run(stored: Record<string, string> | null, hash = '') {
    Object.defineProperty(window, 'isSecureContext', { configurable: true, value: false });
    const server = new FakeServer([LAN]);
    const storage = memoryStorage(stored || {});
    document.body.innerHTML = '';
    const root = document.createElement('div');
    document.body.append(root);
    const register = vi.fn(async () => { throw new Error('insecure'); });
    mountPhoneApp(root, {
      fetch: server.fetch, storage, location: { origin: LAN, hash },
      navigator: { language: 'en', userAgent: 'x', serviceWorker: { register } },
      now: () => 1_700_000_000_000, setTimeout: (f: any, ms: number) => globalThis.setTimeout(f, ms === 2500 ? 30 : ms),
      crypto: noSubtle
    });
    return { root, server, storage, register };
  }
  it('connects and lists bots', async () => {
    const m = run(pairedStore());
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(m.register).toHaveBeenCalled();
  });
  it('pairs from a fragment', async () => {
    const m = run(null, '#/pair/' + CODE + '?a=' + encodeURIComponent(LAN));
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(m.storage.data.size).toBe(1);
  });
  it('Settings explains notifications need https', async () => {
    const m = run(pairedStore());
    await waitFor(() => !!m.root.querySelector('.row'));
    (m.root.querySelector('[data-act="settings"]') as HTMLElement).click();
    await waitFor(() => hasText(m.root, 'Notifications need a secure address (https). Use the Reach from outside link or your own https address.'));
    expect((m.root.querySelector('.switch') as HTMLButtonElement).disabled).toBe(true);
  });
});
