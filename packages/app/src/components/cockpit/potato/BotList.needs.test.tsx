// @vitest-environment jsdom
// The bot list's "Needs you" mark: a bot with a card waiting is pinned on top and its row carries the dot, in the full
// list and in the strip. A bot with no card waiting carries no mark.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AvatarId } from '../../../lib/companion';
import { BotList, type BotListProps } from './BotList';
import { bot } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const NOW = new Date(2026, 10, 6, 15, 0).getTime();
const props = (over: Partial<BotListProps> = {}): BotListProps => ({
  bots: [],
  avatars: new Map<string, AvatarId>(),
  openId: null,
  onOpen: () => {},
  onNew: () => {},
  candidates: [],
  onUseCompanion: () => {},
  now: NOW,
  ...over,
});
const rowOf = (name: string) => [...host.querySelectorAll<HTMLButtonElement>('.ck-potato-conv')].find((b) => b.querySelector('.ck-row-title')?.textContent === name);
const names = () => [...host.querySelectorAll('.ck-potato-conv .ck-row-title')].map((n) => n.textContent);

describe('BotList: Needs you', () => {
  it('a bot with a card waiting is pinned first, with the Needs you line and the needs mark', () => {
    const waiting = bot({ id: 'b', name: 'Beta', openCards: 1, lastLine: 'Shall I publish?' });
    const idle = bot({ id: 'a', name: 'Alpha', openCards: 0, lastLine: 'Done.' });
    act(() => root.render(<BotList {...props({ bots: [idle, waiting] })} />));
    expect(names()).toEqual(['Beta', 'Alpha']);
    const row = rowOf('Beta')!;
    expect(row.classList.contains('is-needs')).toBe(true);
    expect(row.textContent).toContain('Waiting for you');
    expect(rowOf('Alpha')!.classList.contains('is-needs')).toBe(false);
  });

  it('the strip keeps the mark on the face of a bot that needs you', () => {
    const waiting = bot({ id: 'b', name: 'Beta', openCards: 2 });
    act(() => root.render(<BotList {...props({ bots: [waiting], mini: true })} />));
    expect(host.querySelector('.ck-potato-dot--needs')).not.toBeNull();
    expect(host.querySelector('[aria-label="Beta, Needs you"]')).not.toBeNull();
  });

  it('no card waiting: no mark', () => {
    act(() => root.render(<BotList {...props({ bots: [bot({ id: 'a', name: 'Alpha', openCards: 0 })] })} />));
    expect(host.querySelector('.ck-potato-dot--needs')).toBeNull();
    expect(host.textContent).not.toContain('Needs you');
  });
});
