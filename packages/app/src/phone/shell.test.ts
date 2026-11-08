// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { FakeServer, bot, chat, mount, mountList, openChat, waitFor, sleep, LAN } from './harness';

const q = (m: { root: HTMLElement }, sel: string) => m.root.querySelector(sel) as HTMLElement;
const qa = (m: { root: HTMLElement }, sel: string) => Array.from(m.root.querySelectorAll(sel)) as HTMLElement[];

function fakeViewport(height: number, offsetTop = 0) {
  const listeners: Record<string, Array<() => void>> = {};
  const vv: any = {
    height, offsetTop, width: 390,
    addEventListener: (t: string, f: () => void) => { (listeners[t] = listeners[t] || []).push(f); },
    removeEventListener: () => {},
    fire: (t = 'resize') => (listeners[t] || []).forEach((f) => f())
  };
  return vv;
}
const rootVar = (n: string) => document.documentElement.style.getPropertyValue(n);

function twoBots() {
  const s = new FakeServer();
  s.bots = [bot({ id: 'b1', name: 'Scout', computer: true }), bot({ id: 'b2', name: 'Writer' })];
  s.chats = { b1: chat({ bot: s.bots[0] }), b2: chat({ bot: s.bots[1] }) };
  return s;
}
function msgFromWindow(m: any, data: any, o: { source?: any; origin?: string } = {}) {
  const frame = q(m, 'iframe') as HTMLIFrameElement;
  window.dispatchEvent(new MessageEvent('message', { data, origin: o.origin ?? LAN, source: o.source === undefined ? frame.contentWindow : o.source }));
}
const statusText = (m: any) => q(m, '.screen-status-text').textContent;

describe('app shell', () => {
  it('has the header first and the composer pinned last in the chat', async () => {
    const m = await mountList({ server: twoBots() });
    await openChat(m);
    const screen = q(m, 'section.screen.chat');
    const kids = Array.from(screen.children) as HTMLElement[];
    expect(kids[0].classList.contains('head')).toBe(true);
    expect(kids[0].querySelector('header.bar')).not.toBeNull();
    expect(kids[kids.length - 1].classList.contains('dock')).toBe(true);
    const dock = kids[kids.length - 1];
    expect(dock.lastElementChild!.classList.contains('composer')).toBe(true);
    // the message list sits between them and is the only scroller
    expect(kids.filter((k) => k.classList.contains('scroll')).length).toBe(1);
    expect(kids.indexOf(q(m, '.chat-scroll'))).toBeGreaterThan(0);
    expect(kids.indexOf(q(m, '.chat-scroll'))).toBeLessThan(kids.length - 1);
  });
  it('shows a header with a connection dot on the list and on the status screens', async () => {
    const m = await mountList({ server: twoBots() });
    expect(q(m, '.head .conn')).not.toBeNull();
    const m2 = mount({ stored: null });
    await waitFor(() => !!m2.root.querySelector('.head'));
    expect(m2.root.querySelector('.head .brand-name')!.textContent).toBe('Potato Bot');
  });
  it('groups messages from one sender and shows the name once', async () => {
    const s = twoBots();
    const t0 = 1_700_000_000_000;
    s.chats.b1.messages = [
      { id: 'a', at: t0 - 50000, from: { kind: 'bot', id: 'b1', name: 'Scout' }, kind: 'text', text: 'one' },
      { id: 'b', at: t0 - 40000, from: { kind: 'bot', id: 'b1', name: 'Scout' }, kind: 'text', text: 'two' },
      { id: 'c', at: t0 - 30000, from: { kind: 'you' }, kind: 'text', text: 'three' }
    ];
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => qa(m, '.msg').length === 3);
    expect(qa(m, '.msg').map((x) => x.getAttribute('data-pos'))).toEqual(['first', 'last', 'single']);
    expect(qa(m, '.who').length).toBe(1);
  });
});

describe('keyboard (visual viewport)', () => {
  it('publishes the keyboard height and the visible height as CSS variables, and follows resizes', async () => {
    (window as any).innerHeight = 844;
    const vv = fakeViewport(844);
    const m = await mountList({ server: twoBots(), visualViewport: vv });
    expect(rootVar('--kb')).toBe('0px');
    expect(m.root.querySelector('.app')!.classList.contains('kb-open')).toBe(false);
    vv.height = 508; vv.fire('resize');
    expect(rootVar('--kb')).toBe('336px');
    expect(rootVar('--vvh')).toBe('508px');
    expect(rootVar('--vvtop')).toBe('0px');
    expect(m.root.querySelector('.app')!.classList.contains('kb-open')).toBe(true);
    // iOS scrolls the layout viewport: the shell follows the offset
    vv.height = 480; vv.offsetTop = 40; vv.fire('scroll');
    expect(rootVar('--vvtop')).toBe('40px');
    expect(rootVar('--kb')).toBe('324px');
    vv.height = 844; vv.offsetTop = 0; vv.fire('resize');
    expect(rootVar('--kb')).toBe('0px');
    expect(m.root.querySelector('.app')!.classList.contains('kb-open')).toBe(false);
    m.app.destroy();
  });
  it('keeps the newest message in view when the keyboard opens (pinned to the bottom)', async () => {
    (window as any).innerHeight = 844;
    const vv = fakeViewport(844);
    const m = await mountList({ server: twoBots(), visualViewport: vv });
    await openChat(m);
    const sc = q(m, '.chat-scroll');
    Object.defineProperty(sc, 'scrollHeight', { configurable: true, get: () => 2000 });
    Object.defineProperty(sc, 'clientHeight', { configurable: true, get: () => 400 });
    sc.scrollTop = 1600; sc.dispatchEvent(new Event('scroll'));
    sc.scrollTop = 0;
    vv.height = 500; vv.fire('resize');
    expect(sc.scrollTop).toBe(2000);
    // a reader who scrolled up is left alone
    sc.scrollTop = 100; sc.dispatchEvent(new Event('scroll'));
    vv.height = 844; vv.fire('resize');
    expect(sc.scrollTop).toBe(100);
    m.app.destroy();
  });
  it('works without visualViewport (plain browsers)', async () => {
    const m = await mountList({ server: twoBots() });
    expect(q(m, '.composer, .row')).not.toBeNull();
    m.app.destroy();
  });
});

describe('Computer button', () => {
  it('shows only for a bot that has computer:true', async () => {
    const s = twoBots();
    const m = await mountList({ server: s });
    await openChat(m, 'b1');
    expect(q(m, '[data-act="computer"]')).not.toBeNull();
    (q(m, '[data-act="back"]')).click();
    await waitFor(() => qa(m, '.row').length === 2);
    await openChat(m, 'b2');
    expect(m.root.querySelector('[data-act="computer"]')).toBeNull();
  });
  it('learns the flag from the bot list when a chat is opened by link', async () => {
    const s = twoBots();
    const m = mount({ server: s, hash: '#/bot/b1' });
    await waitFor(() => !!m.root.querySelector('[data-act="computer"]'));
  });
});

describe('Computer screen', () => {
  async function openComputer(s = twoBots()) {
    const m = await mountList({ server: s });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => !!m.root.querySelector('iframe'));
    return { m, s };
  }
  it('asks for a view screen, loads the returned src exactly, and removes the iframe on Back', async () => {
    const { m, s } = await openComputer();
    expect(s.callsTo('/api/bots/b1/screen', 'POST')[0].body).toEqual({ mode: 'view' });
    const frame = q(m, 'iframe');
    expect(frame.getAttribute('src')).toBe('/sbx/embed.html?path=websockify#vnc=pw-view');
    expect(q(m, '[data-mode="view"]').getAttribute('aria-checked')).toBe('true');
    q(m, '[data-act="back"]').click();
    await waitFor(() => !m.root.querySelector('iframe'));
    await waitFor(() => !!m.root.querySelector('.composer'));
  });
  it('shows the status posted by its own iframe and offers Retry on failure', async () => {
    const { m, s } = await openComputer();
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'connecting' });
    expect(statusText(m)).toBe('Connecting...');
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'connected' });
    expect(statusText(m)).toBe('Live - watching');
    expect((q(m, '[data-act="screen-retry"]') as HTMLElement).hidden).toBe(true);
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'failed', reason: 'timeout' });
    expect(statusText(m)).toContain('Could not show the screen');
    const retry = q(m, '[data-act="screen-retry"]') as HTMLElement;
    expect(retry.hidden).toBe(false);
    retry.click();
    await waitFor(() => s.callsTo('/api/bots/b1/screen', 'POST').length === 2);
    await waitFor(() => !!m.root.querySelector('iframe') && statusText(m) === 'Loading the screen...');
  });
  it('ignores status messages from other windows, other origins and other types', async () => {
    const { m } = await openComputer();
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'connected' });
    expect(statusText(m)).toBe('Live - watching');
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'failed', reason: 'error' }, { source: window });
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'failed', reason: 'error' }, { source: null });
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'failed', reason: 'error' }, { origin: 'https://evil.example' });
    msgFromWindow(m, { type: 'something-else', status: 'failed' });
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'rm -rf' });
    msgFromWindow(m, 'pm-sbx-screen-status');
    expect(statusText(m)).toBe('Live - watching');
    expect((q(m, '[data-act="screen-retry"]') as HTMLElement).hidden).toBe(true);
  });
  it('asks for a new control src when switching to Control, and falls back to View on 403', async () => {
    const { m, s } = await openComputer();
    q(m, '[data-mode="control"]').click();
    await waitFor(() => s.callsTo('/api/bots/b1/screen', 'POST').length === 2);
    expect(s.callsTo('/api/bots/b1/screen', 'POST')[1].body).toEqual({ mode: 'control' });
    await waitFor(() => (q(m, 'iframe') as HTMLElement | null)?.getAttribute('src') === '/sbx/embed.html?control=1&path=websockify#vnc=pw-control');
    expect(q(m, '[data-mode="control"]').getAttribute('aria-checked')).toBe('true');
    msgFromWindow(m, { type: 'pm-sbx-screen-status', status: 'connected' });
    expect(statusText(m)).toBe('Live - you are in control');
    // now this phone is not allowed to control
    const ok = s.screen;
    s.screen = (id, body) => (body.mode === 'control' ? { status: 403, body: { error: 'not_allowed', message: 'No control.' } } : ok(id, body));
    q(m, '[data-mode="view"]').click();
    await waitFor(() => statusText(m) === 'Loading the screen...');
    q(m, '[data-mode="control"]').click();
    await waitFor(() => !!m.root.querySelector('.screen-note') && !(m.root.querySelector('.screen-note') as HTMLElement).hidden);
    expect(q(m, '.screen-note').textContent).toBe("This phone isn't allowed to control.");
    expect(q(m, '[data-mode="view"]').getAttribute('aria-checked')).toBe('true');
    await waitFor(() => (q(m, 'iframe') as HTMLElement | null)?.getAttribute('src') === '/sbx/embed.html?path=websockify#vnc=pw-view');
  });
  it('shows the server message on other errors and refuses a src from another origin', async () => {
    const s = twoBots();
    s.screen = () => ({ status: 409, body: { error: 'sandbox_off', message: 'The sandbox is not running.' } });
    const m = await mountList({ server: s });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => statusText(m) === 'The sandbox is not running.');
    expect(m.root.querySelector('iframe')).toBeNull();
    expect((q(m, '[data-act="screen-retry"]') as HTMLElement).hidden).toBe(false);
    s.screen = (_id, body) => ({ status: 200, body: { src: 'https://evil.example/x', mode: body.mode, expiresAtMs: 1 } });
    q(m, '[data-act="screen-retry"]').click();
    await waitFor(() => statusText(m) === 'Could not open the computer. Try again.');
    expect(m.root.querySelector('iframe')).toBeNull();
    await sleep(5);
  });
  it('is reachable from a #/computer/<id> link and has no composer', async () => {
    const m = mount({ server: twoBots(), hash: '#/computer/b1' });
    await waitFor(() => !!m.root.querySelector('iframe'));
    expect(m.root.querySelector('.composer')).toBeNull();
  });
});

describe('chat layout with pending cards and a job', () => {
  function busy() {
    const s = twoBots();
    s.chats.b1 = chat({
      bot: s.bots[0],
      job: { goal: 'Plan the trip', step: 'Comparing hotels', startedMs: 1, spendNano: 1_230_000_000, state: 'running' },
      cards: [
        { id: 'c1', kind: 'approval', action: 'Run: npm install', why: 'Needed', choices: [], state: 'open', keepGoing: null, connector: null, login: null, mcpAdd: null, canAlwaysAllow: false },
        { id: 'c2', kind: 'question', action: 'Which area?', why: '', choices: ['Ximending', 'Daan'], state: 'open', keepGoing: null, connector: null, login: null, mcpAdd: null, canAlwaysAllow: false }
      ]
    });
    return s;
  }
  it('puts the cards inside the message list, not between the list and the composer', async () => {
    const m = await mountList({ server: busy() });
    await openChat(m);
    await waitFor(() => qa(m, '.card').length === 2);
    const scroll = q(m, '.chat-scroll');
    for (const c of qa(m, '.card')) expect(scroll.contains(c)).toBe(true);
    const dock = q(m, '.dock');
    expect(dock.querySelector('.card, .cards')).toBeNull();
    // the cards come after the last message
    const kids = Array.from(scroll.children);
    expect(kids.indexOf(q(m, '.cards'))).toBeGreaterThan(kids.indexOf(q(m, '.msgs')));
    // the dock holds only the composer (plus the floating pill), so the list keeps the height
    expect(Array.from(dock.children).filter((k) => !k.classList.contains('needs-pill') && !k.classList.contains('chips')).map((k) => k.className)).toEqual(['composer']);
  });
  it('keeps the job as one slim line under the header, not in the dock', async () => {
    const m = await mountList({ server: busy() });
    await openChat(m);
    await waitFor(() => !(q(m, '.jobbar') as HTMLElement).hidden);
    const bar = q(m, '.jobbar');
    expect(q(m, '.head').contains(bar)).toBe(true);
    expect(q(m, '.dock').contains(bar)).toBe(false);
    // goal, step and spend are inline spans of a single row
    expect(Array.from(bar.querySelector('.job-info')!.children).map((k) => k.tagName)).toEqual(['SPAN', 'SPAN', 'SPAN']);
    expect(bar.querySelectorAll('.btn').length).toBe(1);
  });
  it('shows a "needs you" pill only while the reader is scrolled away from the cards, and it jumps back', async () => {
    const m = await mountList({ server: busy() });
    await openChat(m);
    await waitFor(() => qa(m, '.card').length === 2);
    const pill = q(m, '.needs-pill') as HTMLElement;
    const sc = q(m, '.chat-scroll');
    Object.defineProperty(sc, 'scrollHeight', { configurable: true, get: () => 2000 });
    Object.defineProperty(sc, 'clientHeight', { configurable: true, get: () => 400 });
    expect(pill.hidden).toBe(true); // newest cards are scrolled into view
    sc.scrollTop = 100; sc.dispatchEvent(new Event('scroll'));
    expect(pill.hidden).toBe(false);
    expect(pill.textContent).toBe('2 needs you');
    pill.click();
    expect(sc.scrollTop).toBe(2000);
    expect(pill.hidden).toBe(true);
  });
  it('answering a card still works from inside the list', async () => {
    const s = busy();
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => qa(m, '.card').length === 2);
    (qa(m, '.card[data-card="c1"] button').find((b) => b.textContent === 'Allow once') as HTMLElement).click();
    await waitFor(() => s.callsTo('/api/bots/b1/cards/c1/answer', 'POST').length === 1);
    expect(s.callsTo('/api/bots/b1/cards/c1/answer', 'POST')[0].body).toEqual({ answer: 'allow_once' });
    await waitFor(() => qa(m, '.card').length === 1);
  });
  it('the stylesheet gives the cards no fixed share of the screen and keeps them in the flow', () => {
    const css = readFileSync(join(__dirname, '..', '..', 'phone', 'style.css'), 'utf8');
    const rule = /\n\.cards\s*\{[^}]*\}/.exec(css)![0];
    expect(rule).not.toMatch(/max-height|position:\s*(fixed|absolute|sticky)/);
    expect(css).not.toMatch(/\.dock\s+\.cards/);
  });
});
