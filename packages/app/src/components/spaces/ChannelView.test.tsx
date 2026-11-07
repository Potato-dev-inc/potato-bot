// @vitest-environment jsdom
// The channel feed: order, the start line, older messages, deleted and blocked rows, replies, and the archived lock.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceChannel, SpaceMessage } from '../../lib/spaces';

const state = vi.hoisted(() => ({
  ch: {
    messages: [] as unknown[],
    atStart: true,
    loadOlder: vi.fn(async () => {}),
    send: vi.fn(async () => null as string | null),
    error: null as string | null,
  },
}));

vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return { ...orig, useChannel: () => state.ch };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

import { ChannelView } from './ChannelView';

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.ch.messages = [];
  state.ch.atStart = true;
  state.ch.loadOlder.mockClear();
  state.ch.send.mockReset();
  state.ch.error = null;
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const channel: SpaceChannel = { id: 'c1', name: 'general', archived: false, unread: 0 };
const at = new Date(2026, 10, 7, 10, 2).getTime();

const msg = (over: Partial<SpaceMessage> = {}): SpaceMessage => ({
  id: 'm1',
  channel: 'c1',
  member: 'mem-sam',
  authorName: 'Sam',
  isFish: false,
  isMine: false,
  createdAtMs: at,
  text: 'hi',
  replyTo: null,
  replyPreview: null,
  deleted: null,
  reactions: [],
  file: null,
  blocked: false,
  ...over,
});

function render(over: Partial<SpaceChannel> = {}) {
  act(() => {
    root.render(<ChannelView space="s1" channel={{ ...channel, ...over }} />);
  });
}
const rows = () => Array.from(host.querySelectorAll('li.ck-spaces-msg'));
const buttonNamed = (name: string) =>
  Array.from(host.querySelectorAll('button')).find((b) => b.textContent === name) as HTMLButtonElement;

describe('ChannelView', () => {
  it('shows messages oldest first with author and time', () => {
    state.ch.messages = [
      msg({ id: 'm1', text: 'hi', authorName: 'Sam', createdAtMs: new Date(2026, 10, 7, 10, 2).getTime() }),
      msg({ id: 'm2', text: 'yo', authorName: 'Ana', createdAtMs: new Date(2026, 10, 7, 10, 3).getTime() }),
    ];
    render();
    const r = rows();
    expect(r.map((x) => x.querySelector('.ck-spaces-msg-text')?.textContent)).toEqual(['hi', 'yo']);
    expect(r[0].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Sam');
    expect(r[0].querySelector('time')?.textContent).toBe('10:02');
  });

  it('shows the start of channel line', () => {
    render();
    expect(host.querySelector('.ck-spaces-start')?.textContent).toBe('This is the start of #general.');
  });

  it('older button loads older', () => {
    state.ch.atStart = false;
    state.ch.messages = [msg()];
    render();
    act(() => buttonNamed('Show older messages').click());
    expect(state.ch.loadOlder).toHaveBeenCalledTimes(1);
  });

  it('fish rows are marked', () => {
    state.ch.messages = [msg({ isFish: true, authorName: "Fish (Ren's)", text: 'on it' })];
    render();
    const row = rows()[0];
    expect(row.classList.contains('is-fish')).toBe(true);
    expect(row.querySelector('.ck-spaces-msg-author')?.textContent).toBe("Fish (Ren's)");
  });

  it('deleted row texts', () => {
    state.ch.messages = [
      msg({ id: 'a', isMine: true, deleted: { byName: 'Me', byOwner: false } }),
      msg({ id: 'b', deleted: { byName: 'Owner', byOwner: true } }),
      msg({ id: 'c', deleted: { byName: 'Sam', byOwner: false } }),
    ];
    render();
    const texts = rows().map((x) => x.querySelector('.ck-spaces-msg-text')?.textContent);
    expect(texts).toEqual(['You deleted this message.', 'Deleted by the owner.', 'Sam deleted this message.']);
  });

  it('blocked row hides text until Show', () => {
    state.ch.messages = [msg({ blocked: true, text: 'secret' })];
    render();
    expect(host.textContent).toContain('Message from someone you blocked.');
    expect(host.textContent).not.toContain('secret');
    act(() => buttonNamed('Show').click());
    expect(host.textContent).toContain('secret');
  });

  it('reply sets the reply bar', () => {
    state.ch.messages = [msg({ authorName: 'Sam' })];
    render();
    act(() => buttonNamed('Reply').click());
    expect(host.querySelector('.ck-spaces-composer-reply')?.textContent).toContain('Replying to Sam');
  });

  it('text is not rendered as html', () => {
    state.ch.messages = [msg({ text: '<b>x</b>' })];
    render();
    const text = rows()[0].querySelector('.ck-spaces-msg-text');
    expect(text?.textContent).toBe('<b>x</b>');
    expect(text?.querySelector('b')).toBeNull();
  });

  it('archived channel disables composer', () => {
    render({ archived: true });
    const box = host.querySelector('textarea') as HTMLTextAreaElement;
    expect(box.disabled).toBe(true);
  });

  it('read-only feed has no composer and the row gives its Reply to the menu', () => {
    state.ch.messages = [msg()];
    act(() => {
      root.render(<ChannelView space="s1" channel={channel} readOnly menu={(m, reply) => <button onClick={reply}>menu {m.id}</button>} />);
    });
    expect(host.querySelector('textarea')).toBeNull();
    expect(buttonNamed('menu m1')).toBeTruthy();
    expect(buttonNamed('Reply')).toBeUndefined();
  });
});
