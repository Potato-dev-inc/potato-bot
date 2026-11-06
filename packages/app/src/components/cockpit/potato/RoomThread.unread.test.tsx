// @vitest-environment jsdom
// The "New" line in a group thread, through the real RoomThread: it sits above the first unread bot line, stays after
// opening reads the group, and is absent when nothing is unread.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeRoomsTransport, type FakeRooms, type RoomView } from '../../../lib/potato-rooms';
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
const fish = { kind: 'bot' as const, id: 'fish', name: 'Fish' };
const me = { kind: 'person' as const, id: 'me', name: 'You' };

async function open(unread: number, texts: Array<[string, 'bot' | 'me']>): Promise<{ rooms: FakeRooms; room: RoomView }> {
  const rooms = fakeRoomsTransport({ names: { fish: 'Fish', potato: 'Potato' } });
  const created = await rooms.create('Weekend', ['fish', 'potato']);
  texts.forEach(([text, who], i) => rooms.addLine(created.id, { from: who === 'bot' ? fish : me, text, at: NOW - (texts.length - i) * 1000 }));
  const room: RoomView = { ...created, unread };
  const potato = fakePotatoTransport({ bots: [bot({ id: 'fish', name: 'Fish' })], messages: {} });
  await act(async () => root.render(<RoomThread room={room} transport={rooms} potato={potato} bots={(await potato.state()).bots} onDeleted={() => {}} onNotify={() => {}} now={NOW} />));
  await act(async () => {});
  await act(async () => {});
  return { rooms, room };
}

const divider = () => host.querySelector('.ck-potato-sep--unread');

describe('RoomThread unread line', () => {
  it('puts the New line above the first unread bot line', async () => {
    await open(2, [['hello', 'me'], ['old reply', 'bot'], ['first new', 'bot'], ['second new', 'bot']]);
    const line = divider();
    expect(line).not.toBeNull();
    const next = line!.nextElementSibling as HTMLElement;
    expect(next.textContent).toContain('first new');
    expect(line!.previousElementSibling?.textContent).toContain('old reply');
  });

  it('shows no line when nothing is unread', async () => {
    await open(0, [['hello', 'me'], ['a reply', 'bot']]);
    expect(divider()).toBeNull();
  });

  it('keeps the line after opening reads the group, after the app reports it read', async () => {
    const { rooms, room } = await open(1, [['hello', 'me'], ['old', 'bot'], ['new one', 'bot']]);
    expect(rooms.read).toContain(room.id);
    expect(divider()).not.toBeNull();
    // the app now reports the group read: the line stays
    const potato = fakePotatoTransport({ bots: [bot({ id: 'fish', name: 'Fish' })], messages: {} });
    await act(async () => root.render(<RoomThread room={{ ...room, unread: 0 }} transport={rooms} potato={potato} bots={(await potato.state()).bots} onDeleted={() => {}} onNotify={() => {}} now={NOW} />));
    await act(async () => {});
    expect(divider()).not.toBeNull();
  });
});
