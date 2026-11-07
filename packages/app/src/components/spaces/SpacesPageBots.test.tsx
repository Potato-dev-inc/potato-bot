// @vitest-environment jsdom
// SpacesPage level proof that the bot screens are mounted and reachable by clicking, and what the channel shows.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceDetail, SpacesView } from '../../lib/spaces';
import { botList } from './botTestKit';

const state = vi.hoisted(() => ({
  view: null as SpacesView | null,
  detail: null as SpaceDetail | null,
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => null as unknown),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: state.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

// Only the page's data hooks are replaced. The bot hook and the bot commands are the real ones.
vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return {
    ...orig,
    useSpaces: () => ({
      view: state.view, error: null, create: async () => 's9', invite: async () => null, cancelInvite: async () => {},
      join: async () => true, answer: async () => {}, cancelJoin: async () => {},
    }),
    useSpace: () => ({ detail: state.detail, error: null, reload: () => {} }),
    useChannel: () => ({ messages: [], atStart: true, loadOlder: async () => {}, send: async () => null, error: null }),
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
const channels = [
  { id: 'c1', name: 'general', archived: false, unread: 0 },
  { id: 'c2', name: 'books', archived: false, unread: 0 },
];
const botsInDetail = [
  { id: 'b1', name: 'Ren的fish', ownerName: 'Ren', mine: true, muted: false },
  { id: 'b2', name: 'Sam的cat', ownerName: 'Sam', mine: false, muted: false },
];
const view = (): SpacesView => ({
  spaces: [{ id: 's1', name: 'Book club', isOwner: true, unread: 0, members: 2, online: 0 }],
  requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true,
});
const detail = (over: Partial<SpaceDetail> = {}): SpaceDetail => ({
  id: 's1', name: 'Book club', isOwner: true, myMember: 'm1', ownerName: 'Ren', channels, members: [], openInvites: 0,
  relayed: false, anyoneOnline: true, state: 'active', leavePending: false, bots: botsInDetail,
  botRules: { perPerson: 3, channels: null }, ...over,
});
const text = () => document.body.textContent ?? '';
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) ?? null;
const click = async (el: Element | null) => {
  if (!el) throw new Error('nothing to click');
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};

async function setUp(over: Partial<SpaceDetail> = {}) {
  state.view = view();
  state.detail = detail(over);
  state.invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'space_bot_list') return botList({ bots: [botList().bots[0]] });
    if (cmd === 'space_bot_candidates') return { space: 's1', candidates: [{ id: 'p1', name: 'fish', suggestedName: 'Ren的fish' }] };
    if (cmd === 'space_bot_add') return botList({ bots: [botList().bots[0]], pendingAdd: ['Ren的fish'] });
    return null;
  });
  await act(async () => root.render(<SpacesPage />));
  await act(async () => {});
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.invoke.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('Spaces page bots', () => {
  it('the Bots button opens the real BotPanel, Add my bot opens the real AddBotSheet, and Add bot goes back to the panel', async () => {
    await setUp();
    expect(text()).not.toContain('Bots in Book club');
    await click(button('Bots'));
    expect(state.invoke).toHaveBeenCalledWith('space_bot_list', { space: 's1' });
    expect(text()).toContain('Bots in Book club');
    expect(text()).toContain('Answered 3 of 50 today.');
    await click(button('Add my bot'));
    expect(state.invoke).toHaveBeenCalledWith('space_bot_candidates', { space: 's1' });
    expect(document.querySelector('[role="dialog"] h2')?.textContent).toBe('Add a bot');
    expect((document.querySelector('.ck-spaces-field input') as HTMLInputElement).value).toBe('Ren的fish');
    expect(text()).toContain('People here may get your bot to repeat what it remembers.');
    await click(button('Add bot'));
    expect(state.invoke).toHaveBeenCalledWith('space_bot_add', { space: 's1', potatoBot: 'p1', name: 'Ren的fish' });
    expect(document.querySelector('[role="dialog"] h2')?.textContent).toBe('Bots in Book club');
    expect(text()).toContain("Waiting for the space owner's computer: Ren的fish");
  });

  it('Close leaves the panel', async () => {
    await setUp();
    await click(button('Bots'));
    await click(button('Close'));
    expect(text()).not.toContain('Bots in Book club');
  });

  it('the channel header shows the bots line and the call hint', async () => {
    await setUp();
    expect(text()).toContain('Bots here: Ren的fish · Sam的cat');
    expect(text()).toContain("Type @ and a bot's name to call it.");
  });

  it('a channel outside the rules shows that bots are off', async () => {
    await setUp({ botRules: { perPerson: 3, channels: ['c2'] } });
    expect(text()).toContain('Bots are off in this channel.');
    expect(text()).not.toContain('Bots here:');
  });

  it('a channel inside the rules shows the bots', async () => {
    await setUp({ botRules: { perPerson: 3, channels: ['c1'] } });
    expect(text()).toContain('Bots here: Ren的fish · Sam的cat');
  });

  it('a space I left has no Bots button', async () => {
    await setUp({ state: 'left' });
    expect(button('Bots')).toBeNull();
  });
});
