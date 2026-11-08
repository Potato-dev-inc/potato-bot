// @vitest-environment jsdom
// Real-screen proof (SpacesPage) that the Spaces sheets and the Bots panel are built the Potato way:
// the shared `.ck-sheet` and `.ck-btn` classes, one primary action, a body that scrolls while the footer stays put,
// and the bot panel split into a card per bot with the three kinds of control kept in their own places.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceDetail, SpacesView } from '../../lib/spaces';
import { botList, botRow, otherRow } from './botTestKit';

const state = vi.hoisted(() => ({
  view: null as SpacesView | null,
  detail: null as SpaceDetail | null,
  invite: null as unknown,
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => null as unknown),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: state.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return {
    ...orig,
    useSpaces: () => ({
      view: state.view, error: null, create: async () => 's9', invite: async () => state.invite, cancelInvite: async () => {},
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
const view = (over: Partial<SpacesView> = {}): SpacesView => ({
  spaces: [{ id: 's1', name: 'Book club', isOwner: true, unread: 0, members: 1, online: 0 }],
  requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true, ...over,
});
const detail = (over: Partial<SpaceDetail> = {}): SpaceDetail => ({
  id: 's1', name: 'Book club', isOwner: true, myMember: 'm1', ownerName: 'Ren', channels, members: [], openInvites: 0,
  relayed: false, anyoneOnline: true, state: 'active', leavePending: false, bots: [],
  botRules: { perPerson: 3, channels: null }, ...over,
});
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) as HTMLButtonElement | undefined;
const click = async (el: Element | null | undefined) => {
  if (!el) throw new Error('nothing to click');
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};
const sheet = () => document.querySelector('.ck-sheet') as HTMLElement;

async function setUp(opts: { view?: Partial<SpacesView>; detail?: Partial<SpaceDetail> } = {}) {
  state.view = view(opts.view);
  state.detail = detail(opts.detail);
  state.invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'space_bot_list') return botList();
    if (cmd === 'space_bot_candidates') return { space: 's1', candidates: [{ id: 'p2', name: 'cat', suggestedName: 'Ren的cat' }] };
    return null;
  });
  await act(async () => root.render(<SpacesPage />));
  await act(async () => {});
}

/** What every sheet shares: the Potato sheet and buttons, one primary action, and a footer outside the scrolling body. */
function expectPotatoSheet(primary: string) {
  const s = sheet();
  expect(s).not.toBeNull();
  expect(s.querySelector('.ck-spaces-btn')).toBeNull();
  const primaries = [...s.querySelectorAll('.ck-btn--primary')].map((b) => b.textContent?.trim());
  expect(primaries).toEqual([primary]);
  const footer = s.querySelector('.ck-sheet-actions');
  expect(footer).not.toBeNull();
  expect(footer?.closest('.ck-spaces-sheet-scroll')).toBeNull();
  expect(footer?.parentElement).toBe(s);
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.invoke.mockReset();
  state.invite = null;
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('Spaces sheets, from the Spaces screen', () => {
  it('New space and Join a space are Potato sheets with one primary action', async () => {
    await setUp();
    await click(button('New space'));
    expectPotatoSheet('Create');
    await click(button('Cancel'));
    await click(button('Join a space'));
    expectPotatoSheet('Ask to join');
  });

  it('the invite sheet, the join request and Space info are Potato sheets with one primary action', async () => {
    state.invite = {
      id: 'i1', space: 's1', link: 'pms1:abc', words: ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'], qr: '',
      expiresAtMs: Date.now() + 3_600_000, state: 'open',
    };
    await setUp();
    await click(button('Invite people'));
    expectPotatoSheet('Done');
    // The words sit in their own group, and Cancel invite is not the primary action.
    expect(document.querySelectorAll('.ck-spaces-words li')).toHaveLength(8);
    expect(button('Cancel invite')?.classList.contains('ck-btn--primary')).toBe(false);
    await click(button('Done'));
    await click(button('Space info'));
    expectPotatoSheet('Close');
    // The switch keeps being a real checkbox, in a grouped card.
    expect(sheet().querySelector('.ck-spaces-card input[type="checkbox"].ck-spaces-switch')).not.toBeNull();
  });

  it('a join request is a Potato sheet where Let in is the one primary action', async () => {
    await setUp({
      view: { requests: [{ id: 1, space: 's1', spaceName: 'Book club', name: 'Sam', short: 'ab12', receivedAtMs: 0, answerByMs: 9e12 }] },
    });
    expectPotatoSheet('Let in');
    expect(button('No')?.classList.contains('ck-btn--secondary')).toBe(true);
  });

  it('a confirm sheet has Cancel and one bold primary action', async () => {
    await setUp({ view: { spaces: [{ id: 's1', name: 'Book club', isOwner: false, unread: 0, members: 2, online: 1 }] }, detail: { isOwner: false } });
    await click(document.querySelector('[aria-label="Space actions"]'));
    await click(button('Leave space'));
    expectPotatoSheet('Leave');
    expect(button('Leave')?.classList.contains('is-danger')).toBe(true);
    expect(button('Cancel')?.classList.contains('ck-btn--secondary')).toBe(true);
  });
});

describe('the Bots panel, from the Spaces screen', () => {
  it('is a scrolling body with a card per bot, state in words, and the three kinds of control in their own places', async () => {
    await setUp({ detail: { bots: [] } });
    await click(button('Bots'));
    const s = sheet();
    expect(s.classList.contains('is-wide')).toBe(true);
    // One primary action in the panel: Add my bot. Close and Save bot rules are quiet.
    const primaries = [...s.querySelectorAll('.ck-btn--primary')].map((b) => b.textContent?.trim());
    expect(primaries).toEqual(['Add my bot']);
    expect(s.querySelector('.ck-spaces-btn')).toBeNull();
    const footer = s.querySelector('.ck-sheet-actions') as HTMLElement;
    expect(footer.textContent).toBe('Close');
    expect(footer.closest('.ck-spaces-sheet-scroll')).toBeNull();

    const cards = [...s.querySelectorAll('li.ck-spaces-card')] as HTMLElement[];
    expect(cards).toHaveLength(2);
    const mine = cards[0];
    const theirs = cards[1];
    // State is said in words on each card.
    expect(mine.querySelector('.ck-spaces-bot-state')?.textContent).toBe('Your bot');
    expect(theirs.querySelector('.ck-spaces-bot-state')?.textContent).toBe('Owner: Sam');
    // Owner settings only on my bot, inside its card; hiding (everyone) is outside that group; the space owner's rules are a separate group.
    expect(mine.querySelector('fieldset legend')?.textContent).toBe("Your bot's settings");
    expect(theirs.querySelector('fieldset')).toBeNull();
    for (const c of cards) {
      const hide = [...c.querySelectorAll('button')].find((b) => b.textContent === 'Hide on this computer');
      expect(hide).toBeDefined();
      expect(hide?.closest('fieldset')).toBeNull();
    }
    const rules = s.querySelector('section[aria-label="Space owner settings"]') as HTMLElement;
    expect(rules).not.toBeNull();
    expect(rules.closest('li')).toBeNull();
    expect(rules.closest('.ck-spaces-sheet-scroll')).not.toBeNull();
    // The settings are rows in one card: Memory and Silence are real switches.
    const switches = [...mine.querySelectorAll('input[role="switch"]')];
    expect(switches).toHaveLength(2);
    expect(switches.every((i) => i.classList.contains('ck-spaces-switch'))).toBe(true);
  });

  it('keeps Add my bot and the limit line in one row, and an error stays outside the scrolling body', async () => {
    state.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'space_bot_list') return botList({ rules: { perPerson: 1, channels: null }, bots: [botRow(), otherRow()] });
      if (cmd === 'space_bot_remove') throw 'Only the bot owner can change this.';
      return null;
    });
    state.view = view();
    state.detail = detail();
    await act(async () => root.render(<SpacesPage />));
    await act(async () => {});
    await click(button('Bots'));
    const add = button('Add my bot') as HTMLButtonElement;
    const row = add.closest('.ck-spaces-addrow') as HTMLElement;
    expect(row.textContent).toContain('This space allows 1 bot per person. You have 1.');
    await click([...document.querySelectorAll('li.ck-spaces-card button')].find((b) => b.textContent === 'Remove bot'));
    await click(button('Yes, remove'));
    const alert = sheet().querySelector('[role="alert"]') as HTMLElement;
    expect(alert.textContent).toBe('Only the bot owner can change this.');
    expect(alert.closest('.ck-spaces-sheet-scroll')).toBeNull();
    expect(button('Yes, remove')).toBeUndefined();
  });

  it('Add a bot is a Potato sheet, and Change Bots per person still lands on the setting', async () => {
    state.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'space_bot_list') return botList({ rules: { perPerson: 1, channels: null }, mineCount: 1, bots: [botRow()] });
      if (cmd === 'space_bot_candidates') return { space: 's1', candidates: [{ id: 'p2', name: 'cat', suggestedName: 'Ren的cat' }] };
      return null;
    });
    state.view = view();
    state.detail = detail();
    await act(async () => root.render(<SpacesPage />));
    await act(async () => {});
    await click(button('Bots'));
    await click(button('Add my bot'));
    const s = sheet();
    expect(s.querySelector('h2')?.textContent).toBe('Add a bot');
    expect(s.querySelector('.ck-spaces-btn')).toBeNull();
    expect(s.querySelectorAll('.ck-btn--primary')).toHaveLength(1);
    expect(s.querySelector('.ck-sheet-actions')?.closest('.ck-spaces-sheet-scroll')).toBeNull();
    await click(button('Change Bots per person'));
    const per = document.querySelector('.ck-spaces-bot-rules select') as HTMLSelectElement;
    expect(document.activeElement).toBe(per);
    expect(per.closest('.ck-spaces-card')).not.toBeNull();
  });
});
