// @vitest-environment jsdom
// SpacesPage level proof for the channel view design pass: the `@` picker, bots in the People column, who said what,
// the "did not answer" note going away, Escape on the More menu, and the narrow People control.
// What jsdom cannot show: that the page fills the window height, that More appears on hover, and how it looks at 480px.
// Those are CSS; the guard test at the end only checks that the rule is written down, it does not prove the pixels.
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
      view: state.view, error: null, create: async () => 's9', invite: async () => null, cancelInvite: async () => {},
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
  state.send.mockClear();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('the @ picker', () => {
  it('typing @ offers the bots here, and more letters narrow the list', async () => {
    await setUp();
    typeLine('hi @');
    expect(options()).toEqual(['Ren的Hoot', 'Sam的Cat']);
    typeLine('hi @ho');
    expect(options()).toEqual(['Ren的Hoot']);
    typeLine('hi @zzz');
    expect(options()).toEqual([]);
  });

  it('Enter inserts the full name, with the 的, and a space after it', async () => {
    await setUp();
    typeLine('@ho');
    await press(box(), 'Enter');
    expect(box().value).toBe('@Ren的Hoot ');
    expect(options()).toEqual([]);
    expect(state.send).not.toHaveBeenCalled();
  });

  it('arrows move the choice and Tab takes it', async () => {
    await setUp();
    typeLine('@');
    await press(box(), 'ArrowDown');
    await press(box(), 'Tab');
    expect(box().value).toBe('@Sam的Cat ');
  });

  it('Escape closes the list and keeps the text; the mouse picks too', async () => {
    await setUp();
    typeLine('hello @s');
    await press(box(), 'Escape');
    expect(options()).toEqual([]);
    expect(box().value).toBe('hello @s');
    typeLine('hello @sa');
    await click(document.querySelector('[role="option"]'));
    expect(box().value).toBe('hello @Sam的Cat ');
  });

  it('is not offered where bots are off, for a mail-like @ inside a word, or for a hidden bot', async () => {
    await setUp({ botRules: { perPerson: 3, channels: ['other'] } });
    typeLine('@');
    expect(options()).toEqual([]);
    await setUp({ bots: [hoot({ muted: true }), cat()] });
    typeLine('mail me at ren@');
    expect(options()).toEqual([]);
    typeLine('@');
    expect(options()).toEqual(['Sam的Cat']);
  });
});

describe('bots in the People column and in the feed', () => {
  it('lists bots under their own heading with the owner and the word Bot, and counts only people', async () => {
    await setUp();
    const people = document.querySelector('aside[aria-label="People"]') as HTMLElement;
    expect(people.querySelector('h2')?.textContent).toBe('People (1)');
    const heads = [...people.querySelectorAll('h2')].map((h) => h.textContent);
    expect(heads).toEqual(['People (1)', 'Bots']);
    const rows = [...people.querySelectorAll('.ck-spaces-botmember')].map((r) => r.textContent);
    expect(rows).toEqual(['Ren的HootBot · Ren (yours)', 'Sam的CatBot · Sam']);
    expect(people.querySelectorAll('.ck-spaces-member')).toHaveLength(1);
  });

  it('a bot message says Bot in words; a person and I do not', async () => {
    await setUp({}, [
      message({ id: 'a', authorName: 'Ren的Hoot', isFish: true, isMine: false, botId: 'b1', text: 'hoo' }),
      message({ id: 'b', authorName: 'Sam', isMine: false, member: 'm2', text: 'hi' }),
      message({ id: 'c' }),
    ]);
    const rows = [...document.querySelectorAll('li.ck-spaces-msg')];
    expect(rows.map((r) => r.querySelector('.ck-spaces-bot-tag')?.textContent ?? null)).toEqual(['Bot', null, null]);
    expect(rows[0].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Ren的Hoot');
  });
});

describe('the "did not answer" note', () => {
  const NOTE = 'Ren的Hoot did not answer: it is switched off, paused or has no key.';
  it('goes once a later line that calls nobody is sent', async () => {
    await setUp({ bots: [hoot({ refused: 'unavailable' }), cat()] });
    expect(text()).toContain(NOTE);
    typeLine('just talking');
    await press(box(), 'Enter');
    await act(async () => {});
    expect(state.send).toHaveBeenCalledWith('just talking', null);
    expect(text()).not.toContain('did not answer');
  });

  it('stays when the next line calls a bot, and comes back for a new refusal', async () => {
    await setUp({ bots: [hoot({ refused: 'unavailable' }), cat()] });
    typeLine('@Ren的Hoot again?');
    await press(box(), 'Enter');
    await act(async () => {});
    expect(text()).toContain(NOTE);
    typeLine('and now plain');
    await press(box(), 'Enter');
    await act(async () => {});
    expect(text()).not.toContain('did not answer');
    // the backend takes a later call (cleared), then refuses another one
    await act(async () => { state.detail = detail({ bots: [hoot({ refused: null }), cat()] }); root.render(<SpacesPage />); });
    await act(async () => { state.detail = detail({ bots: [hoot({ refused: 'unavailable' }), cat()] }); root.render(<SpacesPage />); });
    expect(text()).toContain(NOTE);
  });
});

describe('the message menu and the narrow People control', () => {
  it('More is a real button in the keyboard order, and Escape closes its menu', async () => {
    await setUp({}, [message()]);
    const more = document.querySelector('[aria-label="Message actions"]') as HTMLButtonElement;
    expect(more.tabIndex).toBe(0);
    expect(more.textContent).toBe('More');
    await click(more);
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    await press(document.querySelector('[role="menuitem"]') as Element, 'Escape');
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(more.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(more);
  });

  it('a click elsewhere closes the menu too', async () => {
    await setUp({}, [message()]);
    await click(document.querySelector('[aria-label="Message actions"]'));
    await act(async () => { document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true })); });
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it('the People button opens the people and bots panel, says so, and Escape closes it', async () => {
    await setUp();
    const people = document.querySelector('aside[aria-label="People"]') as HTMLElement;
    const toggle = button('People') as HTMLButtonElement;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(people.classList.contains('is-open')).toBe(false);
    await click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(people.classList.contains('is-open')).toBe(true);
    expect(people.textContent).toContain('Sam的Cat');
    await press(document.body, 'Escape');
    expect(people.classList.contains('is-open')).toBe(false);
  });
});

describe('the page frame (a guard, not a proof of the pixels)', () => {
  const css = readFileSync(resolve(process.cwd(), 'src/styles/cockpit-spaces.css'), 'utf8');
  it('writes down that the page fills the height of the main column and the feed scrolls inside it', () => {
    const rule = (sel: string) => css.slice(css.indexOf(`${sel} {`), css.indexOf('}', css.indexOf(`${sel} {`)));
    expect(rule('.cockpit .ck-spaces')).toMatch(/flex: 1;[^}]*min-height: 0/);
    expect(rule('.cockpit .ck-spaces-layout')).toMatch(/flex: 1;[^}]*min-height: 0/);
    expect(rule('.cockpit .ck-spaces-msgs')).toMatch(/overflow-y: auto/);
    expect(css).toMatch(/@media \(max-width: 820px\)/);
  });
});

describe('the More menu opens upward near the bottom of the feed', () => {
  // jsdom has no layout, so the rects are stubbed: the menu list's bottom and the feed's bottom.
  const stubRects = (menuBottom: number, feedBottom: number) =>
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
      const bottom = this.classList.contains('ck-spaces-menu-list') ? menuBottom
        : this.classList.contains('ck-spaces-msgs') ? feedBottom : 0;
      return { x: 0, y: 0, top: 0, left: 0, right: 0, width: 0, height: 0, bottom, toJSON: () => ({}) } as DOMRect;
    });
  afterEach(() => vi.restoreAllMocks());

  it('the menu passes the bottom of the feed, so it gets the upward class', async () => {
    stubRects(500, 400);
    await setUp({}, [message()]);
    await click(document.querySelector('[aria-label="Message actions"]'));
    expect(document.querySelector('[role="menu"]')?.classList.contains('is-up')).toBe(true);
  });

  it('the menu fits above the feed bottom, so the class is absent', async () => {
    stubRects(300, 400);
    await setUp({}, [message()]);
    await click(document.querySelector('[aria-label="Message actions"]'));
    expect(document.querySelector('[role="menu"]')?.classList.contains('is-up')).toBe(false);
  });
});
