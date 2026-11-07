// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { FakeServer, mount, mountList, waitFor, hasText, btn, sleep, CODE, TOKEN, LAN, LAN2, TUNNEL, STORE_KEY, pairedStore } from './harness';

const pairHash = (code = CODE, cands: string[] = [LAN]) => '#/pair/' + code + '?' + cands.map((c) => 'a=' + encodeURIComponent(c)).join('&');

describe('pairing', () => {
  it('pairs from the URL fragment, stores the token and shows the list', async () => {
    const server = new FakeServer([LAN]);
    const m = mount({ server, stored: null, hash: pairHash() });
    await waitFor(() => !!m.root.querySelector('.row'));
    const saved = JSON.parse(m.storage.data.get(STORE_KEY)!);
    expect(saved.token).toBe(TOKEN);
    expect(saved.deviceId).toBe('dev-1');
    expect(saved.candidates).toContain(LAN);
    const pair = server.callsTo('/api/pair')[0];
    expect(pair.body.code).toBe(CODE);
    expect(typeof pair.body.deviceName).toBe('string');
    expect(pair.headers['x-potato-phone']).toBe('1');
    expect(pair.headers.authorization).toBeUndefined();
  });
  it('uses a pair-kind hello and verifies its proof before posting the code', async () => {
    const server = new FakeServer([LAN]);
    const m = mount({ server, stored: null, hash: pairHash() });
    await waitFor(() => server.callsTo('/api/pair').length === 1);
    const h = server.hellos()[0];
    expect(h.body.kind).toBe('pair');
    expect(h.body.nonce).toMatch(/^[0-9a-f]{32}$/);
    expect(h.body.deviceId).toBeUndefined();
    expect(server.calls.indexOf(h)).toBeLessThan(server.calls.indexOf(server.callsTo('/api/pair')[0]));
    m.app.destroy();
  });
  it('clears the fragment from the address bar', async () => {
    const spy = vi.spyOn(window.history, 'replaceState');
    const m = mount({ stored: null, hash: pairHash() });
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(spy).toHaveBeenCalled();
    expect(String(spy.mock.calls[0][2])).not.toContain('pair');
    expect(window.location.hash).not.toContain('pair');
  });
  it('races the candidates from the fragment and pairs through the one that answers', async () => {
    const server = new FakeServer([TUNNEL]);
    server.behaviors[LAN] = 'timeout';
    const m = mount({ server, stored: null, hash: pairHash(CODE, [LAN, TUNNEL]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(server.callsTo('/api/pair')[0].origin).toBe(TUNNEL);
  });
  it('shows the scan hint when there is no token and no fragment', async () => {
    const m = mount({ stored: null });
    await waitFor(() => hasText(m.root, "Open the pairing QR on your PC (Settings > Phone) and scan it with this phone's camera."));
    expect(m.server.calls.length).toBe(0);
  });
  it('shows the server message for a wrong code', async () => {
    const server = new FakeServer([LAN]);
    server.pairError = { status: 400, error: 'bad_code', message: 'That code is not right.' };
    const m = mount({ server, stored: null, hash: pairHash() });
    await waitFor(() => hasText(m.root, 'That code is not right.'));
    expect(m.storage.data.has(STORE_KEY)).toBe(false);
  });
  it('shows the rate limit message', async () => {
    const server = new FakeServer([LAN]);
    server.pairError = { status: 429, error: 'rate_limited', message: 'Too many tries. Wait a minute.' };
    const m = mount({ server, stored: null, hash: pairHash() });
    await waitFor(() => hasText(m.root, 'Too many tries. Wait a minute.'));
  });
  it('shows the pairing-specific message when no candidate proves the code', async () => {
    const server = new FakeServer([LAN]);
    server.behaviors[LAN] = 'wrong';
    const m = mount({ server, stored: null, hash: pairHash() });
    await waitFor(() => hasText(m.root, 'This pairing code was already used or has run out'));
    expect(hasText(m.root, 'Make a new code')).toBe(true);
    expect(hasText(m.root, 'Sorry')).toBe(false);
    expect(server.callsTo('/api/pair').length).toBe(0);
    btn(m.root, 'Try again');
  });
  it('rejects a code in a bad fragment shape', async () => {
    const m = mount({ stored: null, hash: '#/pair/' });
    await waitFor(() => hasText(m.root, 'Open the pairing QR'));
  });
});

describe('finding the PC (candidates)', () => {
  it('all LAN candidates time out and the tunnel wins', async () => {
    const server = new FakeServer([TUNNEL]);
    server.behaviors[LAN] = 'timeout';
    server.behaviors[LAN2] = 'timeout';
    const m = mount({ server, stored: pairedStore([LAN, LAN2, TUNNEL]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    const api = server.calls.filter((c) => c.path.startsWith('/api/bots'));
    expect(api.length).toBeGreaterThan(0);
    expect(api.every((c) => c.origin === TUNNEL)).toBe(true);
    expect(server.hellos().map((h) => h.origin).sort()).toEqual([LAN, LAN2, TUNNEL].sort());
  });
  it('the hello for a paired phone sends the deviceId and a fresh nonce each time', async () => {
    const server = new FakeServer([LAN, LAN2]);
    const m = mount({ server, stored: pairedStore([LAN, LAN2]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    const hs = server.hellos();
    expect(hs.length).toBe(2);
    for (const h of hs) {
      expect(h.body.kind).toBe('device');
      expect(h.body.deviceId).toBe('dev-1');
      expect(h.headers['x-potato-phone']).toBe('1');
      expect(h.headers.authorization).toBeUndefined();
    }
    expect(hs[0].body.nonce).not.toBe(hs[1].body.nonce);
  });
  it('rejects a wrong machine (bad proof) at the first address and uses the next', async () => {
    const server = new FakeServer([LAN2]);
    server.behaviors[LAN] = 'wrong';
    const m = mount({ server, stored: pairedStore([LAN, LAN2]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(server.calls.filter((c) => c.origin === LAN && c.path !== '/api/hello').length).toBe(0);
    expect(server.calls.some((c) => c.origin === LAN2 && c.path === '/api/bots')).toBe(true);
  });
  it('never sends the token to a machine that failed the proof', async () => {
    const server = new FakeServer([LAN2]);
    server.behaviors[LAN] = 'wrong';
    const m = mount({ server, stored: pairedStore([LAN, LAN2]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(server.calls.filter((c) => c.origin === LAN).every((c) => !c.headers.authorization)).toBe(true);
  });
  it('a network-change event triggers a new race', async () => {
    const m = await mountList();
    const before = m.server.hellos().length;
    window.dispatchEvent(new Event('online'));
    await waitFor(() => m.server.hellos().length > before);
    await sleep(20);
    const mid = m.server.hellos().length;
    window.dispatchEvent(new Event('offline'));
    await waitFor(() => m.server.hellos().length > mid);
  });
  it('navigator.connection change triggers a new race', async () => {
    const conn = new EventTarget();
    const m = mount({ nav: { connection: conn } });
    await waitFor(() => !!m.root.querySelector('.row'));
    const before = m.server.hellos().length;
    conn.dispatchEvent(new Event('change'));
    await waitFor(() => m.server.hellos().length > before);
  });
  it('coming back within 30 s does not race again', async () => {
    const m = await mountList();
    const before = m.server.hellos().length;
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
    document.dispatchEvent(new Event('visibilitychange'));
    await sleep(30);
    // same clock => under 30 s => no new race
    expect(m.server.hellos().length).toBe(before);
    m.app.destroy();
  });
  it('keeps the generic unreachable screen for an already paired phone', async () => {
    const server = new FakeServer([LAN]);
    server.behaviors[LAN] = 'wrong';
    const m = mount({ server, stored: pairedStore([LAN]) });
    await waitFor(() => hasText(m.root, 'This phone cannot reach your PC.'));
    expect(hasText(m.root, 'pairing code')).toBe(false);
  });
  it('shows the plain screen when no candidate answers, and Try again recovers', async () => {
    const server = new FakeServer([LAN]);
    server.behaviors[LAN] = 'timeout';
    const m = mount({ server, stored: pairedStore([LAN]) });
    await waitFor(() => hasText(m.root, 'This phone cannot reach your PC.'));
    expect(hasText(m.root, 'turn on Reach from outside your Wi-Fi')).toBe(true);
    server.behaviors[LAN] = 'ok';
    btn(m.root, 'Try again').click();
    await waitFor(() => !!m.root.querySelector('.row'));
  });
  it('a failed request re-races and retries once on the new base', async () => {
    const server = new FakeServer([LAN, TUNNEL]);
    server.me.candidates = [LAN, TUNNEL];
    const m = mount({ server, stored: pairedStore([LAN, TUNNEL]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    // the LAN dies; the next refresh fails, re-races and lands on the tunnel
    server.behaviors[LAN] = 'down';
    server.rev++;
    const base = (m.root.querySelector('[data-bot]') as HTMLElement);
    base.click();
    await waitFor(() => server.calls.some((c) => c.origin === TUNNEL && c.path.startsWith('/api/bots/b1')));
    await waitFor(() => hasText(m.root, 'Scout'));
  });
  it('refreshes the stored candidates from /api/me', async () => {
    const server = new FakeServer([LAN]);
    server.me.candidates = [LAN, TUNNEL];
    const m = mount({ server, stored: pairedStore([LAN]) });
    await waitFor(() => !!m.root.querySelector('.row'));
    expect(JSON.parse(m.storage.data.get(STORE_KEY)!).candidates).toEqual([LAN, TUNNEL]);
  });
  it('sends X-Potato-Phone and the bearer on every authorised call and never puts the token in a URL or the DOM', async () => {
    const m = await mountList();
    for (const c of m.server.calls.filter((x) => x.path !== '/api/hello')) {
      expect(c.headers['x-potato-phone'], c.path).toBe('1');
      expect(c.headers.authorization, c.path).toBe('Bearer ' + TOKEN);
      expect(c.path).not.toContain(TOKEN);
    }
    expect(document.documentElement.outerHTML).not.toContain(TOKEN);
  });
});

describe('signed out', () => {
  it('401 clears storage and returns to the pair screen with the message', async () => {
    const m = await mountList();
    m.server.revoked = true;
    m.server.bump();
    await waitFor(() => hasText(m.root, 'This phone was removed on your PC.'));
    expect(hasText(m.root, 'Open the pairing QR')).toBe(true);
    expect(m.storage.data.has(STORE_KEY)).toBe(false);
  });
  it('401 on first load shows the pair screen', async () => {
    const server = new FakeServer([LAN]);
    server.revoked = true;
    const m = mount({ server });
    await waitFor(() => hasText(m.root, 'Open the pairing QR'));
    expect(m.storage.data.has(STORE_KEY)).toBe(false);
  });
});
