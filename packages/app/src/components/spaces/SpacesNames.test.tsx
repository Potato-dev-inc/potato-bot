// @vitest-environment jsdom
// Names in Spaces: a bot's line is never "mine" and shows the bot's name, the page asks for my name before the first
// space is made or joined, and the Settings row for it is drawn.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceMessage, SpacesView } from '../../lib/spaces';

const state = vi.hoisted(() => ({
  view: null as SpacesView | null,
  messages: [] as unknown[],
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => null as unknown),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: state.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return {
    ...orig,
    useSpaces: () => ({
      view: state.view, error: null, create: async () => 's9', invite: async () => null, cancelInvite: async () => {},
      join: async () => true, answer: async () => {}, cancelJoin: async () => {},
    }),
    useSpace: () => ({ detail: null, error: null, reload: () => {} }),
    useChannel: () => ({ messages: state.messages, atStart: true, loadOlder: async () => {}, send: async () => null, error: null }),
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

import { SpacesPage } from './SpacesPage';
import { MessageRow } from './MessageRow';
import { ChannelView } from './ChannelView';
import { spacesRows } from '../settings/sections/SpacesRows';
import { mountRows, settle, type Mounted } from '../settings/test-utils';

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.view = null;
  state.messages = [];
  state.invoke.mockReset();
  state.invoke.mockImplementation(async () => null);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const msg = (over: Partial<SpaceMessage> = {}): SpaceMessage => ({
  id: 'm1', channel: 'c1', member: 'mem-ren', authorName: 'Ren', isFish: false, isMine: false, createdAtMs: 0, text: 'hi',
  replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false, ...over,
});
const view = (over: Partial<SpacesView> = {}): SpacesView => ({
  spaces: [], requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true, ...over,
});
const button = (label: string) => Array.from(host.querySelectorAll('button')).find((b) => b.textContent === label) as HTMLButtonElement | undefined;
const typeInto = (input: HTMLInputElement, value: string) => {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
};

describe('a bot line', () => {
  it('is not mine even when the PC says so, and shows the bot name, a Bot tag and the owner', () => {
    // The owner's own PC used to say isMine for its bot, because the line is signed with the owner's key.
    const bot = msg({ id: 'b1', authorName: 'Ren的fish', isFish: true, botId: 'bot1', isMine: true, ownerName: 'Ren', ownedByMe: true, text: 'hello' });
    const person = msg({ id: 'p1', authorName: 'Ren', isMine: true, text: 'hi bot' });
    act(() => root.render(<ol><MessageRow message={person} onReply={() => {}} /><MessageRow message={bot} onReply={() => {}} /></ol>));
    const [mine, theirs] = Array.from(host.querySelectorAll('li.ck-spaces-msg'));
    expect(mine.getAttribute('data-mine')).toBe('true');
    expect(theirs.hasAttribute('data-mine')).toBe(false);
    expect(theirs.classList.contains('is-bot')).toBe(true);
    expect(theirs.querySelector('.ck-spaces-msg-author')?.textContent).toBe('Ren的fish');
    expect(theirs.querySelector('.ck-spaces-bot-tag')?.textContent).toBe('Bot');
    expect(theirs.querySelector('.ck-spaces-msg-owner')?.textContent).toBe('yours');
    expect(theirs.querySelector('.ck-potato-bubble--you')).toBeNull();
  });

  it("shows the owner's name on someone else's bot", () => {
    const bot = msg({ authorName: 'Sam的cat', isFish: true, botId: 'bot2', ownerName: 'Sam', ownedByMe: false });
    act(() => root.render(<ol><MessageRow message={bot} onReply={() => {}} /></ol>));
    expect(host.querySelector('.ck-spaces-msg-owner')?.textContent).toBe("Sam's");
  });

  it('shows the sender on the first row of a run in the feed, not only on hover', () => {
    state.messages = [
      msg({ id: 'a', authorName: 'Sam', member: 'mem-sam', createdAtMs: 1000 }),
      msg({ id: 'b', authorName: 'Sam', member: 'mem-sam', createdAtMs: 2000 }),
      msg({ id: 'c', authorName: 'Ren的fish', isFish: true, botId: 'bot1', isMine: true, ownerName: 'Ren', ownedByMe: true, createdAtMs: 3000 }),
    ];
    act(() => root.render(<ChannelView space="s1" channel={{ id: 'c1', name: 'general', archived: false, unread: 0 }} />));
    const rows = Array.from(host.querySelectorAll('li.ck-spaces-msg'));
    expect(rows.map((r) => r.classList.contains('is-grouped'))).toEqual([false, true, false]);
    expect(rows.map((r) => r.hasAttribute('data-mine'))).toEqual([false, false, false]);
    expect(rows[0].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Sam');
    expect(rows[2].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Ren的fish');
  });
});

describe('the display name prompt', () => {
  it('asks for a name before the New space sheet when none is set', async () => {
    state.view = view({ displayName: null });
    state.invoke.mockImplementation(async (cmd: string, args?: unknown) =>
      cmd === 'spaces_set_display_name' ? { displayName: (args as { name: string }).name, view: view({ displayName: 'Sam' }) } : null);
    await act(async () => root.render(<SpacesPage />));
    await act(async () => { button('New space')!.click(); });
    expect(host.textContent).toContain('Your name in Spaces');
    expect(host.querySelector('input[placeholder="Book club"]')).toBeNull();
    await act(async () => { typeInto(host.querySelector('input') as HTMLInputElement, ' Sam '); });
    await act(async () => { button('Continue')!.click(); });
    expect(state.invoke).toHaveBeenCalledWith('spaces_set_display_name', { name: 'Sam' });
    expect(host.textContent).not.toContain('Your name in Spaces');
    expect(host.querySelector('input[placeholder="Book club"]')).not.toBeNull();
  });

  it('asks before the Join sheet too', async () => {
    state.view = view({ displayName: null });
    await act(async () => root.render(<SpacesPage />));
    await act(async () => { button('Join a space')!.click(); });
    expect(host.textContent).toContain('Your name in Spaces');
    expect(host.textContent).not.toContain('Invite link');
  });

  it('asks someone already in a space (from before names) once, on opening', async () => {
    state.view = view({ displayName: null, spaces: [{ id: 's1', name: 'Home' } as unknown as SpacesView['spaces'][number]] });
    await act(async () => root.render(<SpacesPage />));
    expect(host.textContent).toContain('Your name in Spaces');
    await act(async () => { button('Cancel')!.click(); });
    expect(host.textContent).not.toContain('Your name in Spaces');
  });

  it('goes straight to New space once a name is set', async () => {
    state.view = view({ displayName: 'Sam' });
    await act(async () => root.render(<SpacesPage />));
    await act(async () => { button('New space')!.click(); });
    expect(host.textContent).not.toContain('Your name in Spaces');
    expect(host.querySelector('input[placeholder="Book club"]')).not.toBeNull();
  });
});

// settings.css hides a row and a group that have no `.st-rowwrap > .st-row` (or `> section`) child; jsdom cannot run :has(),
// so the rule is checked here by its own selectors (same as PhoneSection.visible.test.tsx).
const rowDrawn = (wrap: Element) => [...wrap.children].some((c) => c.matches('.st-row, section'));

describe('Settings > Display name', () => {
  let m: Mounted | null = null;
  afterEach(async () => {
    await m?.unmount();
    m = null;
  });

  it('draws the row with the saved name, and saves a change through the command', async () => {
    state.invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'spaces_display_name') return { displayName: 'Ren' };
      if (cmd === 'spaces_set_display_name') return { displayName: (args as { name: string }).name, view: view() };
      return null;
    });
    m = await mountRows(spacesRows.filter((r) => r.id === 'spaces-display-name'), { settings: { spaces_enabled: true } });
    await settle(3);
    const wrap = m.host.querySelector('.st-rowwrap')!;
    expect(rowDrawn(wrap), 'row would be display:none').toBe(true);
    const group = m.host.querySelector('.st-group')!;
    expect([...group.querySelectorAll('.st-rowwrap')].some(rowDrawn), 'group would be display:none').toBe(true);
    expect(m.host.textContent).toContain('Display name');
    const input = m.host.querySelector('input') as HTMLInputElement;
    expect(input.value).toBe('Ren');
    await act(async () => {
      typeInto(input, 'Renata');
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    await settle(3);
    expect(state.invoke).toHaveBeenCalledWith('spaces_set_display_name', { name: 'Renata' });
  });
});
