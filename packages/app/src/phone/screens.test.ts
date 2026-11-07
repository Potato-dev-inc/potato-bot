// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { FakeServer, bot, chat, mount, mountList, openChat, waitFor, hasText, btn, sleep, LAN, TOKEN } from './harness';

const NOW = 1_700_000_000_000;
const q = (m: { root: HTMLElement }, sel: string) => m.root.querySelector(sel) as HTMLElement;
const qa = (m: { root: HTMLElement }, sel: string) => Array.from(m.root.querySelectorAll(sel)) as HTMLElement[];

function two() {
  const s = new FakeServer([LAN]);
  s.bots = [
    bot({ id: 'b1', name: 'Scout', state: 'needs_you', stateText: 'Needs you', lastLine: 'Can I run npm install?', unread: 3, openCards: 1 }),
    bot({ id: 'b2', name: 'Writer', state: 'working', stateText: 'Working', lastLine: 'Drafting the report', lastAt: NOW - 30_000 }),
    bot({ id: 'b3', name: 'Idle one', state: 'idle', lastLine: null, lastAt: null })
  ];
  s.chats.b1 = chat({ bot: s.bots[0] });
  return s;
}

describe('bot list', () => {
  it('shows name, last line, status dot, time, unread and the Needs you tag', async () => {
    const m = await mountList({ server: two() });
    const rows = qa(m, '.row');
    expect(rows.length).toBe(3);
    const r1 = rows[0];
    expect(r1.querySelector('.row-name')!.textContent).toBe('Scout');
    expect(r1.querySelector('.row-last')!.textContent).toBe('Can I run npm install?');
    expect(r1.querySelector('.dot')!.getAttribute('data-state')).toBe('needs_you');
    expect(r1.querySelector('.tag')!.textContent).toBe('Needs you');
    expect(r1.querySelector('.count')!.textContent).toBe('3');
    expect(r1.querySelector('.row-time')!.textContent).toBe('5m');
    expect(rows[1].querySelector('.dot')!.getAttribute('data-state')).toBe('working');
    expect(rows[1].querySelector('.tag')).toBeNull();
    expect(rows[1].querySelector('.row-time')!.textContent).toBe('now');
    expect(rows[2].querySelector('.row-last')!.textContent).toBe('No messages yet');
  });
  it('shows the PC name in the top bar', async () => {
    const m = await mountList({ server: two() });
    expect(q(m, '.pc-status').textContent).toBe('PC: Home PC');
  });
  it('shows an empty hint when there are no bots', async () => {
    const s = new FakeServer([LAN]);
    s.bots = [];
    const m = mount({ server: s });
    await waitFor(() => hasText(m.root, 'No bots yet. Make one on your PC.'));
  });
  it('does not list rooms unless groups is on', async () => {
    const s = two();
    s.rooms = [{ id: 'r1', name: 'Team', bots: ['Scout', 'Writer'], lastMs: NOW, unread: 0, typing: [] }];
    const m = await mountList({ server: s });
    expect(m.root.querySelector('[data-room]')).toBeNull();
  });
  it('lists rooms below the bots when groups is on and opens a room', async () => {
    const s = two();
    s.me.groups = true;
    s.rooms = [{ id: 'r1', name: 'Team', bots: ['Scout', 'Writer'], lastMs: NOW - 120000, unread: 2, typing: [] }];
    s.roomData.r1 = {
      room: { id: 'r1', name: 'Team', bots: ['Scout', 'Writer'] },
      lines: [
        { id: 'l1', from: { kind: 'person', name: 'Ren' }, text: 'Plan the launch', at: NOW - 60000 },
        { id: 'l2', from: { kind: 'bot', id: 'b1', name: 'Scout' }, text: 'On it', at: NOW - 30000 }
      ],
      work: [{ bot: 'Scout', run: { steps: [{ id: 's1', tool: 'search', status: 'done', detail: 'launch plans' }], endedMs: NOW } }],
      typing: ['Writer']
    };
    const m = await mountList({ server: s });
    expect(q(m, '[data-room="r1"] .row-name').textContent).toBe('Team');
    expect(q(m, '[data-room="r1"] .row-last').textContent).toBe('Scout, Writer');
    q(m, '[data-room="r1"]').click();
    await waitFor(() => hasText(m.root, 'Plan the launch'));
    expect(qa(m, '.msg').map((x) => x.getAttribute('data-from'))).toEqual(['person', 'bot']);
    expect(hasText(m.root, 'Writer typing')).toBe(true);
    expect(hasText(m.root, 'Scout: 1 steps')).toBe(true);
    expect(m.root.querySelector('.composer input[type=file]')).toBeNull();
    expect(m.root.querySelector('.cards')).toBeNull();
    expect(m.root.querySelector('.jobbar')).toBeNull();
    const ta = q(m, '.composer textarea') as HTMLTextAreaElement;
    ta.value = 'hello team';
    ta.dispatchEvent(new Event('input'));
    btn(m.root, 'Send').click();
    await waitFor(() => s.callsTo('/api/rooms/r1/send', 'POST').length === 1);
    expect(s.callsTo('/api/rooms/r1/send')[0].body).toEqual({ text: 'hello team' });
  });
  it('marks the chat read once per open, not on a live refetch', async () => {
    const s = two();
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => s.callsTo('/api/bots/b1/read', 'POST').length === 1);
    await waitFor(() => s.callsTo('/api/changes').length >= 1);
    const before = s.callsTo('/api/bots/b1').length;
    s.bump();
    await waitFor(() => s.callsTo('/api/bots/b1').length > before);
    await sleep(30);
    expect(s.callsTo('/api/bots/b1/read').length).toBe(1);
  });
  it('opens a chat when a row is tapped and goes back to the list', async () => {
    const m = await mountList({ server: two() });
    await openChat(m);
    expect(q(m, '.bar-title h1').textContent).toBe('Scout');
    (q(m, '[data-act="back"]') as HTMLElement).click();
    await waitFor(() => qa(m, '.row').length === 3);
  });
});

describe('pause all', () => {
  it('asks first, then pauses and shows the resume banner', async () => {
    const s = two();
    const m = await mountList({ server: s });
    q(m, '[data-act="pause"]').click();
    await waitFor(() => !!m.root.querySelector('.sheet'));
    expect(q(m, '.sheet h2').textContent).toBe('Pause all bots?');
    expect(s.callsTo('/api/pause-all').length).toBe(0);
    q(m, '.sheet [data-act="confirm"]').click();
    await waitFor(() => hasText(m.root, 'Paused - Start bots again'));
    expect(s.callsTo('/api/pause-all', 'POST').length).toBe(1);
    expect(m.root.querySelector('[data-act="pause"]')).toBeNull();
  });
  it('cancel does nothing', async () => {
    const s = two();
    const m = await mountList({ server: s });
    q(m, '[data-act="pause"]').click();
    await waitFor(() => !!m.root.querySelector('.sheet'));
    q(m, '.sheet [data-act="cancel"]').click();
    await waitFor(() => !m.root.querySelector('.sheet'));
    expect(s.callsTo('/api/pause-all').length).toBe(0);
  });
  it('Start bots again resumes', async () => {
    const s = two();
    s.paused = true;
    const m = await mountList({ server: s });
    await waitFor(() => hasText(m.root, 'Paused - Start bots again'));
    q(m, '[data-act="resume"]').click();
    await waitFor(() => !hasText(m.root, 'Paused - Start bots again'));
    expect(s.callsTo('/api/resume-all', 'POST').length).toBe(1);
  });
});

describe('chat', () => {
  const history = [
    { id: 'm1', at: NOW - 60000, from: { kind: 'you' }, kind: 'text', text: 'Find me a hotel' },
    { id: 'm2', at: NOW - 30000, from: { kind: 'bot', name: 'Scout' }, kind: 'text', text: 'Searching now' },
    { id: 'm3', at: NOW - 20000, from: { kind: 'note' }, kind: 'note', text: 'Scout joined' }
  ];
  function withChat(over: any = {}) {
    const s = two();
    s.chats.b1 = chat({ bot: s.bots[0], messages: history, ...over });
    return s;
  }

  it('shows the history', async () => {
    const m = await mountList({ server: withChat() });
    await openChat(m);
    await waitFor(() => hasText(m.root, 'Searching now'));
    expect(qa(m, '.msg').map((x) => x.className.replace('msg ', ''))).toEqual(['mine', 'theirs', 'note']);
    expect(hasText(m.root, 'Find me a hotel')).toBe(true);
  });
  it('sending a message POSTs it and shows it', async () => {
    const s = withChat();
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => hasText(m.root, 'Searching now'));
    const send = btn(m.root, 'Send');
    expect(send.disabled).toBe(true);
    const ta = q(m, '.composer textarea') as HTMLTextAreaElement;
    ta.value = 'Under $200 please';
    ta.dispatchEvent(new Event('input'));
    expect(send.disabled).toBe(false);
    send.click();
    await waitFor(() => s.callsTo('/api/bots/b1/send', 'POST').length === 1);
    expect(s.callsTo('/api/bots/b1/send')[0].body).toEqual({ text: 'Under $200 please', files: [] });
    await waitFor(() => qa(m, '.msg').filter((x) => x.textContent!.includes('Under $200 please')).length === 1);
    expect(ta.value).toBe('');
  });
  it('shows the message at once and puts the text back if sending fails', async () => {
    const s = withChat();
    s.errors['POST /api/bots/b1/send'] = { status: 500, error: 'boom', message: 'The PC is busy.' };
    const m = await mountList({ server: s });
    await openChat(m);
    const ta = q(m, '.composer textarea') as HTMLTextAreaElement;
    ta.value = 'retry me';
    ta.dispatchEvent(new Event('input'));
    btn(m.root, 'Send').click();
    await waitFor(() => hasText(m.root, 'The PC is busy.'));
    expect(ta.value).toBe('retry me');
    expect(qa(m, '.msg').some((x) => x.textContent!.includes('retry me'))).toBe(false);
  });
  it('shows a rate limit message from the server', async () => {
    const s = withChat();
    s.errors['POST /api/bots/b1/send'] = { status: 429, error: 'rate_limited', message: 'Slow down a little.' };
    const m = await mountList({ server: s });
    await openChat(m);
    const ta = q(m, '.composer textarea') as HTMLTextAreaElement;
    ta.value = 'x';
    ta.dispatchEvent(new Event('input'));
    btn(m.root, 'Send').click();
    await waitFor(() => q(m, '.toast').textContent === 'Slow down a little.');
  });
  function pick(m: any, files: File[]) {
    const input = q(m, 'input[type=file]') as HTMLInputElement;
    Object.defineProperty(input, 'files', { configurable: true, value: files });
    input.dispatchEvent(new Event('change'));
  }
  it('sends a photo as base64 with kind image', async () => {
    const s = withChat();
    const m = await mountList({ server: s });
    await openChat(m);
    const bytes = new Uint8Array([137, 80, 78, 71, 1, 2, 3]);
    pick(m, [new File([bytes], 'cat.png', { type: 'image/png' })]);
    await waitFor(() => hasText(m.root, 'cat.png'));
    btn(m.root, 'Send').click();
    await waitFor(() => s.callsTo('/api/bots/b1/send', 'POST').length === 1);
    const body = s.callsTo('/api/bots/b1/send')[0].body;
    expect(body.files).toEqual([{ name: 'cat.png', kind: 'image', data: Buffer.from(bytes).toString('base64') }]);
    expect(body.files[0].data).not.toContain('base64,');
  });
  it('allows at most 3 photos and a photo can be removed', async () => {
    const m = await mountList({ server: withChat() });
    await openChat(m);
    const f = (n: string) => new File([new Uint8Array([1])], n, { type: 'image/png' });
    pick(m, [f('a.png'), f('b.png'), f('c.png'), f('d.png')]);
    await waitFor(() => qa(m, '.chip').length === 3);
    expect(q(m, '.toast').textContent).toBe('You can add up to 3 photos.');
    (q(m, '.chip-x') as HTMLElement).click();
    expect(qa(m, '.chip').length).toBe(2);
  });
  it('a live reply appears after the changes loop returns a new rev', async () => {
    const s = withChat();
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => hasText(m.root, 'Searching now'));
    await waitFor(() => s.callsTo('/api/changes').length >= 1);
    s.chats.b1.messages.push({ id: 'm9', at: NOW, from: { kind: 'bot', name: 'Scout' }, kind: 'text', text: 'Found three hotels' });
    s.bump();
    await waitFor(() => hasText(m.root, 'Found three hotels'));
    // and it keeps polling with the new rev
    await waitFor(() => s.callsTo('/api/changes').some((c) => c.path.includes('since=' + s.rev)));
  });
  it('stops polling while hidden and polls again when visible', async () => {
    const s = withChat();
    const m = await mountList({ server: s });
    await waitFor(() => s.callsTo('/api/changes').length === 1);
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
    await waitFor(() => s.callsTo('/api/push/presence').some((c) => c.body.visible === false));
    s.bump();
    await sleep(60);
    const during = s.callsTo('/api/changes').length;
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
    document.dispatchEvent(new Event('visibilitychange'));
    await waitFor(() => s.callsTo('/api/changes').length > during);
    expect(s.callsTo('/api/push/presence').some((c) => c.body.visible === true)).toBe(true);
    m.app.destroy();
  });
  it('backs off when the changes call fails', async () => {
    const s = withChat();
    s.errors['GET /api/changes'] = { status: 500, error: 'boom', message: 'x' };
    const m = await mountList({ server: s });
    await sleep(150);
    expect(s.callsTo('/api/changes').length).toBeLessThanOrEqual(2);
    m.app.destroy();
  });
  it('shows a Queued badge on a queued message', async () => {
    const s = withChat({ messages: [...history, { id: 'm4', at: NOW, from: { kind: 'you' }, kind: 'text', text: 'Also check the weather', queued: true }] });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => hasText(m.root, 'Also check the weather'));
    const queued = qa(m, '.badge.queued');
    expect(queued.length).toBe(1);
    expect(queued[0].textContent).toBe('Queued');
    expect(queued[0].closest('.msg')!.textContent).toContain('Also check the weather');
  });
  it('shows the typing indicator only while typing', async () => {
    const s = withChat({ typing: true });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => !(q(m, '.typing') as HTMLElement).hidden);
    s.chats.b1.typing = false;
    s.bump();
    await waitFor(() => (q(m, '.typing') as HTMLElement).hidden);
  });
  it('shows live steps while working and folds them after', async () => {
    const steps = [
      { id: 's1', tool: 'browser', status: 'done', detail: 'open hotels.example' },
      { id: 's2', tool: 'search', status: 'running', detail: 'prices' },
      { id: 's3', tool: 'read', status: 'retry' },
      { id: 's4', tool: 'write', status: 'error', why: 'disk full' }
    ];
    const s = withChat({ work: { startedMs: NOW, steps, failed: false, endedMs: null } });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => qa(m, '.step').length === 4);
    const det = q(m, 'details.steps') as HTMLDetailsElement;
    expect(det.open).toBe(true);
    expect(det.querySelector('summary')!.textContent).toBe('Working... 4 steps');
    expect(qa(m, '.step').map((x) => x.getAttribute('data-status'))).toEqual(['done', 'running', 'retry', 'error']);
    expect(hasText(m.root, 'open hotels.example')).toBe(true);
    s.chats.b1.work = { startedMs: NOW, steps, failed: true, endedMs: NOW + 1 };
    s.bump();
    await waitFor(() => (q(m, 'details.steps') as HTMLDetailsElement).open === false);
    expect(q(m, 'details.steps summary').textContent).toBe('4 steps');
  });
  it('job bar shows goal, newest step and spend, and Stop asks first', async () => {
    const s = withChat({
      job: { goal: 'Plan the trip', step: 'Comparing prices', startedMs: NOW, spendNano: 1_250_000_000, state: 'running' }
    });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => !(q(m, '.jobbar') as HTMLElement).hidden);
    expect(q(m, '.job-goal').textContent).toBe('Plan the trip');
    expect(q(m, '.job-step').textContent).toBe('Comparing prices');
    expect(q(m, '.job-spend').textContent).toBe('Spent $1.25');
    q(m, '[data-act="stop"]').click();
    await waitFor(() => !!m.root.querySelector('.sheet'));
    expect(s.callsTo('/api/bots/b1/stop').length).toBe(0);
    q(m, '.sheet [data-act="confirm"]').click();
    await waitFor(() => s.callsTo('/api/bots/b1/stop', 'POST').length === 1);
  });
  it('hides the job bar for a finished job', async () => {
    const s = withChat({ job: { goal: 'Old', step: '', startedMs: NOW, spendNano: 0, state: 'done' } });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => hasText(m.root, 'Searching now'));
    expect((q(m, '.jobbar') as HTMLElement).hidden).toBe(true);
  });
  it('renders bot text as plain text, never as HTML', async () => {
    const evil = '<img src=x onerror=alert(1)><script>window.__pwned=1</script>';
    const s = withChat({ messages: [{ id: 'x1', at: NOW, from: { kind: 'bot', name: '<b>Scout</b>' }, kind: 'text', text: evil, file: { name: '<i>f</i>', kind: 'image', size: 1 } }] });
    s.bots[0].lastLine = evil;
    const m = await mountList({ server: s });
    expect(q(m, '.row-last').textContent).toBe(evil);
    await openChat(m);
    await waitFor(() => hasText(m.root, evil));
    expect(m.root.querySelector('img')).toBeNull();
    expect(m.root.querySelector('script')).toBeNull();
    expect(m.root.querySelector('b')).toBeNull();
    expect(q(m, '.txt').textContent).toBe(evil);
    expect((globalThis as any).__pwned).toBeUndefined();
  });
  it('does not turn URLs into links', async () => {
    const s = withChat({ messages: [{ id: 'x1', at: NOW, from: { kind: 'bot', name: 'Scout' }, kind: 'text', text: 'see https://evil.example/a' }] });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => hasText(m.root, 'https://evil.example/a'));
    expect(m.root.querySelector('a')).toBeNull();
  });
});

describe('cards', () => {
  const answerPath = '/api/bots/b1/cards/c1/answer';
  async function withCard(card: any) {
    const s = two();
    s.chats.b1 = chat({ bot: s.bots[0], cards: [{ id: 'c1', kind: 'question', action: '', why: '', choices: [], state: 'open', keepGoing: null, connector: null, login: null, mcpAdd: null, canAlwaysAllow: false, ...card }] });
    const m = await mountList({ server: s });
    await openChat(m);
    await waitFor(() => !!m.root.querySelector('.card'));
    return { s, m };
  }
  const labels = (m: any) => qa(m, '.card button').map((b) => b.textContent);

  it('approval: three buttons that post allow_once, allow_job and deny (never Always allow)', async () => {
    const { s, m } = await withCard({ kind: 'approval', action: 'Run npm install', why: 'To build the site', choices: ['Allow once', 'Always allow', 'Deny'], canAlwaysAllow: true });
    expect(labels(m)).toEqual(['Allow once', 'Allow for this job', 'Deny']);
    expect(hasText(m.root, 'Run npm install')).toBe(true);
    expect(hasText(m.root, 'Why: To build the site')).toBe(true);
    expect(hasText(m.root, 'Always')).toBe(false);
    btn(m.root, 'Allow for this job').click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer: 'allow_job' });
    await waitFor(() => !m.root.querySelector('.card'));
  });
  it.each([['Allow once', 'allow_once'], ['Deny', 'deny']])('approval %s posts %s', async (label, answer) => {
    const { s, m } = await withCard({ kind: 'approval', action: 'x', choices: [] });
    btn(m.root, label).click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer });
  });
  it('question: one button per choice, the answer is the index', async () => {
    const { s, m } = await withCard({ kind: 'question', action: 'Which city?', choices: ['Paris', 'Rome', 'Oslo'] });
    expect(labels(m)).toEqual(['Paris', 'Rome', 'Oslo']);
    btn(m.root, 'Oslo').click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer: '2' });
  });
  it('keep going card shows its choices', async () => {
    const { s, m } = await withCard({ kind: 'question', action: 'Still working', keepGoing: 'The bot used 40 steps. Keep going?', choices: ['Keep going', 'Stop'] });
    expect(labels(m)).toEqual(['Keep going', 'Stop']);
    expect(hasText(m.root, 'The bot used 40 steps. Keep going?')).toBe(true);
    btn(m.root, 'Stop').click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer: '1' });
  });
  it('connector card hides the first choice and keeps the original indices', async () => {
    const { s, m } = await withCard({ kind: 'question', action: 'Needs Gmail', connector: 'Gmail', choices: ['Open settings', "I'm done", 'Skip'] });
    expect(hasText(m.root, 'Connect Gmail on your PC')).toBe(true);
    expect(labels(m)).toEqual(["I'm done", 'Skip']);
    btn(m.root, 'Skip').click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer: '2' });
  });
  it('mcp_add card shows its choices as buttons', async () => {
    const { s, m } = await withCard({ kind: 'question', action: 'Add a tool', mcpAdd: { name: 'files', url: 'https://mcp.example/files' }, choices: ['Add it', 'No'] });
    expect(hasText(m.root, 'Add tool: files')).toBe(true);
    expect(hasText(m.root, 'https://mcp.example/files')).toBe(true);
    expect(labels(m)).toEqual(['Add it', 'No']);
    btn(m.root, 'Add it').click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer: '0' });
  });
  it('sign-in card: no inputs, says Sign in on your PC, only offers Not now', async () => {
    const { s, m } = await withCard({ kind: 'question', action: 'Sign in to Shop', login: 'https://shop.example', choices: ['Sign in on PC', 'Not now'] });
    const card = q(m, '.card');
    expect(card.querySelector('input, textarea, select')).toBeNull();
    expect(hasText(card, 'Sign in on your PC')).toBe(true);
    expect(hasText(card, 'https://shop.example')).toBe(true);
    expect(labels(m)).toEqual(['Not now']);
    btn(m.root, 'Not now').click();
    await waitFor(() => s.callsTo(answerPath, 'POST').length === 1);
    expect(s.callsTo(answerPath)[0].body).toEqual({ answer: '1' });
  });
  it('sign-in card without a Not now choice has no buttons at all', async () => {
    const { m } = await withCard({ kind: 'question', login: 'https://shop.example', choices: ['Sign in on PC'] });
    expect(qa(m, '.card button').length).toBe(0);
  });
  it('a failed answer keeps the card and shows the error', async () => {
    const { s, m } = await withCard({ kind: 'approval', action: 'x', choices: [] });
    s.errors['POST ' + answerPath] = { status: 409, error: 'gone', message: 'Someone already answered.' };
    btn(m.root, 'Deny').click();
    await waitFor(() => hasText(m.root, 'Someone already answered.'));
    expect(m.root.querySelector('.card')).not.toBeNull();
    expect(btn(m.root, 'Deny').disabled).toBe(false);
  });
});

describe('secrets never pass through the phone', () => {
  const bad = (root: HTMLElement) => {
    expect(root.querySelector('input[type=password]')).toBeNull();
    for (const i of Array.from(root.querySelectorAll('input, textarea'))) {
      const id = [i.getAttribute('name'), i.getAttribute('id'), i.getAttribute('autocomplete'), i.getAttribute('type')].join(' ');
      expect(id).not.toMatch(/user|pass|login|email|otp|secret/i);
    }
  };
  it('no password or user field on the pair, unreachable, list, chat (with a sign-in card) and settings screens', async () => {
    const m0 = mount({ stored: null });
    await waitFor(() => hasText(m0.root, 'Open the pairing QR'));
    bad(m0.root);

    const dead = new FakeServer([]);
    const m1 = mount({ server: dead });
    await waitFor(() => hasText(m1.root, 'cannot reach your PC'));
    bad(m1.root);

    const s = two();
    s.chats.b1 = chat({ bot: s.bots[0], cards: [{ id: 'c1', kind: 'question', action: 'Sign in', why: '', choices: ['Sign in on PC', 'Not now'], state: 'open', keepGoing: null, connector: null, login: 'https://shop.example', mcpAdd: null, canAlwaysAllow: false }] });
    const m = await mountList({ server: s });
    bad(m.root);
    q(m, '[data-act="settings"]').click();
    await waitFor(() => !!m.root.querySelector('.sheet'));
    bad(m.root);
    q(m, '.sheet .btn:not(.danger-text)').click();
    await waitFor(() => !m.root.querySelector('.sheet'));
    await openChat(m);
    await waitFor(() => !!m.root.querySelector('.card'));
    bad(m.root);
  });
});

describe('settings', () => {
  it('shows this phone and forgets it: DELETE /api/me, storage cleared, pair screen', async () => {
    const s = two();
    const m = await mountList({ server: s });
    q(m, '[data-act="settings"]').click();
    await waitFor(() => !!m.root.querySelector('.sheet'));
    expect(q(m, '.v-name').textContent).toBe('iPhone');
    expect(q(m, '.v-pc').textContent).toBe('Home PC');
    q(m, '[data-act="forget"]').click();
    await waitFor(() => m.root.querySelectorAll('.sheet').length === 2);
    q(m, '.sheet [data-act="confirm"]').click();
    await waitFor(() => hasText(m.root, 'Open the pairing QR'));
    expect(s.callsTo('/api/me', 'DELETE').length).toBe(1);
    expect(m.storage.data.size).toBe(0);
    expect(m.root.querySelector('.sheet')).toBeNull();
  });
  it('says notifications are unavailable when the browser cannot do push', async () => {
    const m = await mountList({ server: two() });
    q(m, '[data-act="settings"]').click();
    await waitFor(() => hasText(m.root, 'This browser cannot show notifications.'));
    expect((q(m, '.switch') as HTMLButtonElement).disabled).toBe(true);
  });
  it('turns notifications on and off through the push endpoints', async () => {
    (window as any).Notification = { requestPermission: vi.fn(async () => 'granted') };
    let current: any = null;
    const sub = { toJSON: () => ({ endpoint: 'https://push.example/abc', keys: { p256dh: 'PKEY', auth: 'AKEY' } }), unsubscribe: vi.fn(async () => true) };
    const subscribe = vi.fn(async (_o: any) => { current = sub; return sub; });
    const reg = { pushManager: { getSubscription: async () => current, subscribe } };
    const s = two();
    const m = mount({ server: s, nav: { serviceWorker: { ready: Promise.resolve(reg), register: async () => reg } } });
    await waitFor(() => !!m.root.querySelector('.row'));
    q(m, '[data-act="settings"]').click();
    await waitFor(() => !(q(m, '.switch') as HTMLButtonElement).disabled);
    q(m, '.switch').click();
    await waitFor(() => s.callsTo('/api/push/subscribe', 'POST').length === 1);
    expect(s.callsTo('/api/push/key').length).toBe(1);
    expect(s.callsTo('/api/push/subscribe')[0].body).toEqual({ endpoint: 'https://push.example/abc', keys: { p256dh: 'PKEY', auth: 'AKEY' } });
    expect(subscribe.mock.calls[0][0]).toMatchObject({ userVisibleOnly: true });
    await waitFor(() => q(m, '.switch').getAttribute('aria-checked') === 'true');
    q(m, '.switch').click();
    await waitFor(() => s.callsTo('/api/push/subscribe', 'DELETE').length === 1);
    await waitFor(() => sub.unsubscribe.mock.calls.length === 1);
    delete (window as any).Notification;
  });
  it('explains when notifications are blocked', async () => {
    (window as any).Notification = { requestPermission: async () => 'denied' };
    const reg = { pushManager: { getSubscription: async () => null, subscribe: vi.fn() } };
    const s = two();
    const m = mount({ server: s, nav: { serviceWorker: { ready: Promise.resolve(reg), register: async () => reg } } });
    await waitFor(() => !!m.root.querySelector('.row'));
    q(m, '[data-act="settings"]').click();
    await waitFor(() => !(q(m, '.switch') as HTMLButtonElement).disabled);
    q(m, '.switch').click();
    await waitFor(() => hasText(m.root, 'Notifications are blocked.'));
    expect(s.callsTo('/api/push/subscribe').length).toBe(0);
    delete (window as any).Notification;
  });
  it('posts presence on start', async () => {
    const s = two();
    await mountList({ server: s });
    await waitFor(() => s.callsTo('/api/push/presence', 'POST').length >= 1);
    expect(s.callsTo('/api/push/presence')[0].body).toEqual({ visible: true });
  });
});

describe('language', () => {
  it('uses English for en-US', async () => {
    const m = await mountList({ server: two(), lang: 'en-US' });
    expect(hasText(m.root, 'Pause all')).toBe(true);
    expect(document.documentElement.lang).toBe('en');
  });
  it.each(['zh-TW', 'zh-Hant', 'zh-HK', 'zh-CN'])('uses Traditional Chinese for %s', async (lang) => {
    const m = await mountList({ server: two(), lang });
    expect(hasText(m.root, '全部暫停')).toBe(true);
    expect(q(m, '.tag').textContent).toBe('需要你');
    expect(hasText(m.root, 'Pause all')).toBe(false);
    expect(document.documentElement.lang).toBe('zh-Hant');
  });
  it('translates the pair screen and the cards', async () => {
    const m = mount({ stored: null, lang: 'zh-TW' });
    await waitFor(() => hasText(m.root, '請在電腦上開啟配對 QR'));
    const s = two();
    s.chats.b1 = chat({ bot: s.bots[0], cards: [{ id: 'c1', kind: 'approval', action: 'x', why: '', choices: [], state: 'open', keepGoing: null, connector: null, login: null, mcpAdd: null, canAlwaysAllow: false }] });
    const m2 = await mountList({ server: s, lang: 'zh-TW' });
    await openChat(m2);
    await waitFor(() => !!m2.root.querySelector('.card'));
    expect(qa(m2, '.card button').map((b) => b.textContent)).toEqual(['允許一次', '這項工作都允許', '拒絕']);
  });
  it('never shows a raw i18n key', async () => {
    const m = await mountList({ server: two() });
    await openChat(m);
    expect(m.root.textContent).not.toMatch(/\b(common|list|chat|card|job|step|settings|err|pair|unreach|pause)\.[a-z_]+/);
  });
});

describe('misc', () => {
  it('never leaks the token into the DOM, even in settings', async () => {
    const m = await mountList({ server: two() });
    q(m, '[data-act="settings"]').click();
    await waitFor(() => !!m.root.querySelector('.sheet'));
    expect(document.body.innerHTML).not.toContain(TOKEN);
  });
  it('opens straight into a chat from a notification link', async () => {
    const s = two();
    const m = mount({ server: s, hash: '#/bot/b1' });
    await waitFor(() => !!m.root.querySelector('.composer'));
    await waitFor(() => q(m, '.bar-title h1').textContent === 'Scout');
  });
  it('tap targets: buttons are real buttons, textarea has a label', async () => {
    const m = await mountList({ server: two() });
    await openChat(m);
    expect((q(m, 'textarea') as HTMLElement).getAttribute('aria-label')).toBe('Message');
    expect(qa(m, 'button.icon-btn').every((b) => b.getAttribute('aria-label'))).toBe(true);
  });
});
