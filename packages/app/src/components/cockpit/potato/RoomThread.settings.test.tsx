// @vitest-environment jsdom
// The group settings screen, through the real RoomThread: open it from the menu, rename, choose Listen or Mentions, add and
// remove members, switch a member off in this group only, write instructions, name the shared folder. Save sends it all in
// one go; a bad folder or every bot off keeps Save off; a failed save shows its sentence and the screen stays open.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeRoomsTransport, type FakeRooms, type RoomView } from '../../../lib/potato-rooms';
import { RoomThread } from './RoomThread';
import { bot, buttonByText } from './test-helpers';

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

const NAMES = { fish: 'Fish', potato: 'Potato', mochi: 'Mochi' };
const NOW = new Date(2026, 10, 7, 15, 0).getTime();

async function show(): Promise<{ rooms: FakeRooms; room: RoomView; reread: () => Promise<void> }> {
  const rooms = fakeRoomsTransport({ names: NAMES });
  const room = await rooms.create('Trip', ['fish', 'potato']);
  const potato = fakePotatoTransport({ bots: Object.entries(NAMES).map(([id, name]) => bot({ id, name })), messages: {} });
  const bots = (await potato.state()).bots;
  const render = (r: RoomView) => root.render(<RoomThread room={r} transport={rooms} potato={potato} bots={bots} onDeleted={() => {}} onNotify={() => {}} now={NOW} />);
  await act(async () => render(room));
  await act(async () => {});
  // the page re-reads the group when the app says it changed: here the test does it by hand
  const reread = async () => {
    const fresh = (await rooms.list()).find((x) => x.id === room.id);
    if (fresh) await act(async () => render(fresh));
  };
  return { rooms, room, reread };
}

const click = (el: Element | undefined) => act(async () => (el as HTMLElement).click());
const menuItem = (label: string) => [...host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === label);
const dialog = () => host.querySelector<HTMLElement>('[role="dialog"][aria-label="Group settings"]');
const setValue = (el: HTMLInputElement | HTMLTextAreaElement, value: string) => {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value')!.set!.call(el, value);
  act(() => {
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
};

async function open() {
  const setup = await show();
  await click(host.querySelector('[aria-label="Group menu"]')!);
  await click(menuItem('Group settings'));
  expect(dialog()).not.toBeNull();
  return setup;
}

describe('RoomThread: group settings', () => {
  it('opens from the menu and shows what the group has now', async () => {
    await open();
    const d = dialog()!;
    expect((d.querySelector('input[type="text"]') as HTMLInputElement).value).toBe('Trip');
    expect((d.querySelector('input[type="radio"][name="room-mode"]') as HTMLInputElement).checked).toBe(true);
    expect(d.textContent).toContain('Fish');
    expect(d.textContent).toContain('Potato');
    expect(d.textContent).toContain('/home/pm/groups/r');
    expect(d.querySelector('[aria-label="Fish answers in this group"]')).not.toBeNull();
  });

  it('Save sends the name, the mode, who is off, the instructions and the folder together', async () => {
    const { rooms, room, reread } = await open();
    const d = dialog()!;
    setValue(d.querySelector('input[type="text"]')!, 'Trip 2026');
    await click(d.querySelectorAll('input[type="radio"][name="room-mode"]')[1]);
    await click(d.querySelector('[aria-label="Potato answers in this group"]')!);
    setValue(d.querySelector('textarea')!, 'Answer in French.');
    setValue(d.querySelectorAll<HTMLInputElement>('input[type="text"]')[1]!, 'Trip-2026');
    await click(buttonByText(d, 'Save'));
    expect(rooms.settingsSaved).toEqual([{ roomId: room.id, instructions: 'Answer in French.', folder: 'Trip-2026', mode: 'mentions', offBots: ['potato'] }]);
    const saved = rooms.rooms[0]!;
    expect(saved.name).toBe('Trip 2026');
    expect(saved.mode).toBe('mentions');
    expect(saved.folderPath).toBe('/home/pm/groups/trip-2026');
    expect(dialog()).toBeNull();
    await reread();
    // the header says so
    expect(host.textContent).toContain('Mentions only');
    expect(host.textContent).toContain('1 off');
  });

  it('adds a bot and removes a bot; a group never goes under 2', async () => {
    const { rooms, room } = await open();
    const d = dialog()!;
    // two members: Remove is off for both
    for (const b of [...d.querySelectorAll<HTMLButtonElement>('button')].filter((x) => x.textContent === 'Remove')) expect(b.disabled).toBe(true);
    const select = d.querySelector<HTMLSelectElement>('select[aria-label="Add a bot"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')!.set!.call(select, 'mochi');
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await click(buttonByText(d, 'Add'));
    expect(d.querySelector('[aria-label="Mochi answers in this group"]')).not.toBeNull();
    await click(d.querySelector('[aria-label="Remove Fish"]')!);
    expect(d.querySelector('[aria-label="Fish answers in this group"]')).toBeNull();
    await click(buttonByText(d, 'Save'));
    expect(rooms.rooms.find((r) => r.id === room.id)!.bots.map((b) => b.id)).toEqual(['potato', 'mochi']);
  });

  it('a folder that is a path, or every bot switched off, keeps Save off and says why', async () => {
    await open();
    const d = dialog()!;
    const save = buttonByText(d, 'Save')!;
    expect(save.disabled).toBe(false);
    setValue(d.querySelectorAll<HTMLInputElement>('input[type="text"]')[1]!, '../etc');
    expect(save.disabled).toBe(true);
    expect(d.textContent).toContain('Use letters, numbers, - or _ (up to 40).');
    setValue(d.querySelectorAll<HTMLInputElement>('input[type="text"]')[1]!, '');
    expect(save.disabled).toBe(false);
    await click(d.querySelector('[aria-label="Fish answers in this group"]')!);
    await click(d.querySelector('[aria-label="Potato answers in this group"]')!);
    expect(save.disabled).toBe(true);
    expect(d.textContent).toContain('Keep at least one bot on in this group.');
  });

  it('a failed save shows the sentence, keeps the screen open and changes nothing', async () => {
    const { rooms } = await open();
    rooms.failNext('setSettings', 'POTATO_ROOM_IO: Could not save the group. Try again.');
    const d = dialog()!;
    setValue(d.querySelector('textarea')!, 'Be brief.');
    await click(buttonByText(d, 'Save'));
    expect(d.querySelector('[role="alert"]')!.textContent).toContain('Could not save the group');
    expect(dialog()).not.toBeNull();
    expect(rooms.settingsSaved).toHaveLength(0);
    expect(rooms.rooms[0]!.instructions).toBe('');
  });

  it('Escape and Cancel close it without saving', async () => {
    const { rooms } = await open();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(dialog()).toBeNull();
    await click(host.querySelector('[aria-label="Group menu"]')!);
    await click(menuItem('Group settings'));
    await click(buttonByText(dialog()!, 'Cancel'));
    expect(dialog()).toBeNull();
    expect(rooms.settingsSaved).toHaveLength(0);
  });
});
