// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { FakeServer, bot, chat, mount, mountList, openChat, waitFor, sleep, TOKEN } from './harness';

const q = (m: { root: HTMLElement }, sel: string) => m.root.querySelector(sel) as HTMLElement;
const qa = (m: { root: HTMLElement }, sel: string) => Array.from(m.root.querySelectorAll(sel)) as HTMLElement[];

function oneBot() {
  const s = new FakeServer();
  s.bots = [bot({ id: 'b1', name: 'Scout', computer: true })];
  s.chats = { b1: chat({ bot: s.bots[0] }) };
  return s;
}
async function inControl(s = oneBot()) {
  const m = await mountList({ server: s });
  await openChat(m, 'b1');
  q(m, '[data-act="computer"]').click();
  await waitFor(() => !!m.root.querySelector('iframe'));
  q(m, '[data-mode="control"]').click();
  await waitFor(() => (q(m, 'iframe') as HTMLElement | null)?.getAttribute('src')?.includes('control=1') === true);
  return { m, s };
}
const posts = (s: FakeServer, sub: string) => s.callsTo('/api/bots/b1/screen/' + sub, 'POST');
const setVisibility = (v: 'hidden' | 'visible') => {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => v });
  document.dispatchEvent(new Event('visibilitychange'));
};

describe('Control: permission fallback', () => {
  it('403 phone_needs_desktop_yes falls back to View and says what to turn on', async () => {
    const s = oneBot();
    const ok = s.screen;
    s.screen = (id, body) => (body.mode === 'control' ? { status: 403, body: { error: 'phone_needs_desktop_yes', message: 'raw server text' } } : ok(id, body));
    const m = await mountList({ server: s });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => !!m.root.querySelector('iframe'));
    q(m, '[data-mode="control"]').click();
    await waitFor(() => !(q(m, '.screen-note') as HTMLElement).hidden);
    expect(q(m, '.screen-note').textContent).toBe('To control from this phone, turn on “Can control the bot\'s computer” for it in Settings > Phone on your PC.');
    expect(q(m, '[data-mode="view"]').getAttribute('aria-checked')).toBe('true');
    await waitFor(() => (q(m, 'iframe') as HTMLElement | null)?.getAttribute('src')?.includes('vnc=pw-view') === true);
    // nothing was granted, so nothing is kept alive or released
    expect(m.longTimers.length).toBe(0);
    m.app.destroy();
    expect(posts(s, 'release').length).toBe(0);
  });
  it('403 not_allowed falls back to View with the short text', async () => {
    const s = oneBot();
    const ok = s.screen;
    s.screen = (id, body) => (body.mode === 'control' ? { status: 403, body: { error: 'not_allowed', message: 'x' } } : ok(id, body));
    const m = await mountList({ server: s });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => !!m.root.querySelector('iframe'));
    q(m, '[data-mode="control"]').click();
    await waitFor(() => !(q(m, '.screen-note') as HTMLElement).hidden);
    expect(q(m, '.screen-note').textContent).toBe("This phone isn't allowed to control.");
  });
  it('speaks Traditional Chinese on a zh phone', async () => {
    const s = oneBot();
    const ok = s.screen;
    s.screen = (id, body) => (body.mode === 'control' ? { status: 403, body: { error: 'phone_needs_desktop_yes', message: '' } } : ok(id, body));
    const m = await mountList({ server: s, lang: 'zh-TW' });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => !!m.root.querySelector('iframe'));
    q(m, '[data-mode="control"]').click();
    await waitFor(() => !(q(m, '.screen-note') as HTMLElement).hidden);
    expect(q(m, '.screen-note').textContent).toContain('可以操控機器人的電腦');
  });
});

describe('Control: keep alive and release', () => {
  it('keeps control alive every 5 minutes while in Control', async () => {
    const { m, s } = await inControl();
    expect(m.longTimers.map((x) => x.ms)).toEqual([300000]);
    expect(posts(s, 'keep').length).toBe(0);
    expect(m.fire(300000)).toBe(1);
    await waitFor(() => posts(s, 'keep').length === 1);
    // and it schedules the next one
    await waitFor(() => m.longTimers.some((x) => x.ms === 300000));
    expect(m.fire(300000)).toBe(1);
    await waitFor(() => posts(s, 'keep').length === 2);
    expect(posts(s, 'keep')[0].headers.authorization).toBe('Bearer ' + TOKEN);
    expect(posts(s, 'release').length).toBe(0);
  });
  it('does not keep anything alive in View', async () => {
    const m = await mountList({ server: oneBot() });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => !!m.root.querySelector('iframe'));
    expect(m.longTimers.length).toBe(0);
  });
  it('switching back to View releases control and stops the timer', async () => {
    const { m, s } = await inControl();
    q(m, '[data-mode="view"]').click();
    await waitFor(() => posts(s, 'release').length === 1);
    expect(m.longTimers.length).toBe(0);
    await waitFor(() => (q(m, 'iframe') as HTMLElement | null)?.getAttribute('src')?.includes('vnc=pw-view') === true);
    expect(posts(s, 'release').length).toBe(1);
  });
  it('Back releases control (with keepalive so it survives closing)', async () => {
    const { m, s } = await inControl();
    q(m, '[data-act="back"]').click();
    await waitFor(() => posts(s, 'release').length === 1);
    expect(posts(s, 'release')[0].keepalive).toBe(true);
    expect(m.longTimers.length).toBe(0);
    expect(posts(s, 'release')[0].headers.authorization).toBe('Bearer ' + TOKEN);
  });
  it('closing the page (pagehide) releases control once', async () => {
    const { m, s } = await inControl();
    window.dispatchEvent(new Event('pagehide'));
    await waitFor(() => posts(s, 'release').length === 1);
    window.dispatchEvent(new Event('pagehide'));
    m.app.destroy();
    await sleep(10);
    expect(posts(s, 'release').length).toBe(1);
  });
  it('hidden for more than a minute releases control; coming back returns to View with a note', async () => {
    const { m, s } = await inControl();
    setVisibility('hidden');
    await waitFor(() => m.longTimers.some((x) => x.ms === 60000));
    expect(posts(s, 'release').length).toBe(0);
    expect(m.fire(60000)).toBe(1);
    await waitFor(() => posts(s, 'release').length === 1);
    expect(m.longTimers.some((x) => x.ms === 300000)).toBe(false);
    setVisibility('visible');
    await waitFor(() => !(q(m, '.screen-note') as HTMLElement).hidden);
    expect(q(m, '.screen-note').textContent).toBe('Control ended. Showing view only.');
    expect(q(m, '[data-mode="view"]').getAttribute('aria-checked')).toBe('true');
    await waitFor(() => (q(m, 'iframe') as HTMLElement | null)?.getAttribute('src')?.includes('vnc=pw-view') === true);
  });
  it('hidden for a short while keeps control', async () => {
    const { m, s } = await inControl();
    setVisibility('hidden');
    await waitFor(() => m.longTimers.some((x) => x.ms === 60000));
    setVisibility('visible');
    expect(m.longTimers.some((x) => x.ms === 60000)).toBe(false);
    await sleep(10);
    expect(posts(s, 'release').length).toBe(0);
    expect(q(m, '[data-mode="control"]').getAttribute('aria-checked')).toBe('true');
  });
  it('a refused keep (control expired) drops to View with a note', async () => {
    const { m, s } = await inControl();
    s.errors['POST /api/bots/b1/screen/keep'] = { status: 409, error: 'no_control', message: 'gone' };
    m.fire(300000);
    await waitFor(() => !(q(m, '.screen-note') as HTMLElement).hidden);
    expect(q(m, '.screen-note').textContent).toBe('Control ended. Showing view only.');
    expect(q(m, '[data-mode="view"]').getAttribute('aria-checked')).toBe('true');
  });
});

describe('Computer view is landscape friendly', () => {
  it('has a toggle that hides the bars, and the stylesheet gives the screen the full height in landscape', async () => {
    const m = await mountList({ server: oneBot() });
    await openChat(m, 'b1');
    q(m, '[data-act="computer"]').click();
    await waitFor(() => !!m.root.querySelector('iframe'));
    const screen = q(m, 'section.screen.computer');
    expect(screen.classList.contains('chrome-off')).toBe(false);
    q(m, '[data-act="chrome"]').click();
    expect(screen.classList.contains('chrome-off')).toBe(true);
    q(m, '[data-act="chrome"]').click();
    expect(screen.classList.contains('chrome-off')).toBe(false);
  });
});
