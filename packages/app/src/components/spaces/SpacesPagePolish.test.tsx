// @vitest-environment jsdom
// SpacesPage level proof for the last design pass: the empty state, the quiet rail, the Leave space menu, the icon Attach,
// runs of lines from one sender, the Bots here strip stepping aside, the member menu, and the plain-word states.
// What jsdom cannot show: how any of it looks, hover reveal, and the fade at narrow width. Those are CSS; the guard tests at
// the end only check that the rules are written down, they do not prove the pixels.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceDetail, SpaceDetailBot, SpaceMessage, SpacesView } from '../../lib/spaces';

const state = vi.hoisted(() => ({
  view: null as SpacesView | null,
  detail: null as SpaceDetail | null,
  messages: [] as SpaceMessage[],
  error: null as string | null,
  invite: vi.fn(async (_space: string) => null as unknown),
  cancelInvite: vi.fn(async (_space: string, _invite: string) => {}),
  send: vi.fn(async (_text: string, _replyTo: string | null) => null as string | null),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return {
    ...orig,
    useSpaces: () => ({
      view: state.view, error: state.error, create: async () => 's9', invite: state.invite, cancelInvite: state.cancelInvite,
      join: async () => true, answer: async () => {}, cancelJoin: async () => {},
    }),
    useSpace: () => ({ detail: state.detail, error: null, reload: () => {} }),
    useChannel: () => ({ messages: state.messages, atStart: true, loadOlder: async () => {}, send: state.send, error: null }),
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

import { SpacesPage } from './SpacesPage';

let host: HTMLDivElement;
let root: Root;
const hoot = (over: Partial<SpaceDetailBot> = {}): SpaceDetailBot => ({
  id: 'b1', name: 'Ren的Hoot', ownerName: 'Ren', mine: true, muted: false, silenced: false, dayCap: 30, usedToday: 0, refused: null, ...over,
});
const cat = (over: Partial<SpaceDetailBot> = {}): SpaceDetailBot => ({
  id: 'b2', name: 'Sam的Cat', ownerName: 'Sam', mine: false, muted: false, ...over,
});
const detail = (over: Partial<SpaceDetail> = {}): SpaceDetail => ({
  id: 's1', name: 'Owl club', isOwner: true, myMember: 'm1', ownerName: 'Ren',
  channels: [{ id: 'c1', name: 'general', archived: false, unread: 0 }],
  members: [{ member: 'm1', name: 'Me', role: 'owner', presence: 'online', blocked: false, isMe: true, short: 'AB12-CD34' }],
  openInvites: 0, relayed: false, anyoneOnline: true, state: 'active', leavePending: false,
  bots: [hoot(), cat()], botRules: { perPerson: 3, channels: null }, ...over,
});
const message = (over: Partial<SpaceMessage> = {}): SpaceMessage => ({
  id: 'x1', channel: 'c1', member: 'm1', authorName: 'Me', isFish: false, isMine: true, createdAtMs: 0, text: 'hello',
  replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false, ...over,
});

const box = () => document.querySelector('textarea') as HTMLTextAreaElement;
const options = () => [...document.querySelectorAll('[role="option"]')].map((o) => o.querySelector('.ck-slash-name')?.textContent);
const text = () => document.body.textContent ?? '';
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) as HTMLButtonElement | undefined;
const typeLine = (value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  act(() => {
    setter?.call(box(), value);
    box().setSelectionRange(value.length, value.length);
    box().dispatchEvent(new Event('input', { bubbles: true }));
  });
};
const press = (el: Element, key: string) => act(async () => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
});
const click = async (el: Element | null | undefined) => {
  if (!el) throw new Error('nothing to click');
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};
async function setUp(over: Partial<SpaceDetail> = {}, messages: SpaceMessage[] = []) {
  state.view = {
    spaces: [{ id: 's1', name: 'Owl club', isOwner: true, unread: 0, members: 1, online: 0 }],
    requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true,
  };
  state.detail = detail(over);
  state.messages = messages;
  await act(async () => root.render(<SpacesPage />));
  await act(async () => {});
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.error = null;
  state.invite.mockReset();
  state.invite.mockImplementation(async () => null);
  state.cancelInvite.mockReset();
  state.send.mockReset();
  state.send.mockImplementation(async () => null);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const byLabel = (label: string) => document.querySelector(`[aria-label="${label}"]`) as HTMLButtonElement | null;
const minute = 60_000;
const sam = { member: 'm2', name: 'Sam', role: 'member' as const, presence: 'online' as const, blocked: false, isMe: false, short: 'EF56-GH78' };

describe('empty state', () => {
  it('says what a space is and makes New space the first action', async () => {
    state.view = { spaces: [], requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true };
    await act(async () => root.render(<SpacesPage />));
    expect(text()).toContain('Chat with friends in small groups, computer to computer. No server keeps your messages.');
    expect(text()).toContain("Messages live on members' computers. If everyone is offline, new messages wait.");
    expect(button('New space')?.classList.contains('ck-btn--primary')).toBe(true);
    expect(button('Join a space')?.classList.contains('ck-btn--secondary')).toBe(true);
    expect(document.querySelector('.ck-spaces-empty-card')).not.toBeNull();
  });
  it('still shows the card, with the reason quietly beneath, when the view could not be loaded (service not up yet)', async () => {
    state.view = null;
    state.error = "Spaces can't go online right now. Check your internet connection.";
    await act(async () => root.render(<SpacesPage />));
    expect(document.querySelector('.ck-spaces-empty-card')).not.toBeNull();
    expect(button('New space')).toBeDefined();
    expect(button('Join a space')).toBeDefined();
    expect(text()).not.toContain('Pick a space on the left.');
    expect(document.querySelector('.ck-spaces-empty-card .ck-spaces-hint')?.textContent).toBe(state.error);
  });
  it('says offline beneath the card when the view loaded but the network is off', async () => {
    state.view = { spaces: [], requests: [], joining: [], invites: [], online: false, relayed: false, showOnline: true };
    await act(async () => root.render(<SpacesPage />));
    expect(document.querySelector('.ck-spaces-empty-card .ck-spaces-offline')).not.toBeNull();
    expect(button('New space')).toBeDefined();
  });
});

describe('Invite people', () => {
  const open = (id: string, ms = 3_600_000) => ({ id, space: 's1', link: '', words: [], qr: '', expiresAtMs: Date.now() + ms, state: 'open' as const });
  const made = { ...open('i-new'), link: 'pm://x', words: ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] };
  it('makes one invite when none is open', async () => {
    await setUp();
    state.invite.mockImplementation(async () => made);
    await click(button('Invite people'));
    expect(state.invite).toHaveBeenCalledTimes(1);
    expect(text()).toContain('Invite link');
  });
  it('shows the open invite instead of making another, and says the words cannot be shown again', async () => {
    await setUp();
    state.view = { ...state.view!, invites: [open('i1')] };
    await act(async () => root.render(<SpacesPage />));
    await click(button('Invite people'));
    expect(state.invite).not.toHaveBeenCalled();
    expect(text()).toContain('Open invites for Owl club');
    expect(text()).toContain("can't show its link and 8 words again");
    expect(button('Show link and words')).toBeUndefined();
    await click(button('Cancel invite'));
    expect(state.cancelInvite).toHaveBeenCalledWith('s1', 'i1');
    expect(button('New invite')?.disabled).toBe(false);
  });
  it('three open invites: the message is inside the sheet, New invite is off, each can be cancelled there', async () => {
    await setUp();
    state.view = { ...state.view!, invites: [open('i1'), open('i2'), open('i3')] };
    await act(async () => root.render(<SpacesPage />));
    await click(button('Invite people'));
    const sheet = document.querySelector('.ck-spaces-sheet')!;
    expect(sheet.textContent).toContain('Three invites are already open. Cancel one first.');
    expect(button('New invite')?.disabled).toBe(true);
    expect([...sheet.querySelectorAll('button')].filter((b) => b.textContent === 'Cancel invite')).toHaveLength(3);
    expect(state.invite).not.toHaveBeenCalled();
  });
  it('an invite made in this window can be shown again from the list', async () => {
    await setUp();
    state.invite.mockImplementation(async () => made);
    await click(button('Invite people'));
    await click(button('Done'));
    state.view = { ...state.view!, invites: [made] };
    await act(async () => root.render(<SpacesPage />));
    await click(button('Invite people'));
    expect(state.invite).toHaveBeenCalledTimes(1);
    await click(button('Show link and words'));
    expect(text()).toContain('Invite link');
  });
});

describe('rail and channel list', () => {
  it('the two ways in are quiet rows under the list, and the channels have a heading', async () => {
    await setUp();
    const actions = document.querySelector('.ck-spaces-rail-actions')!;
    expect([...actions.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['New space', 'Join a space']);
    expect(actions.querySelector('.ck-spaces-btn')).toBeNull();
    expect(document.querySelector('.ck-spaces-channels h2')?.textContent).toBe('Channels');
  });
});

describe('Leave space menu', () => {
  it('is one small menu: closed it is not in the header, open it holds Leave space, Escape closes it', async () => {
    await setUp({ isOwner: false });
    expect(button('Leave space')).toBeUndefined();
    const more = byLabel('Space actions')!;
    await click(more);
    expect(button('Leave space')?.disabled).toBe(false);
    await press(more, 'Escape');
    expect(button('Leave space')).toBeUndefined();
    expect(document.activeElement).toBe(more);
  });

  it('for the owner Leave stays off and the reason is written in the menu and on the item', async () => {
    await setUp();
    await click(byLabel('Space actions'));
    expect(button('Leave space')?.disabled).toBe(true);
    expect(button('Leave space')?.title).toBe("You own this space. Owners can't leave in this version.");
    expect(document.querySelector('.ck-spaces-menu-note')?.textContent).toBe("You own this space. Owners can't leave in this version.");
  });

  it('a space I left has no menu', async () => {
    await setUp({ state: 'left' });
    expect(byLabel('Space actions')).toBeNull();
  });
});

describe('composer', () => {
  it('Attach file is the Potato paperclip button with the same name', async () => {
    await setUp();
    const attach = byLabel('Attach file')!;
    expect(attach.classList.contains('ck-potato-round')).toBe(true);
    expect(attach.textContent).toBe('');
    expect(attach.querySelector('svg')).not.toBeNull();
    expect(attach.title).toBe('Attach file');
  });

  it('says Sending… while a line goes out', async () => {
    let done: (v: string | null) => void = () => {};
    state.send.mockImplementation(() => new Promise<string | null>((r) => { done = r; }));
    await setUp();
    typeLine('hello');
    await click(button('Send'));
    expect(document.querySelector('[role="status"]')?.textContent).toBe('Sending…');
    await act(async () => done(null));
    expect(text()).not.toContain('Sending…');
  });

  it('a failed send keeps the text and says so in words', async () => {
    state.send.mockImplementation(async () => 'Could not send. Try again.');
    await setUp();
    typeLine('hello');
    await click(button('Send'));
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('Could not send. Try again.');
    expect(text()).toContain('Your message is still in the box. Press Enter to try again.');
    expect(box().value).toBe('hello');
  });

  it('a space I left says the channel is read-only', async () => {
    await setUp({ state: 'removed' }, [message()]);
    expect(box()).toBeNull();
    expect(text()).toContain("You can read this channel, but you can't write in it.");
  });
});

describe('runs of lines from one sender', () => {
  const rows = () => [...document.querySelectorAll('.ck-spaces-msg')];
  it('groups close lines from the same sender and keeps time and More reachable', async () => {
    await setUp({}, [
      message({ id: 'a', createdAtMs: 0, text: 'one' }),
      message({ id: 'b', createdAtMs: minute, text: 'two' }),
      message({ id: 'c', createdAtMs: 2 * minute, text: 'three' }),
      message({ id: 'd', createdAtMs: 20 * minute, text: 'later' }),
      message({ id: 'e', createdAtMs: 21 * minute, member: 'm2', authorName: 'Sam', isMine: false, text: 'sam' }),
      message({ id: 'f', createdAtMs: 21.5 * minute, member: 'm2', authorName: 'Sam', isMine: false, text: 'reply', replyPreview: { authorName: 'Me', text: 'x' } }),
    ]);
    expect(rows().map((r) => r.classList.contains('is-grouped'))).toEqual([false, true, true, false, false, false]);
    for (const r of rows().slice(1, 3)) {
      expect(r.querySelector('time')).not.toBeNull();
      expect(r.querySelector('.ck-spaces-msg-actions')).not.toBeNull();
      expect(r.querySelector('.ck-spaces-msg-author')?.textContent).toBe('Me');
    }
  });

  it('a bot is not grouped with a person, and a deleted line breaks the run', async () => {
    await setUp({}, [
      message({ id: 'a', createdAtMs: 0 }),
      message({ id: 'b', createdAtMs: 1000, isFish: true, botId: 'b1', authorName: 'Ren的Hoot', isMine: false }),
      message({ id: 'c', createdAtMs: 2000, isFish: true, botId: 'b1', authorName: 'Ren的Hoot', isMine: false }),
      message({ id: 'd', createdAtMs: 3000, isFish: true, botId: 'b1', authorName: 'Ren的Hoot', isMine: false, deleted: { byName: 'Ren', byOwner: false } }),
    ]);
    expect(rows().map((r) => r.classList.contains('is-grouped'))).toEqual([false, false, true, false]);
  });
});

describe('Bots here strip', () => {
  it('is marked as repeating the People list, so wide windows can drop it; the words stay', async () => {
    await setUp();
    const line = document.querySelector('.ck-spaces-botsline')!;
    expect(line.textContent).toBe('Bots here: Ren的Hoot · Sam的Cat');
    expect(line.classList.contains('is-listed')).toBe(true);
  });
  it('the Bots are off line is never dropped', async () => {
    await setUp({ botRules: { perPerson: 3, channels: ['other'] } });
    const line = document.querySelector('.ck-spaces-botsline')!;
    expect(line.textContent).toBe('Bots are off in this channel.');
    expect(line.classList.contains('is-listed')).toBe(false);
  });
});

describe('member menu', () => {
  it('More matches the message one: the row reveals it, Escape closes it and returns focus', async () => {
    await setUp({ members: [...detail().members, sam] });
    const more = byLabel('Actions for Sam')!;
    expect(more.classList.contains('ck-spaces-more')).toBe(true);
    expect(more.closest('.ck-spaces-member-actions')).not.toBeNull();
    await click(more);
    expect(document.querySelector('.ck-spaces-members [role="menu"]')).not.toBeNull();
    await press(more, 'Escape');
    expect(document.querySelector('.ck-spaces-members [role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(more);
  });
});

describe('offline', () => {
  it('says it in a plain status line', async () => {
    await setUp();
    state.view = { ...state.view!, online: false };
    await act(async () => root.render(<SpacesPage />));
    const line = document.querySelector('.ck-spaces-offline')!;
    expect(line.getAttribute('role')).toBe('status');
    expect(line.textContent).toBe("You're offline. New messages will go out when you're back online.");
  });
});

describe('CSS guards (rules written down; the pixels are not proven here)', () => {
  const css = readFileSync(resolve(__dirname, '../../styles/cockpit-spaces.css'), 'utf8');
  it('narrow strips fade at the edge and keep the actions out of the scrolling list', () => {
    expect(css).toMatch(/mask-image: linear-gradient\(to right/);
    expect(css).toMatch(/\.ck-spaces-rail-actions \{ margin: 0;/);
  });
  it('sheets are sized against the page area, not the window, and every scroller is thin', () => {
    expect(css).toMatch(/\.ck-scrim:has\(> \.ck-spaces-sheet\) \{ grid-template-columns: minmax\(0, 1fr\)/);
    expect(css).toMatch(/\.ck-sheet\.ck-spaces-sheet \{ width: min\(400px, 100%\)/);
    expect(css).toMatch(/\.ck-spaces-msgs::-webkit-scrollbar-button[^}]*display: none/);
  });
  it('the Bots here line steps aside on wide windows, grouped lines reveal time on hover and focus', () => {
    expect(css).toMatch(/min-width: 1001px\) \{ \.cockpit \.ck-spaces-botsline\.is-listed \{ display: none/);
    expect(css).toMatch(/is-grouped:focus-within \.ck-spaces-msg-time/);
    expect(css).toMatch(/ck-spaces-member:focus-within \.ck-spaces-member-actions/);
  });
});
