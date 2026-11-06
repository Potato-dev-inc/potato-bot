// @vitest-environment jsdom
// The way in and the way back: Ctrl+K lists "Search chats"; the page searches every chat; a result opens that bot's thread,
// reads it back far enough to contain the message, and marks the message.
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakeCompanionTransport } from '../../../lib/companion';
import { fakePotatoTransport, type FakePotatoTransport, type PotatoState } from '../../../lib/potato';
import { fakeSearchTransport, type SearchHit } from '../../../lib/potato-search';
import { usePotatoState } from '../../../hooks/usePotato';
import { PotatoPage } from './PotatoPage';
import './SearchChatsPage';
import { bot, msg } from './test-helpers';

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

const settle = () => act(async () => new Promise((r) => setTimeout(r, 5)));

function Harness({ transport, search }: { transport: FakePotatoTransport; search: ReturnType<typeof fakeSearchTransport> }) {
  const potato = usePotatoState(transport);
  const [openId, setOpenId] = useState<string | null>('fish');
  if (!potato.state) return null;
  return (
    <PotatoPage
      state={potato.state as PotatoState}
      version={potato.version}
      transport={transport}
      companionTransport={fakeCompanionTransport({ companions: [] })}
      companions={[]}
      openBotId={openId}
      onOpenBot={setOpenId}
      onChanged={potato.refresh}
      onCompanionsChanged={() => {}}
      onNotify={() => {}}
      onOpenSettings={() => {}}
      searchTransport={search}
    />
  );
}

function ctrlK() {
  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true, cancelable: true }));
  });
}
function type(el: HTMLInputElement, text: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, text);
  act(() => {
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('Search chats inside Potato Bot', () => {
  it('Ctrl+K has a Search chats entry that opens the page, and a long-ago private message is read back and marked', async () => {
    // 251 messages; the one searched for is the 251st from the end: older than the 200 a thread normally reads.
    const old = msg({ text: 'the Lisbon budget is 900', from: { kind: 'you' } });
    const thread = [old, ...Array.from({ length: 250 }, (_, i) => msg({ text: `filler ${i}` }))];
    const transport = fakePotatoTransport({ bots: [bot({ id: 'fish', name: 'Fish' })], messages: { fish: thread } });
    const hit: SearchHit = { kind: 'bot', ownerId: 'fish', ownerName: 'Fish', messageId: old.id, at: old.at, speaker: 'You', snippet: 'the Lisbon budget is 900', marks: [[4, 10]], fromEnd: 250 };
    const search = fakeSearchTransport([hit]);
    act(() => {
      root.render(<Harness transport={transport} search={search} />);
    });
    await settle();
    expect(host.textContent).not.toContain('the Lisbon budget is 900');

    ctrlK();
    const palette = host.querySelector<HTMLInputElement>('input.ck-potato-palette-input')!;
    type(palette, 'search');
    expect([...host.querySelectorAll('.ck-potato-palette-label')].map((l) => l.textContent)).toEqual(['Search chats']);
    act(() => {
      palette.closest('.ck-potato-palette')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    });
    await settle();
    expect(host.querySelector('.ck-search')).not.toBeNull();

    type(host.querySelector<HTMLInputElement>('input.ck-search-input')!, 'lisbon');
    await act(async () => new Promise((r) => setTimeout(r, 300)));
    await settle();
    act(() => (host.querySelector('.ck-search-hit') as HTMLElement).click());
    await settle();
    await settle();

    expect(host.querySelector('.ck-search')).toBeNull();
    const reads = transport.calls.filter((c) => c.method === 'messages').map((c) => c.args[1]);
    expect(reads).toContain(280);
    const row = host.querySelector(`[data-message-id="${old.id}"]`);
    expect(row).not.toBeNull();
    expect(row?.classList.contains('is-jumped')).toBe(true);
  });
});
