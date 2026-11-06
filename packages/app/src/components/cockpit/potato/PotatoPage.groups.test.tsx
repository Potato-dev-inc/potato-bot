// @vitest-environment jsdom
// Screen-level test (plan 12, G9): the page with the group switch on shows Groups, makes a group with the New group
// sheet, opens it, sends a message and sees two bots answer by name (G10). Without the switch the page has no Groups at all.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakeCompanionTransport } from '../../../lib/companion';
import { fakePotatoTransport, type PotatoState } from '../../../lib/potato';
import { fakeRoomsTransport, type FakeRooms } from '../../../lib/potato-rooms';
import { usePotatoState } from '../../../hooks/usePotato';
import { PotatoPage } from './PotatoPage';
import { bot, typeInto } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

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

const transport = () =>
  fakePotatoTransport({
    bots: [bot({ id: 'fish', name: 'Fish' }), bot({ id: 'potato', name: 'Potato' })],
    messages: { fish: [], potato: [] },
  });

function Harness({ groupsOn, roomsTransport, potatoTransport }: { groupsOn: boolean; roomsTransport: FakeRooms; potatoTransport: ReturnType<typeof transport> }) {
  const potato = usePotatoState(potatoTransport);
  if (!potato.state) return null;
  return (
    <PotatoPage
      state={potato.state as PotatoState}
      version={potato.version}
      transport={potatoTransport}
      companionTransport={fakeCompanionTransport({ companions: [] })}
      companions={[]}
      openBotId={null}
      onOpenBot={() => {}}
      onChanged={() => potato.refresh()}
      onCompanionsChanged={() => {}}
      onNotify={() => {}}
      onOpenSettings={() => {}}
      groupsOn={groupsOn}
      roomsTransport={roomsTransport}
    />
  );
}

async function mount(groupsOn: boolean, rooms: FakeRooms = fakeRoomsTransport()) {
  const potatoTransport = transport();
  await act(async () => root.render(<Harness groupsOn={groupsOn} roomsTransport={rooms} potatoTransport={potatoTransport} />));
  await act(async () => {});
  return rooms;
}

function buttonNamed(label: string): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === label) as HTMLButtonElement | undefined;
}

function setInput(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

// The group chat and its sheet load lazily, so wait until the screen shows the piece the test needs.
async function until(check: () => boolean) {
  for (let i = 0; i < 200 && !check(); i++) await act(async () => new Promise((r) => setTimeout(r, 5)));
}

describe('PotatoPage with bot groups', () => {
  it('shows no Groups block while the switch is off', async () => {
    await mount(false);
    expect(host.textContent).toContain('Fish');
    expect(host.textContent).not.toContain('Groups');
    expect(buttonNamed('New group')).toBeUndefined();
  });

  it('shows Groups, makes a group, and opens it', async () => {
    const rooms = await mount(true);
    expect(host.textContent).toContain('Groups');

    await act(async () => buttonNamed('New group')?.click());
    await until(() => host.querySelector('[role="dialog"]') !== null);
    const dialog =host.querySelector('[role="dialog"]') as HTMLElement | null;
    expect(dialog).not.toBeNull();

    await act(async () => setInput(host.querySelector('[role="dialog"] input[type="text"]') as HTMLInputElement, 'Weekend'));
    const boxes = host.querySelectorAll<HTMLInputElement>('[role="dialog"] input[type="checkbox"]');
    await act(async () => boxes[0].click());
    await act(async () => boxes[1].click());
    await act(async () => buttonNamed('Make group')?.click());
    await until(() => host.querySelector('[aria-label="Group chat Weekend"]') !== null);

    expect(rooms.rooms.map((r) => r.name)).toEqual(['Weekend']);
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    const thread = host.querySelector('[aria-label="Group chat Weekend"]');
    expect(thread?.textContent).toContain('Weekend');
    expect(thread?.textContent).toContain('2 bots');
    expect(host.querySelector('textarea')?.getAttribute('placeholder')).toBe('Message the group');
    expect(host.querySelector('[data-room-id]')?.getAttribute('aria-pressed')).toBe('true');
  });

  it('opens a group, sends a message and shows both bots answering with their names', async () => {
    const rooms = fakeRoomsTransport({ names: { fish: 'Fish', potato: 'Potato' }, everyone: true });
    await rooms.create('Weekend', ['fish', 'potato']);
    await mount(true, rooms);
    await act(async () => (host.querySelector('[data-room-id]') as HTMLButtonElement).click());
    await until(() => host.textContent?.includes('Say hi.') ?? false);
    expect(host.textContent).toContain('Say hi. Call a bot with @ and its name, for example @Fish.');

    typeInto(host.querySelector('textarea') as HTMLTextAreaElement, '@everyone hi');
    await act(async () => (host.querySelector('button.ck-potato-send') as HTMLButtonElement).click());
    for (let i = 0; i < 4; i++) await act(async () => {});

    expect(rooms.sent).toEqual(['@everyone hi']);
    const rowText = Array.from(host.querySelectorAll('ol.ck-potato-rows > li')).map((li) => li.textContent ?? '');
    expect(rowText.some((t) => t.includes('@everyone hi'))).toBe(true);
    const fish = rowText.find((t) => t.includes('Got it.'));
    const potato = rowText.find((t) => t.includes('Me too.'));
    expect(fish).toContain('Fish');
    expect(potato).toContain('Potato');
    expect(Array.from(host.querySelectorAll('.ck-potato-roomline-from')).map((n) => n.textContent)).toEqual(['Fish', 'Potato']);
    // the person's own line shows no name
    expect(rowText.find((t) => t.includes('@everyone hi'))).not.toContain('You');
  });

  it('closes a group that is deleted somewhere else, and says so', async () => {
    const rooms = fakeRoomsTransport({ names: { fish: 'Fish', potato: 'Potato' } });
    const made = await rooms.create('Weekend', ['fish', 'potato']);
    await mount(true, rooms);
    await act(async () => (host.querySelector('[data-room-id]') as HTMLButtonElement).click());
    await act(async () => {});
    expect(host.querySelector('[aria-label="Group chat Weekend"]')).not.toBeNull();
    await act(async () => rooms.remove(made.id));
    for (let i = 0; i < 3; i++) await act(async () => {});
    expect(host.querySelector('[aria-label="Group chat Weekend"]')).toBeNull();
    expect(host.querySelector('textarea[placeholder="Message the group"]')).toBeNull();
  });
});
