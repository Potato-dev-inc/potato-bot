// @vitest-environment jsdom
// The group chat (plan 12, slice G10), through the real RoomThread with the in-memory rooms and the fake bots.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeRoomsTransport, ROOM_NOTE_HOP, type FakeRooms, type RoomView } from '../../../lib/potato-rooms';
import { RoomThread, type RoomThreadProps } from './RoomThread';
import { bot, buttonByText, typeInto } from './test-helpers';

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

const NAMES = { fish: 'Fish', potato: 'Potato' };
const NOW = new Date(2026, 10, 5, 15, 0).getTime();

interface Setup {
  rooms: FakeRooms;
  room: RoomView;
  potato: ReturnType<typeof fakePotatoTransport>;
  onDeleted: ReturnType<typeof vi.fn>;
  onGone: ReturnType<typeof vi.fn>;
  onNotify: ReturnType<typeof vi.fn>;
}

async function show(over: Partial<RoomThreadProps> = {}, options: { everyone?: boolean; states?: Record<string, 'ready' | 'idle'> } = {}): Promise<Setup> {
  const rooms = fakeRoomsTransport({ names: NAMES, everyone: options.everyone });
  const room = await rooms.create('Weekend', ['fish', 'potato']);
  const states = options.states ?? {};
  const potato = fakePotatoTransport({
    bots: [
      bot({ id: 'fish', name: 'Fish', brainState: states.fish ?? 'ready' }),
      bot({ id: 'potato', name: 'Potato', brainState: states.potato ?? 'ready' }),
      bot({ id: 'mochi', name: 'Mochi', brainState: 'idle' }),
    ],
    messages: {},
    artifacts: { items: [{ id: 'a-png', name: 'chart.png', bot: 'potato', kind: 'image', size: 900, created: NOW, version: 1, file: 'a-png.png' }] },
  });
  const onDeleted = vi.fn();
  const onGone = vi.fn();
  const onNotify = vi.fn();
  const props: RoomThreadProps = {
    room,
    transport: rooms,
    potato,
    bots: (await potato.state()).bots,
    onDeleted,
    onGone,
    onNotify,
    now: NOW,
    ...over,
  };
  await act(async () => root.render(<RoomThread {...props} />));
  await act(async () => {});
  return { rooms, room: props.room, potato, onDeleted, onGone, onNotify };
}

const textarea = () => host.querySelector('textarea') as HTMLTextAreaElement;
const press = (el: Element, key: string, shiftKey = false) =>
  act(async () => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key, shiftKey, bubbles: true, cancelable: true }));
  });
const add = (rooms: FakeRooms, roomId: string, line: Parameters<FakeRooms['addLine']>[1]) =>
  act(async () => {
    rooms.addLine(roomId, line);
  });
const flush = async (n = 4) => {
  for (let i = 0; i < n; i++) await act(async () => {});
};
const rowTexts = () => Array.from(host.querySelectorAll('ol.ck-potato-rows > li')).map((li) => li.textContent ?? '');
const openMenu = async () => {
  await act(async () => (host.querySelector('button[aria-label="Group menu"]') as HTMLButtonElement).click());
};
const pickMenu = async (label: string) => {
  const item = Array.from(host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')).find((b) => b.textContent === label);
  await act(async () => item?.click());
};

describe('RoomThread', () => {
  it('shows the empty state with the first bot name', async () => {
    await show();
    expect(host.textContent).toContain('Say hi. Call a bot with @ and its name, for example @Fish. Say everyone to hear from all of them.');
    expect(host.querySelector('textarea')?.getAttribute('placeholder')).toBe('Message the group');
    expect(host.querySelector('h1')?.textContent).toBe('Weekend');
    expect(host.textContent).toContain('2 bots');
  });

  it('sends with Enter and shows the reply with the bot name above it', async () => {
    const { rooms } = await show();
    typeInto(textarea(), 'hi');
    await press(textarea(), 'Enter');
    await flush();
    expect(rooms.sent).toEqual(['hi']);
    expect(host.textContent).toContain('hi');
    const reply = Array.from(host.querySelectorAll('li.ck-potato-line')).find((li) => li.textContent?.includes('Got it.'));
    expect(reply?.querySelector('.ck-potato-roomline-from')?.textContent).toBe('Fish');
    // the person's own line carries no name
    const mine = Array.from(host.querySelectorAll('li.ck-potato-line')).find((li) => li.textContent?.includes('hi') && !li.textContent.includes('Got it.'));
    expect(mine?.querySelector('.ck-potato-roomline-from')).toBeNull();
    expect(textarea().value).toBe('');
  });

  it('shift enter makes a new line and sends nothing', async () => {
    const { rooms } = await show();
    typeInto(textarea(), 'a\nb');
    await press(textarea(), 'Enter', true);
    await flush(2);
    expect(textarea().value).toBe('a\nb');
    expect(rooms.sent).toEqual([]);
  });

  it('shows who is typing', async () => {
    const { rooms, room } = await show();
    await act(async () => root.render(<RoomThread room={{ ...room, typing: ['Potato'] }} transport={rooms} potato={fakePotatoTransport({ bots: [] })} bots={[]} onDeleted={() => {}} onNotify={() => {}} now={NOW} />));
    expect(host.textContent).toContain('Potato is typing…');
    expect(host.textContent).not.toContain('Say hi.');
  });

  it('lists the group bots and everyone after @, and narrows by what is typed', async () => {
    await show();
    typeInto(textarea(), '@');
    const options = () => Array.from(host.querySelectorAll('[role="option"]')).map((o) => o.textContent);
    expect(options()).toEqual(['@everyone', '@Fish', '@Potato']);
    typeInto(textarea(), '@po');
    expect(options()).toEqual(['@Potato']);
    typeInto(textarea(), '@ev');
    expect(options()).toEqual(['@everyone']);
  });

  it('puts the picked name in the box and sends it as typed', async () => {
    const { rooms } = await show();
    typeInto(textarea(), '@po');
    await press(textarea(), 'Enter');
    expect(textarea().value).toBe('@Potato ');
    typeInto(textarea(), '@Potato hello');
    await press(textarea(), 'Enter');
    await flush();
    expect(rooms.sent).toEqual(['@Potato hello']);
  });

  it('shows a file card from a bot and opens it with that bot', async () => {
    const { rooms, room, potato } = await show();
    const open = vi.spyOn(potato.artifacts, 'open');
    await add(rooms, room.id, { from: { kind: 'bot', id: 'potato', name: 'Potato' }, text: 'Sent a file: chart.png', file: { id: 'a-png', name: 'chart.png', kind: 'image', size: 900 } });
    await flush();
    expect(host.querySelector('[aria-label="chart.png"]')).not.toBeNull();
    expect(open).toHaveBeenCalledWith('potato', 'a-png');
    expect(host.querySelector('.ck-potato-roomline-from')?.textContent).toBe('Potato');
  });

  it('shows a hop 255 line as a grey note, not as something a bot said', async () => {
    const { rooms, room } = await show();
    await add(rooms, room.id, { from: { kind: 'bot', id: 'fish', name: 'Fish' }, text: 'Fish is not running, so it did not get this line.', hop: ROOM_NOTE_HOP });
    await flush();
    const note = host.querySelector('li.ck-potato-note');
    expect(note?.getAttribute('role')).toBe('note');
    expect(note?.textContent).toContain('Fish is not running');
    expect(host.querySelector('.ck-potato-roomline-from')).toBeNull();
    expect(host.querySelector('li.ck-potato-line')).toBeNull();
  });

  it('names a bot once for a run of its lines, and again when another bot speaks', async () => {
    const { rooms, room } = await show();
    const fish = { kind: 'bot' as const, id: 'fish', name: 'Fish' };
    await add(rooms, room.id, { from: fish, text: 'one' });
    await add(rooms, room.id, { from: fish, text: 'two' });
    await add(rooms, room.id, { from: { kind: 'bot', id: 'potato', name: 'Potato' }, text: 'three' });
    await flush();
    expect(Array.from(host.querySelectorAll('.ck-potato-roomline-from')).map((n) => n.textContent)).toEqual(['Fish', 'Potato']);
  });

  it('reads the group again when the app says it changed', async () => {
    const { rooms, room } = await show();
    expect(host.textContent).not.toContain('late news');
    await add(rooms, room.id, { from: { kind: 'bot', id: 'fish', name: 'Fish' }, text: 'late news' });
    await flush();
    expect(host.textContent).toContain('late news');
  });

  it('marks the group read on open', async () => {
    const { rooms, room } = await show();
    expect(rooms.read).toContain(room.id);
  });

  it('asks before deleting, then deletes and says so', async () => {
    const { rooms, onDeleted } = await show();
    await openMenu();
    await pickMenu('Delete group');
    expect(host.textContent).toContain('Delete this group?');
    expect(host.textContent).toContain('The chat is removed. The bots stay.');
    expect(rooms.rooms).toHaveLength(1);
    await act(async () => buttonByText(host, 'Cancel')?.click());
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(rooms.rooms).toHaveLength(1);
    await openMenu();
    await pickMenu('Delete group');
    await act(async () => buttonByText(host.querySelector('[role="dialog"]') as HTMLElement, 'Delete group')?.click());
    await flush();
    expect(onDeleted).toHaveBeenCalledTimes(1);
    expect(rooms.rooms).toHaveLength(0);
  });

  it('refuses an empty name when renaming and renames with a good one', async () => {
    const { rooms } = await show();
    await openMenu();
    await pickMenu('Rename group');
    const field = host.querySelector('input[aria-label="Name"]') as HTMLInputElement;
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      set.call(field, '  ');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => buttonByText(host, 'Save')?.click());
    expect(host.textContent).toContain('Give the group a name.');
    expect(rooms.rooms[0].name).toBe('Weekend');
    await act(async () => {
      set.call(field, 'Trip');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => buttonByText(host, 'Save')?.click());
    await flush();
    expect(rooms.rooms[0].name).toBe('Trip');
    expect(host.querySelector('input[aria-label="Name"]')).toBeNull();
  });

  it('changes the bots of the group with the same checklist', async () => {
    const { rooms } = await show();
    await openMenu();
    await pickMenu('Change bots');
    const dialog = host.querySelector('[role="dialog"]') as HTMLElement;
    expect(dialog.getAttribute('aria-label')).toBe('Change bots');
    expect(dialog.querySelector('input[type="text"]')).toBeNull();
    const boxes = Array.from(dialog.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'));
    expect(boxes.map((b) => b.checked)).toEqual([true, true, false]);
    await act(async () => boxes[2].click());
    await act(async () => buttonByText(dialog, 'Save')?.click());
    await flush();
    expect(rooms.rooms[0].bots.map((b) => b.id)).toEqual(['fish', 'potato', 'mochi']);
    expect(host.querySelector('[role="dialog"]')).toBeNull();
  });

  it('Stop stops every member bot that is running, and no one else', async () => {
    const { potato } = await show({}, { states: { fish: 'ready', potato: 'idle' } });
    await act(async () => buttonByText(host, 'Stop')?.click());
    await flush();
    expect(potato.calls.filter((c) => c.method === 'stopBot').map((c) => c.args[0])).toEqual(['fish']);
  });

  it('Stop stops both bots when both are running, and is not offered when none is', async () => {
    const both = await show();
    await act(async () => buttonByText(host, 'Stop')?.click());
    await flush();
    expect(both.potato.calls.filter((c) => c.method === 'stopBot').map((c) => c.args[0]).sort()).toEqual(['fish', 'potato']);
    act(() => root.unmount());
    root = createRoot(host);
    await show({}, { states: { fish: 'idle', potato: 'idle' } });
    expect(buttonByText(host, 'Stop')).toBeUndefined();
  });

  it('closes gracefully when the group is gone, once', async () => {
    const rooms = fakeRoomsTransport({ names: NAMES });
    const room = await rooms.create('Weekend', ['fish', 'potato']);
    await rooms.remove(room.id);
    const onGone = vi.fn();
    const onDeleted = vi.fn();
    await act(async () =>
      root.render(<RoomThread room={room} transport={rooms} potato={fakePotatoTransport({ bots: [] })} bots={[]} onDeleted={onDeleted} onGone={onGone} onNotify={() => {}} now={NOW} />),
    );
    await flush();
    expect(onGone).toHaveBeenCalledTimes(1);
    expect(onDeleted).not.toHaveBeenCalled();
    expect(host.textContent).not.toContain('POTATO_ROOM');
  });

  it('says a plain sentence, never the code, when a send fails', async () => {
    const { rooms, onNotify } = await show();
    rooms.failNext('send', 'POTATO_ROOM_GONE: This group no longer exists.');
    typeInto(textarea(), 'hi');
    await press(textarea(), 'Enter');
    await flush();
    expect(host.textContent).toContain('This group no longer exists.');
    expect(host.textContent).not.toContain('POTATO_ROOM_GONE');
    expect(onNotify).toHaveBeenCalledWith('This group no longer exists.');
    // the words come back into the box so nothing is lost
    expect(textarea().value).toBe('hi');
  });

  it('says a plain sentence when the lines cannot be read', async () => {
    const rooms = fakeRoomsTransport({ names: NAMES });
    const room = await rooms.create('Weekend', ['fish', 'potato']);
    rooms.failNext('lines', 'POTATO_ROOM_IO');
    await act(async () => root.render(<RoomThread room={room} transport={rooms} potato={fakePotatoTransport({ bots: [] })} bots={[]} onDeleted={() => {}} onNotify={() => {}} now={NOW} />));
    await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Could not save the group. Try again.');
  });
});

describe('RoomThread scrolling', () => {
  const geometry = (el: HTMLElement, scrollHeight: number, clientHeight: number) => {
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => scrollHeight });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => clientHeight });
  };

  it('follows the bottom, leaves a reader alone further up, and jumps back on request', async () => {
    const { rooms, room } = await show();
    const scroller = host.querySelector('.ck-potato-body') as HTMLElement;
    geometry(scroller, 1000, 100);
    await add(rooms, room.id, { from: { kind: 'bot', id: 'fish', name: 'Fish' }, text: 'first' });
    await flush();
    expect(scroller.scrollTop).toBe(1000);

    // the person reads further up
    scroller.scrollTop = 0;
    await act(async () => {
      scroller.dispatchEvent(new Event('scroll', { bubbles: true }));
    });
    expect(buttonByText(host, 'Jump to latest')).toBeDefined();
    await add(rooms, room.id, { from: { kind: 'bot', id: 'fish', name: 'Fish' }, text: 'second' });
    await flush();
    expect(scroller.scrollTop).toBe(0);

    await act(async () => buttonByText(host, 'Jump to latest')?.click());
    expect(scroller.scrollTop).toBe(1000);
    expect(buttonByText(host, 'Jump to latest')).toBeUndefined();
  });
});
