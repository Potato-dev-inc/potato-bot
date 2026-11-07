// @vitest-environment jsdom
// The channel feed with bots: the Bots here line, the call hint, bots off, and hidden bot rows.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceChannel, SpaceDetailBot, SpaceMessage } from '../../lib/spaces';

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

import { ChannelView } from './ChannelView';

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.ch.messages = [];
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const channel: SpaceChannel = { id: 'c1', name: 'general', archived: false, unread: 0 };
const msg = (over: Partial<SpaceMessage> = {}): SpaceMessage => ({
  id: 'm1', channel: 'c1', member: 'mem-sam', authorName: 'Sam', isFish: false, isMine: false, createdAtMs: 0, text: 'hi',
  replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false, ...over,
});
const bots: SpaceDetailBot[] = [
  { id: 'b1', name: 'Ren的fish', ownerName: 'Ren', mine: true, muted: false },
  { id: 'b2', name: 'Sam的cat', ownerName: 'Sam', mine: false, muted: false },
];
const renderBots = (props: { bots?: SpaceDetailBot[]; botsOff?: boolean }) => {
  act(() => {
    root.render(<ChannelView space="s1" channel={channel} {...props} />);
  });
};
const rows = () => Array.from(host.querySelectorAll('li.ck-spaces-msg'));

describe('ChannelView bots', () => {
  it('shows the Bots here line and the composer hint', () => {
    renderBots({ bots });
    expect(host.querySelector('.ck-spaces-botsline')?.textContent).toBe('Bots here: Ren的fish · Sam的cat');
    expect(host.querySelector('.ck-spaces-composer-hint')?.textContent).toBe("Type @ and a bot's name to call it.");
  });

  it('shows neither line when the space has no bots', () => {
    renderBots({ bots: [] });
    expect(host.querySelector('.ck-spaces-botsline')).toBeNull();
    expect(host.querySelector('.ck-spaces-composer-hint')).toBeNull();
  });

  it('leaves a bot I hid out of the line', () => {
    renderBots({ bots: [bots[0], { ...bots[1], muted: true }] });
    expect(host.querySelector('.ck-spaces-botsline')?.textContent).toBe('Bots here: Ren的fish');
  });

  it('shows that bots are off in a channel the rules keep them out of, with no call hint', () => {
    renderBots({ bots, botsOff: true });
    expect(host.querySelector('.ck-spaces-botsline')?.textContent).toBe('Bots are off in this channel.');
    expect(host.querySelector('.ck-spaces-composer-hint')).toBeNull();
  });

  it('hides rows from a bot I hid and keeps the rest', () => {
    state.ch.messages = [
      msg({ id: 'a', text: 'from cat', isFish: true, botId: 'b2', botName: 'Sam的cat', authorName: 'Sam的cat' }),
      msg({ id: 'b', text: 'from fish', isFish: true, botId: 'b1', botName: 'Ren的fish', authorName: 'Ren的fish' }),
      msg({ id: 'c', text: 'from Sam' }),
    ];
    renderBots({ bots: [bots[0], { ...bots[1], muted: true }] });
    expect(rows().map((x) => x.querySelector('.ck-spaces-msg-text')?.textContent)).toEqual(['from fish', 'from Sam']);
  });

  it('shows the bot space name as the author and marks the row', () => {
    state.ch.messages = [msg({ isFish: true, botId: 'b1', botName: 'Ren的fish', authorName: 'Ren的fish', text: 'on it' })];
    renderBots({ bots });
    expect(rows()[0].querySelector('.ck-spaces-msg-author')?.textContent).toBe('Ren的fish');
    expect(rows()[0].classList.contains('is-fish')).toBe(true);
  });
});
