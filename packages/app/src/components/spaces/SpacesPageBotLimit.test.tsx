// @vitest-environment jsdom
// Real-screen proof (SpacesPage) for the bot limit and for calling a bot with `@`:
// - the founder's case: one bot per person, `Ren的Hoot` already here, Add my bot pressed;
// - the limit is told before the form, with a way to the setting for the space owner, and a plain line for a member;
// - a call nobody will answer says why (wrong name, silenced, hidden, daily limit, bot off, bots off in the channel).
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceDetail, SpaceDetailBot, SpacesView } from '../../lib/spaces';
import { botList, botRow, otherRow } from './botTestKit';
import { ALL_HERE, NO_CANDIDATES } from './AddBotSheet';

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
const channels = [{ id: 'c1', name: 'general', archived: false, unread: 0 }];
const hoot = (over: Partial<SpaceDetailBot> = {}): SpaceDetailBot => ({
  id: 'b1', name: 'Ren的Hoot', ownerName: 'Ren', mine: true, muted: false, silenced: false, dayCap: 30, usedToday: 0, refused: null, ...over,
});
const view = (): SpacesView => ({
  spaces: [{ id: 's1', name: 'Book club', isOwner: true, unread: 0, members: 1, online: 0 }],
  requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true,
});
const detail = (over: Partial<SpaceDetail> = {}): SpaceDetail => ({
  id: 's1', name: 'Book club', isOwner: true, myMember: 'm1', ownerName: 'Ren', channels, members: [], openInvites: 0,
  relayed: false, anyoneOnline: true, state: 'active', leavePending: false, bots: [hoot()],
  botRules: { perPerson: 1, channels: null }, ...over,
});
const text = () => document.body.textContent ?? '';
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) as HTMLButtonElement | undefined;
const click = async (el: Element | null | undefined) => {
  if (!el) throw new Error('nothing to click');
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};
const composer = () => document.querySelector('textarea') as HTMLTextAreaElement;
const typeLine = (value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  act(() => {
    setter?.call(composer(), value);
    composer().dispatchEvent(new Event('input', { bubbles: true }));
  });
};

/** The founder's roster: one bot of mine, one bot per person. */
const atLimit = (over = {}) => botList({
  rules: { perPerson: 1, channels: null }, mineCount: 1, bots: [botRow({ id: 'b1', name: 'Ren的Hoot', mode: 'helps' })], ...over,
});

async function setUp(opts: { detail?: Partial<SpaceDetail>; list?: ReturnType<typeof botList>; add?: () => unknown; candidates?: unknown[] } = {}) {
  state.view = view();
  state.detail = detail(opts.detail);
  const list = opts.list ?? atLimit();
  const candidates = opts.candidates ?? [{ id: 'p2', name: 'cat', suggestedName: 'Ren的cat' }];
  state.invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'space_bot_list') return list;
    if (cmd === 'space_bot_candidates') return { space: 's1', candidates };
    if (cmd === 'space_bot_add') {
      if (opts.add) return opts.add();
      return list;
    }
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

describe('the bot limit, from the Spaces screen', () => {
  it('the panel tells the limit and what is used before Add is pressed, and Add stays pressable', async () => {
    await setUp();
    await click(button('Bots'));
    expect(text()).toContain('This space allows 1 bot per person. You have 1.');
    expect(button('Add my bot')?.disabled).toBe(false);
  });

  it('at the limit the add sheet names the bot that is already here and shows no form', async () => {
    await setUp();
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).toContain('Ren的Hoot is already here. This space allows 1 bot per person.');
    expect(text()).not.toContain('Name in this space');
    expect(button('Add bot')?.disabled).toBe(true);
    // said in words, not as a failed attempt
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });

  it('the space owner gets a way straight to Bots per person', async () => {
    await setUp();
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).not.toContain('Ask the space owner to allow more.');
    await click(button('Change Bots per person'));
    expect(document.querySelector('[role="dialog"] h2')?.textContent).toBe('Bots in Book club');
    const per = document.querySelector('.ck-spaces-bot-rules select') as HTMLSelectElement;
    expect(per.value).toBe('1');
    expect(document.activeElement).toBe(per);
  });

  it('a member is told to ask the space owner and gets no setting', async () => {
    await setUp({ list: atLimit({ isOwner: false }), detail: { isOwner: false } });
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).toContain('Ren的Hoot is already here. This space allows 1 bot per person.');
    expect(text()).toContain('Ask the space owner to allow more.');
    expect(button('Change Bots per person')).toBeUndefined();
  });

  it('a bot still waiting for the space owner counts, and is named', async () => {
    await setUp({ list: atLimit({ isOwner: false, mineCount: 0, bots: [otherRow()], pendingAdd: ['Sam的fish'] }), detail: { isOwner: false } });
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).toContain('Sam的fish is already here. This space allows 1 bot per person.');
  });

  it('with room left the limit is shown above the form', async () => {
    await setUp({ list: atLimit({ rules: { perPerson: 3, channels: null } }) });
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).toContain('This space allows 3 bots per person. You have 1.');
    expect(text()).toContain('Name in this space');
    expect(button('Add bot')?.disabled).toBe(false);
  });

  it('with room left but no bot to add, the sheet says all bots are here, not that none can join', async () => {
    await setUp({ list: atLimit({ rules: { perPerson: 3, channels: null } }), candidates: [] });
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).toContain(ALL_HERE);
    expect(text()).not.toContain(NO_CANDIDATES);
    expect(button('Add bot')?.disabled).toBe(true);
  });

  it('zero bots per person says so', async () => {
    await setUp({ list: atLimit({ rules: { perPerson: 0, channels: null }, mineCount: 0, bots: [] }) });
    await click(button('Bots'));
    await click(button('Add my bot'));
    expect(text()).toContain('This space allows 0 bots per person.');
  });

  it('when the space refuses although the list showed room, the sheet explains instead of showing the raw refusal', async () => {
    await setUp({
      list: atLimit({ rules: { perPerson: 3, channels: null } }),
      add: () => { throw 'You already have as many bots here as this space allows.'; },
    });
    await click(button('Bots'));
    await click(button('Add my bot'));
    await click(button('Add bot'));
    expect(text()).toContain('already here. This space allows 3 bots per person.');
    expect(text()).not.toContain('Name in this space');
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });
});

describe('calling a bot with @, from the Spaces screen', () => {
  it('@hoot when the bot is named Ren的Hoot says so and offers the full name', async () => {
    await setUp();
    typeLine('@hoot are you there?');
    expect(text()).toContain('No bot here is called @hoot. Did you mean @Ren的Hoot?');
  });

  it('the full name, in any case, gets no warning', async () => {
    await setUp();
    typeLine('@ren的hoot are you there?');
    expect(text()).not.toContain('No bot here');
    expect(text()).not.toContain('will not answer');
    typeLine('hello @someone else');
    expect(text()).not.toContain('No bot here');
  });

  it('a silenced bot, a hidden bot and a bot over its daily limit each say why they will not answer', async () => {
    await setUp({ detail: { bots: [hoot({ silenced: true })] } });
    typeLine('@Ren的Hoot hi');
    expect(text()).toContain('Ren的Hoot is silenced, so it will not answer.');
    await act(async () => { state.detail = detail({ bots: [hoot({ muted: true })] }); root.render(<SpacesPage />); });
    typeLine('@Ren的Hoot hi');
    expect(text()).toContain('Ren的Hoot is hidden on this computer.');
    await act(async () => { state.detail = detail({ bots: [hoot({ usedToday: 30 })] }); root.render(<SpacesPage />); });
    typeLine('@Ren的Hoot hi');
    expect(text()).toContain('Ren的Hoot reached its daily limit of 30, so it will not answer today.');
  });

  it('in a channel where bots are off a call says nobody will answer', async () => {
    await setUp({ detail: { botRules: { perPerson: 1, channels: ['c9'] } } });
    typeLine('@Ren的Hoot hi');
    expect(text()).toContain('Bots are off in this channel, so nobody will answer.');
  });

  it('a call the bot refused (off, no key) is explained under the composer after the send', async () => {
    await setUp({ detail: { bots: [hoot({ refused: 'unavailable' })] } });
    expect(text()).toContain('Ren的Hoot did not answer: it is switched off, paused or has no key.');
    await act(async () => { state.detail = detail({ bots: [hoot({ refused: 'limit' })] }); root.render(<SpacesPage />); });
    expect(text()).toContain('Ren的Hoot did not answer: it reached its daily money limit.');
    await act(async () => { state.detail = detail({ bots: [hoot({ refused: null })] }); root.render(<SpacesPage />); });
    expect(text()).not.toContain('did not answer');
  });

  it('someone else\'s bot is never reported as refused or silenced from here', async () => {
    await setUp({ detail: { bots: [hoot({ mine: false, silenced: null, refused: null, ownerName: 'Sam' })] } });
    typeLine('@Ren的Hoot hi');
    expect(text()).not.toContain('will not answer');
    expect(text()).not.toContain('did not answer');
  });
});

