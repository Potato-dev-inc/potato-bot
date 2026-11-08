// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { FakeServer, bot, chat, mount, mountList, waitFor, sleep, hasText, msgOf, spaceOf, LAN } from './harness';

const q = (m: { root: HTMLElement }, sel: string) => m.root.querySelector(sel) as HTMLElement;
const qa = (m: { root: HTMLElement }, sel: string) => Array.from(m.root.querySelectorAll(sel)) as HTMLElement[];

function withSpaces(on = true) {
  const s = new FakeServer([LAN]);
  s.me.spaces = on;
  s.bots = [bot({ id: 'b1', name: 'Scout', computer: true })];
  s.chats.b1 = chat({ bot: s.bots[0] });
  s.spaces = {
    online: true,
    spaces: [
      spaceOf({ unread: 3, channels: [{ id: 'c1', name: 'general', unread: 2, canWrite: true }, { id: 'c2', name: 'news', unread: 1, canWrite: false }] }),
      spaceOf({ id: 's2', name: 'Work', unread: 0, channels: [{ id: 'c9', name: 'ops', unread: 0, canWrite: true }] })
    ]
  };
  s.spaceMsgs['s1/c1'] = [
    msgOf(1, { authorName: 'Ann', text: 'hello there' }),
    msgOf(2, { authorName: 'Ann', text: 'second from Ann' }),
    msgOf(3, { authorName: 'Helper', bot: true, text: 'I can help', files: [{ name: 'plan.pdf', size: 2048 }] }),
    msgOf(4, { authorName: 'Me', mine: true, text: 'thanks' })
  ];
  return s;
}
async function openChannel(m: ReturnType<typeof mount>, space = 's1', chan = 'c1') {
  q(m, '[data-tab="spaces"]').click();
  await waitFor(() => !!m.root.querySelector(`[data-space="${space}"]`));
  q(m, `[data-space="${space}"]`).click();
  await waitFor(() => !!m.root.querySelector(`[data-channel="${chan}"]`));
  q(m, `[data-channel="${chan}"]`).click();
  await waitFor(() => !!m.root.querySelector('.screen.channel .bar-title h1') && (m.root.querySelector('.screen.channel .bar-title h1')!.textContent || '') !== '');
  await waitFor(() => qa(m, '.msg').length > 0 || !!m.root.querySelector('.empty'));
}
const type = (m: any, text: string) => {
  const ta = q(m, '.composer textarea') as HTMLTextAreaElement;
  ta.value = text;
  ta.dispatchEvent(new Event('input'));
};
const sendBtn = (m: any) => q(m, '.composer .send') as HTMLButtonElement;

describe('Bots / Spaces tabs', () => {
  it('show only when /api/me says spaces: true', async () => {
    const off = await mountList({ server: withSpaces(false) });
    expect(off.root.querySelector('.tabbar')).toBeNull();
    expect(off.server.callsTo('/api/spaces').length).toBe(0);
    const on = await mountList({ server: withSpaces(true) });
    await waitFor(() => !!on.root.querySelector('.tabbar'));
    expect(qa(on, '.tab').map((x) => x.getAttribute('data-tab'))).toEqual(['bots', 'spaces']);
    expect(q(on, '[data-tab="bots"]').getAttribute('aria-selected')).toBe('true');
  });
  it('show the unread total on the Spaces tab once the list is known', async () => {
    const m = await mountList({ server: withSpaces() });
    await waitFor(() => !!m.root.querySelector('[data-tab="spaces"] .count'));
    expect(q(m, '[data-tab="spaces"] .count').textContent).toBe('3');
  });
  it('hide when the PC answers 409 spaces_off', async () => {
    const s = withSpaces();
    s.spacesOff = true;
    const m = await mountList({ server: s });
    await sleep(30);
    expect(m.root.querySelector('.tabbar')).toBeNull();
    expect(q(m, '.row')).not.toBeNull();
  });
  it('a #/spaces link falls back to the bot list when spaces is off', async () => {
    const m = mount({ server: withSpaces(false), hash: '#/spaces' });
    await waitFor(() => !!m.root.querySelector('[data-bot="b1"]'));
  });
});

describe('Spaces list and channels', () => {
  it('lists the spaces, then the channels, then opens the channel chat', async () => {
    const m = await mountList({ server: withSpaces() });
    await waitFor(() => !!m.root.querySelector('.tabbar'));
    q(m, '[data-tab="spaces"]').click();
    await waitFor(() => qa(m, '[data-space]').length === 2);
    expect(q(m, '[data-space="s1"] .row-name').textContent).toBe('Family');
    expect(q(m, '[data-space="s1"] .count').textContent).toBe('3');
    expect(q(m, '[data-tab="spaces"]').getAttribute('aria-selected')).toBe('true');
    q(m, '[data-space="s1"]').click();
    await waitFor(() => qa(m, '[data-channel]').length === 2);
    expect(q(m, '[data-channel="c1"] .row-name').textContent).toBe('general');
    expect(q(m, '[data-channel="c1"] .count').textContent).toBe('2');
    expect(q(m, '[data-channel="c2"]').textContent).toContain('Read only');
    q(m, '[data-channel="c1"]').click();
    await waitFor(() => qa(m, '.msg').length === 4);
    expect(q(m, '.screen.channel .bar-title h1').textContent).toBe('# general');
    // Back walks up the same path
    q(m, '[data-act="back"]').click();
    await waitFor(() => qa(m, '[data-channel]').length === 2);
    q(m, '[data-act="back"]').click();
    await waitFor(() => qa(m, '[data-space]').length === 2);
  });
  it('is reachable from a #/space/<sid>/<cid> link', async () => {
    const m = mount({ server: withSpaces(), hash: '#/space/s1/c1' });
    await waitFor(() => qa(m, '.msg').length === 4);
    expect(q(m, '.screen.channel .bar-title h1').textContent).toBe('# general');
  });
  it('shows the quiet offline line when the PC Spaces is offline', async () => {
    const s = withSpaces();
    s.spaces.online = false;
    const m = await mountList({ server: s });
    q(m, '[data-tab="spaces"]').click();
    await waitFor(() => !!m.root.querySelector('[data-space]'));
    expect(hasText(m.root, "Your PC's Spaces is offline. Messages wait until someone is online.")).toBe(true);
    q(m, '[data-space="s1"]').click();
    await waitFor(() => !!m.root.querySelector('[data-channel]'));
    expect(hasText(m.root, "Your PC's Spaces is offline")).toBe(true);
    q(m, '[data-channel="c1"]').click();
    await waitFor(() => qa(m, '.msg').length === 4);
    expect((q(m, '.quiet-line') as HTMLElement).hidden).toBe(false);
  });
});

describe('Channel messages', () => {
  it('shows author names once per run, a bot mark, files, and my messages on the right', async () => {
    const m = await mountList({ server: withSpaces() });
    await openChannel(m);
    const msgs = qa(m, '.msg');
    expect(msgs.map((x) => x.getAttribute('data-pos'))).toEqual(['first', 'last', 'single', 'single']);
    expect(msgs.map((x) => x.classList.contains('mine'))).toEqual([false, false, false, true]);
    expect(qa(m, '.who-name').map((x) => x.textContent)).toEqual(['Ann', 'Helper']);
    expect(qa(m, '.botmark').length).toBe(1);
    expect(q(m, '.botmark').textContent).toBe('bot');
    expect(hasText(m.root, 'File: plan.pdf (2 KB)')).toBe(true);
  });
  it('renders message text as plain text', async () => {
    const s = withSpaces();
    s.spaceMsgs['s1/c1'] = [msgOf(1, { text: '<img src=x onerror=alert(1)>' })];
    const m = await mountList({ server: s });
    await openChannel(m);
    expect(m.root.querySelector('.msg img')).toBeNull();
    expect(hasText(m.root, '<img src=x onerror=alert(1)>')).toBe(true);
  });
  it('marks the channel read when it is open and at the bottom', async () => {
    const s = withSpaces();
    const m = await mountList({ server: s });
    await openChannel(m);
    await waitFor(() => s.callsTo('/api/spaces/s1/channels/c1/read', 'POST').length === 1);
    // a new message while pinned at the bottom is read again, an unchanged list is not
    s.spaceMsgs['s1/c1'].push(msgOf(5, { text: 'new one' }));
    s.bump();
    await waitFor(() => hasText(m.root, 'new one'));
    await waitFor(() => s.callsTo('/api/spaces/s1/channels/c1/read', 'POST').length === 2);
  });
  it('does not mark read while the reader is scrolled up', async () => {
    const s = withSpaces();
    const m = await mountList({ server: s });
    await openChannel(m);
    await waitFor(() => s.callsTo('/api/spaces/s1/channels/c1/read', 'POST').length === 1);
    const sc = q(m, '.chat-scroll');
    Object.defineProperty(sc, 'scrollHeight', { configurable: true, get: () => 2000 });
    Object.defineProperty(sc, 'clientHeight', { configurable: true, get: () => 400 });
    sc.scrollTop = 100; sc.dispatchEvent(new Event('scroll'));
    s.spaceMsgs['s1/c1'].push(msgOf(6, { text: 'while away' }));
    s.bump();
    await waitFor(() => hasText(m.root, 'while away'));
    await sleep(30);
    expect(s.callsTo('/api/spaces/s1/channels/c1/read', 'POST').length).toBe(1);
  });
  it('wakes on a space event and shows the new message', async () => {
    const s = withSpaces();
    const m = await mountList({ server: s });
    await openChannel(m);
    s.spaceMsgs['s1/c1'].push(msgOf(7, { authorName: 'Ben', text: 'ping from Ben' }));
    s.bump();
    await waitFor(() => hasText(m.root, 'ping from Ben'));
  });
  it('loads older messages when scrolled to the top (before=<oldest id>), keeping what is there', async () => {
    const s = withSpaces();
    s.spaceMsgs['s1/c1'] = Array.from({ length: 120 }, (_, i) => msgOf(i + 1));
    const m = await mountList({ server: s });
    await openChannel(m);
    expect(qa(m, '.msg').length).toBe(50);
    expect((q(m, '.earlier') as HTMLElement).hidden).toBe(false);
    const first = s.callsTo('/api/spaces/s1/channels/c1/messages')[0];
    expect(first.path).toContain('limit=50');
    expect(first.path).not.toContain('before=');
    const sc = q(m, '.chat-scroll');
    sc.scrollTop = 0;
    sc.dispatchEvent(new Event('scroll'));
    await waitFor(() => qa(m, '.msg').length === 100);
    const calls = s.callsTo('/api/spaces/s1/channels/c1/messages');
    expect(calls[calls.length - 1].path).toContain('before=sm0071');
    expect(qa(m, '.msg')[0].getAttribute('data-id')).toBe('sm0021');
    expect((q(m, '.earlier') as HTMLElement).hidden).toBe(false);
    // the button works too, and ends when the server says there is no more
    q(m, '.earlier').click();
    await waitFor(() => qa(m, '.msg').length === 120);
    expect((q(m, '.earlier') as HTMLElement).hidden).toBe(true);
    // a refresh after a wake keeps the older pages
    s.spaceMsgs['s1/c1'].push(msgOf(121));
    s.bump();
    await waitFor(() => qa(m, '.msg').length === 121);
    expect(qa(m, '.msg')[0].getAttribute('data-id')).toBe('sm0001');
  });
});

describe('Channel sending', () => {
  it('shows the message as pending, then sent', async () => {
    const s = withSpaces();
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => { release = r; });
    const orig = s.fetch;
    s.fetch = (url: string, init: any = {}) => (/\/send$/.test(url) ? gate.then(() => orig(url, init)) : orig(url, init));
    const m = await mountList({ server: s });
    await openChannel(m);
    type(m, 'hi family');
    sendBtn(m).click();
    await waitFor(() => qa(m, '.msg[data-state="pending"]').length === 1);
    expect(q(m, '.msg[data-state="pending"]').textContent).toContain('hi family');
    expect(q(m, '.msg[data-state="pending"] .badge').textContent).toBe('Sending');
    expect((q(m, '.composer textarea') as HTMLTextAreaElement).value).toBe('');
    release();
    await waitFor(() => qa(m, '.msg[data-state="pending"]').length === 0);
    await waitFor(() => qa(m, '.msg').length === 5);
    expect(s.callsTo('/api/spaces/s1/channels/c1/send', 'POST')[0].body).toEqual({ text: 'hi family' });
    expect(qa(m, '.msg').filter((x) => (x.textContent || '').includes('hi family')).length).toBe(1);
    expect((q(m, '.retry-line') as HTMLElement | null)?.hidden ?? true).toBe(true);
  });
  it('a failed send keeps the text in the box and shows a retry line, then retry works', async () => {
    const s = withSpaces();
    s.spaceSend = () => ({ status: 500, body: { error: 'boom', message: 'The PC said no.' } });
    const m = await mountList({ server: s });
    await openChannel(m);
    type(m, 'important words');
    sendBtn(m).click();
    await waitFor(() => !(q(m, '.retry-line') as HTMLElement).hidden);
    expect(q(m, '.retry-line').textContent).toContain('Your message is still in the box');
    expect((q(m, '.composer textarea') as HTMLTextAreaElement).value).toBe('important words');
    expect(qa(m, '.msg').length).toBe(4);
    // retry
    s.spaceSend = () => null;
    sendBtn(m).click();
    await waitFor(() => qa(m, '.msg').length === 5);
    expect((q(m, '.retry-line') as HTMLElement).hidden).toBe(true);
    expect((q(m, '.composer textarea') as HTMLTextAreaElement).value).toBe('');
  });
  it('a lost connection also keeps the text', async () => {
    const s = withSpaces();
    const m = await mountList({ server: s });
    await openChannel(m);
    s.behaviors[LAN] = 'down';
    type(m, 'offline words');
    sendBtn(m).click();
    await waitFor(() => !(q(m, '.retry-line') as HTMLElement).hidden);
    expect((q(m, '.composer textarea') as HTMLTextAreaElement).value).toBe('offline words');
  });
  it('403 read_only turns the composer into the read-only line', async () => {
    const s = withSpaces();
    s.spaceSend = () => ({ status: 403, body: { error: 'read_only', message: 'Read only.' } });
    const m = await mountList({ server: s });
    await openChannel(m);
    type(m, 'nope');
    sendBtn(m).click();
    await waitFor(() => !m.root.querySelector('.composer'));
    expect(hasText(m.root, "You can read this channel, but you can't write in it.")).toBe(true);
  });
  it('409 spaces_off on send drops the tab and goes home', async () => {
    const s = withSpaces();
    s.spaceSend = () => ({ status: 409, body: { error: 'spaces_off', message: 'off' } });
    const m = await mountList({ server: s });
    await openChannel(m);
    type(m, 'x');
    sendBtn(m).click();
    await waitFor(() => !!m.root.querySelector('[data-bot="b1"]'));
    expect(m.root.querySelector('.tabbar')).toBeNull();
  });
});

describe('Read-only channels', () => {
  it('show the reason instead of a composer', async () => {
    const m = await mountList({ server: withSpaces() });
    q(m, '[data-tab="spaces"]').click();
    await waitFor(() => !!m.root.querySelector('[data-space="s1"]'));
    q(m, '[data-space="s1"]').click();
    await waitFor(() => !!m.root.querySelector('[data-channel="c2"]'));
    q(m, '[data-channel="c2"]').click();
    await waitFor(() => !!m.root.querySelector('.readonly-note'));
    expect(m.root.querySelector('.composer')).toBeNull();
    expect(m.root.querySelector('.composer textarea')).toBeNull();
    expect(q(m, '.readonly-note').textContent).toBe("You can read this channel, but you can't write in it.");
    // the note is the last thing on the screen, where the composer would be
    const kids = Array.from(q(m, 'section.screen.channel').children);
    expect(kids[kids.length - 1].classList.contains('dock')).toBe(true);
  });
  it('a writable channel keeps the composer as the last thing, like the bot chat', async () => {
    const m = await mountList({ server: withSpaces() });
    await openChannel(m);
    const kids = Array.from(q(m, 'section.screen.channel').children) as HTMLElement[];
    expect(kids[0].classList.contains('head')).toBe(true);
    expect(kids[kids.length - 1].classList.contains('dock')).toBe(true);
    expect(kids[kids.length - 1].lastElementChild!.classList.contains('composer')).toBe(true);
    expect(kids.filter((k) => k.classList.contains('scroll')).length).toBe(1);
  });
});

describe('Spaces in Traditional Chinese', () => {
  it('uses zh strings for the tabs and the read-only line', async () => {
    const s = withSpaces();
    const m = await mountList({ server: s, lang: 'zh-TW' });
    await waitFor(() => !!m.root.querySelector('.tabbar'));
    expect(qa(m, '.tab-label').map((x) => x.textContent)).toEqual(['機器人', '空間']);
    q(m, '[data-tab="spaces"]').click();
    await waitFor(() => !!m.root.querySelector('[data-space="s1"]'));
    q(m, '[data-space="s1"]').click();
    await waitFor(() => !!m.root.querySelector('[data-channel="c2"]'));
    q(m, '[data-channel="c2"]').click();
    await waitFor(() => !!m.root.querySelector('.readonly-note'));
    expect(q(m, '.readonly-note').textContent).toBe('你可以閱讀這個頻道，但不能在這裡發言。');
  });
});
