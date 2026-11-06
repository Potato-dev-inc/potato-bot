// @vitest-environment jsdom
// A hand-off in a group, through the real RoomThread: the card says who gave the task, to whom, what it is, and whether it was
// passed on. A hand-off the limit stopped says so. The card is not drawn twice (no plain bubble for the same line).
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeRoomsTransport } from '../../../lib/potato-rooms';
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
  vi.restoreAllMocks();
});

const NOW = new Date(2026, 10, 7, 15, 0).getTime();

async function show() {
  const rooms = fakeRoomsTransport({ names: { fish: 'Fish', potato: 'Potato' } });
  const room = await rooms.create('Trip', ['fish', 'potato']);
  const potato = fakePotatoTransport({ bots: [bot({ id: 'fish', name: 'Fish' }), bot({ id: 'potato', name: 'Potato' })], messages: {} });
  await act(async () => root.render(<RoomThread room={room} transport={rooms} potato={potato} bots={(await potato.state()).bots} onDeleted={() => {}} onNotify={() => {}} now={NOW} />));
  await act(async () => {});
  return { rooms, room };
}

describe('RoomThread: hand-off cards', () => {
  it('shows who gave the task, to whom, what it is, and that it was passed on', async () => {
    const { rooms, room } = await show();
    await act(async () => {
      rooms.addLine(room.id, {
        from: { kind: 'bot', id: 'fish', name: 'Fish' },
        text: '@Potato look up the weather',
        hop: 1,
        handoff: { toId: 'potato', toName: 'Potato', task: 'look up the weather', passed: true },
      });
    });
    const card = host.querySelector('[role="group"][aria-label="Hand-off"]');
    expect(card).not.toBeNull();
    expect(card!.textContent).toContain('Fish');
    expect(card!.textContent).toContain('Potato');
    expect(card!.textContent).toContain('look up the weather');
    expect(card!.textContent).toContain('Passed on');
    // drawn as a card only: the "@Potato ..." line is not also a bubble
    expect(host.textContent!.split('look up the weather').length - 1).toBe(1);
    expect(host.textContent).not.toContain('@Potato');
  });

  it('a hand-off the hop limit stopped says it was not passed on', async () => {
    const { rooms, room } = await show();
    await act(async () => {
      rooms.addLine(room.id, {
        from: { kind: 'bot', id: 'potato', name: 'Potato' },
        text: '@Fish and again',
        hop: 3,
        handoff: { toId: 'fish', toName: 'Fish', task: 'and again', passed: false },
      });
    });
    const card = host.querySelector('[role="group"][aria-label="Hand-off"]')!;
    expect(card.textContent).toContain('Not passed on');
    expect(card.className).toContain('is-stopped');
  });

  it('an ordinary bot line is still a bubble, not a card', async () => {
    const { rooms, room } = await show();
    await act(async () => {
      rooms.addLine(room.id, { from: { kind: 'bot', id: 'fish', name: 'Fish' }, text: 'Morning!', hop: 1 });
    });
    expect(host.querySelector('[aria-label="Hand-off"]')).toBeNull();
    expect(host.textContent).toContain('Morning!');
  });
});
