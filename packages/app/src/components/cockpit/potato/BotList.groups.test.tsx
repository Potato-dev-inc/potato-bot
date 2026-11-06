// @vitest-environment jsdom
// The bot list's Groups block: drawn only with the group switch on, and the Bots header appears beside it.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { RoomView } from '../../../lib/potato-rooms';
import { BotList } from './BotList';
import { bot } from './test-helpers';

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

const room: RoomView = {
  id: 'r000000000001',
  name: 'Weekend',
  bots: [
    { id: 'fish', name: 'Fish' },
    { id: 'potato', name: 'Potato' },
  ],
  lastMs: 1,
  unread: 0,
  typing: [],
};

async function render(groupsOn: boolean) {
  await act(async () =>
    root.render(
      <BotList
        bots={[bot({ id: 'fish', name: 'Fish' })]}
        avatars={new Map()}
        openId={null}
        onOpen={() => {}}
        onNew={() => {}}
        candidates={[]}
        onUseCompanion={() => {}}
        groupsOn={groupsOn}
        rooms={[room]}
        onOpenRoom={() => {}}
        onNewGroup={() => {}}
      />,
    ),
  );
}

describe('BotList groups', () => {
  it('shows no Groups block while the switch is off', async () => {
    await render(false);
    expect(host.textContent).not.toContain('Groups');
    expect(host.textContent).not.toContain('Weekend');
  });

  it('shows Groups, the room with its bot count, then Bots when the switch is on', async () => {
    await render(true);
    const text = host.textContent ?? '';
    expect(text).toContain('Groups');
    expect(text).toContain('Weekend');
    expect(text).toContain('2 bots');
    // The view switch already says Bots at the top; the Bots header comes after the group rows.
    expect(text.indexOf('Bots', text.indexOf('2 bots'))).toBeGreaterThan(text.indexOf('2 bots'));
    expect(host.querySelector('[data-room-id="r000000000001"]')).not.toBeNull();
  });
});
