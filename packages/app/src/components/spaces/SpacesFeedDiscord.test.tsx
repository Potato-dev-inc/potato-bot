// @vitest-environment jsdom
// The channel feed is Discord-like (grayscale): everyone left-aligned, avatar + name on the first row of a run, bots tagged,
// replies shown above, @mentions as pills, actions in a toolbar inside the hovered row, day dividers.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceMessage } from '../../lib/spaces';

const state = vi.hoisted(() => ({ messages: [] as unknown[] }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return { ...orig, useChannel: () => ({ messages: state.messages, atStart: true, loadOlder: async () => {}, send: async () => null, error: null }) };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { ChannelView } from './ChannelView';
import { MessageMenu } from './MessageMenu';
import { shadeOf } from './messageParts';

let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement('div'); document.body.appendChild(host); root = createRoot(host); state.messages = []; });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const channel = { id: 'c1', name: 'general', archived: false, unread: 0 };
const t0 = new Date().setHours(0, 0, 0, 0) + 60_000;
const msg = (over: Partial<SpaceMessage>): SpaceMessage => ({
  id: 'x', channel: 'c1', member: 'mem-ren', authorName: 'Ren', isFish: false, isMine: false, createdAtMs: t0, text: 'hi',
  replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false, ...over,
});
const conversation = (): SpaceMessage[] => [
  msg({ id: 'm1', isMine: true, text: 'hello team' }),
  msg({ id: 'm2', isMine: true, createdAtMs: t0 + 60_000, text: 'ping @Sam, please look' }),
  msg({ id: 'm3', member: 'mem-sam', authorName: 'Sam', createdAtMs: t0 + 120_000, text: '@Ren thanks, asking @Ren的Hoot',
    replyTo: 'm1', replyPreview: { authorName: 'Ren', text: 'hello team' } }),
  msg({ id: 'm4', member: 'mem-ren', authorName: 'Ren的Hoot', isFish: true, botId: 'bot-hoot', ownerName: 'Ren', ownedByMe: true,
    createdAtMs: t0 + 180_000, text: 'Line one\nLine two' }),
  msg({ id: 'm5', member: 'mem-sam', authorName: 'Sam的Pip', isFish: true, botId: 'bot-pip', ownerName: 'Sam', ownedByMe: false,
    createdAtMs: t0 + 240_000, text: 'ok' }),
];
const show = () => {
  state.messages = conversation();
  act(() => root.render(<ChannelView space="s1" channel={channel} />));
  return Array.from(host.querySelectorAll<HTMLElement>('li.ck-spaces-msg'));
};

describe('Discord-like feed', () => {
  it('every row is left-aligned: no right-align class or style, mine looks like the rest', () => {
    const rows = show();
    expect(rows).toHaveLength(5);
    for (const r of rows) {
      expect(r.classList.contains('is-mine')).toBe(false);
      expect(r.getAttribute('style')).toBeNull();
      expect(r.querySelector('.ck-potato-bubble')).toBeNull();
    }
    expect(rows[0].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Ren');
  });

  it('the first row of a run shows avatar and name; a grouped row hides them but keeps the name and a time', () => {
    const rows = show();
    expect(rows[0].classList.contains('is-grouped')).toBe(false);
    expect(rows[0].querySelector('.ck-spaces-avatar')?.textContent).toBe('R');
    expect(rows[1].classList.contains('is-grouped')).toBe(true);
    expect(rows[1].querySelector('.ck-spaces-avatar')).toBeNull();
    expect(rows[1].querySelector('.ck-spaces-msg-gutter time')).not.toBeNull();
    expect(rows[1].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Ren');
  });

  it('a bot shows a rounded avatar, the BOT tag and its owner', () => {
    const rows = show();
    expect(rows[3].querySelector('.ck-spaces-avatar')?.classList.contains('is-bot')).toBe(true);
    expect(rows[3].querySelector('.ck-spaces-bot-tag')?.textContent).toBe('Bot');
    expect(rows[3].querySelector('.ck-spaces-msg-owner')?.textContent).toBe('yours');
    expect(rows[4].querySelector('.ck-spaces-msg-owner')?.textContent).toBe("Sam's");
    expect(rows[3].querySelector('.ck-spaces-msg-text')?.textContent).toBe('Line one\nLine two');
  });

  it('avatar shade is stable per id and stays inside the gray scale', () => {
    expect(shadeOf('mem-ren')).toBe(shadeOf('mem-ren'));
    const rows = show();
    const shade = Number(rows[0].querySelector('.ck-spaces-avatar')?.getAttribute('data-shade'));
    expect(shade).toBe(shadeOf('mem-ren'));
    expect(shade).toBeGreaterThanOrEqual(0);
    expect(shade).toBeLessThan(6);
  });

  it('a reply shows the replied author above the header and jumps to the original', () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    const rows = show();
    const reply = rows[2].querySelector<HTMLButtonElement>('.ck-spaces-msg-reply')!;
    expect(reply.querySelector('.ck-spaces-reply-author')?.textContent).toBe('Ren');
    expect(reply.querySelector('.ck-spaces-reply-text')?.textContent).toBe('hello team');
    expect(reply.compareDocumentPosition(rows[2].querySelector('.ck-spaces-msg-line')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    act(() => reply.click());
    expect(scroll).toHaveBeenCalled();
    expect(host.querySelector('li[data-id="m1"]')?.classList.contains('is-flash')).toBe(true);
  });

  it('@mentions are separate pill elements; a mention of me is stronger and marks the row', () => {
    const rows = show();
    const pills = (r: HTMLElement) => Array.from(r.querySelectorAll('.ck-spaces-mention')).map((p) => [p.textContent, p.classList.contains('is-me')]);
    expect(pills(rows[1])).toEqual([['@Sam', false]]);
    expect(pills(rows[2])).toEqual([['@Ren', true], ['@Ren的Hoot', false]]);
    expect(rows[2].classList.contains('is-mention')).toBe(true);
    expect(rows[1].classList.contains('is-mention')).toBe(false);
    expect(rows[2].querySelector('.ck-spaces-msg-text')?.textContent).toBe('@Ren thanks, asking @Ren的Hoot');
  });

  it('the hover toolbar is inside its own row, never a separate row', () => {
    const rows = show();
    expect(host.querySelectorAll('.ck-spaces-msgs > .ck-spaces-msg-actions')).toHaveLength(0);
    for (const r of rows) expect(r.querySelectorAll(':scope > .ck-spaces-msg-actions')).toHaveLength(1);
    // the start line, one day divider and five rows: nothing else is a row of its own
    expect(host.querySelectorAll('.ck-spaces-msgs > li')).toHaveLength(7);
  });

  it('with the message menu, More sits inside the row toolbar', () => {
    state.messages = conversation();
    act(() => root.render(
      <ChannelView space="s1" channel={channel}
        menu={(m, reply) => <MessageMenu message={m} canDelete={false} onReply={reply} onReact={() => {}} onDelete={() => {}} />} />,
    ));
    const row = host.querySelector('li.ck-spaces-msg')!;
    expect(row.querySelector(':scope > .ck-spaces-msg-actions button[aria-label="Message actions"]')).not.toBeNull();
  });

  it('a day divider says Today, and a new day starts a new run', () => {
    show();
    const dividers = host.querySelectorAll('.ck-spaces-day');
    expect(dividers).toHaveLength(1);
    expect(dividers[0].textContent).toBe('Today');
    state.messages = [msg({ id: 'a', createdAtMs: t0 - 86_400_000 + 30 * 60_000 }), msg({ id: 'b', createdAtMs: t0 })];
    act(() => root.render(<ChannelView space="s1" channel={{ ...channel }} />));
    expect(Array.from(host.querySelectorAll('.ck-spaces-day')).map((d) => d.textContent)).toEqual(['Yesterday', 'Today']);
    expect(Array.from(host.querySelectorAll('li.ck-spaces-msg')).map((r) => r.classList.contains('is-grouped'))).toEqual([false, false]);
  });

  it('keeps "You deleted this message." for my own deleted line', () => {
    state.messages = [msg({ id: 'd', isMine: true, deleted: { byName: 'Ren', byOwner: false }, text: '' })];
    act(() => root.render(<ChannelView space="s1" channel={channel} />));
    expect(host.querySelector('.ck-spaces-msg-text')?.textContent).toBe('You deleted this message.');
  });
});
