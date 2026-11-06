// @vitest-environment jsdom
// The live step lists inside the real RoomThread: one list per working bot, labelled with the bot's name; a bot that is
// typing with no steps keeps the simple row; a list goes running, done, folded.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeRoomsTransport, type RoomView } from '../../../lib/potato-rooms';
import type { WorkRunView, WorkStepView } from '../../../lib/potato-work';
import { RoomThread } from './RoomThread';
import { bot } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  localStorage.clear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const NOW = new Date(2026, 10, 5, 15, 0).getTime();
const s = (id: string, tool: string, status: WorkStepView['status'], a: number, b: number | null = null): WorkStepView => ({ id, tool, status, startedMs: a, endedMs: b, log: [] });
const runOf = (steps: WorkStepView[], endedMs: number | null = null): WorkRunView => ({ startedMs: NOW - 9000, steps, failed: false, endedMs });

async function draw(room: RoomView) {
  const rooms = fakeRoomsTransport({ names: { fish: 'Fish', potato: 'Potato' } });
  const potato = fakePotatoTransport({ bots: [bot({ id: 'fish', name: 'Fish' }), bot({ id: 'potato', name: 'Potato' })], messages: {} });
  await act(async () => root.render(<RoomThread room={room} transport={rooms} potato={potato} bots={(await potato.state()).bots} onDeleted={() => {}} onNotify={() => {}} now={NOW} />));
  await act(async () => {});
}
const base: RoomView = { id: 'r-1', name: 'Weekend', bots: [{ id: 'fish', name: 'Fish' }, { id: 'potato', name: 'Potato' }], lastMs: NOW, unread: 0, typing: [] };

describe('RoomThread with live steps', () => {
  it('gives each working bot its own list with its name, and a typing bot with no steps the simple row', async () => {
    await draw({ ...base, typing: ['Fish', 'Potato'], work: [{ bot: 'Fish', run: runOf([s('a', 'computer_observe', 'running', NOW - 2000)]) }] });
    const lists = host.querySelectorAll('[data-testid="work-steps"]');
    expect(lists).toHaveLength(1);
    expect(lists[0].querySelector('.ck-work-owner')?.textContent).toBe('Fish');
    expect(host.textContent).not.toContain('Fish is typing');
    expect(host.textContent).toContain('Potato is typing…');
  });

  it('goes running, done, folded for a bot', async () => {
    await draw({ ...base, typing: ['Potato'], work: [{ bot: 'Potato', run: runOf([s('a', 'browser_navigate', 'running', NOW - 2000)]) }] });
    expect(host.querySelector('.ck-work-title')?.textContent?.replace(/ /g, ' ')).toBe('Opened a page');
    expect(host.querySelectorAll('.ck-work-arc')).toHaveLength(1);

    const done = runOf([s('a', 'browser_navigate', 'done', NOW - 9000, NOW - 4000)], NOW - 4000);
    await draw({ ...base, typing: [], work: [{ bot: 'Potato', run: done }] });
    expect(host.querySelector('.ck-work-title')?.textContent?.replace(/ /g, ' ')).toBe('Worked for 5s · 1 step');
    expect(host.querySelector('.ck-work-list')).toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>('.ck-work-head')!.click());
    expect(host.querySelectorAll('.ck-work-step')).toHaveLength(1);
  });

  it('lists two working bots, each with its own name', async () => {
    await draw({
      ...base,
      typing: ['Fish', 'Potato'],
      work: [
        { bot: 'Fish', run: runOf([s('a', 'computer_observe', 'running', NOW)]) },
        { bot: 'Potato', run: runOf([s('b', 'browser_act', 'running', NOW)]) },
      ],
    });
    expect([...host.querySelectorAll('.ck-work-owner')].map((e) => e.textContent)).toEqual(['Fish', 'Potato']);
  });
});
