// @vitest-environment jsdom
// Emoji reactions, through the real Potato page: pick one from a message's React button, it shows under the message and
// the app command is called; pick it again and it goes; only the fixed set is offered; a group line has no React button.
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakeCompanionTransport } from '../../../lib/companion';
import { REACTIONS, fakePotatoTransport, type FakePotatoTransport, type PotatoState } from '../../../lib/potato';
import { usePotatoState } from '../../../hooks/usePotato';
import { PotatoPage } from './PotatoPage';
import { bot, msg, you } from './test-helpers';

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

function Harness({ transport }: { transport: FakePotatoTransport }) {
  const potato = usePotatoState(transport);
  const [openId, setOpenId] = useState<string | null>(null);
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
      onChanged={() => potato.refresh()}
      onCompanionsChanged={() => {}}
      onNotify={() => {}}
      onOpenSettings={() => {}}
    />
  );
}

async function mount() {
  const transport = fakePotatoTransport({ bots: [bot()], messages: { fish: [you('hello'), msg({ text: 'Fish heard: hello' })] } });
  await act(async () => root.render(<Harness transport={transport} />));
  await act(async () => {});
  return transport;
}

const reactButton = (line: Element) => line.querySelector<HTMLButtonElement>('button[aria-label="React to message"]');
const botLine = () => host.querySelector('.ck-potato-line--bot')!;
const chips = (line: Element) => [...line.querySelectorAll<HTMLButtonElement>('.ck-potato-reaction')];

describe('reactions in the bot thread', () => {
  it('offers exactly the fixed set, and a pick shows under the message and calls the app command', async () => {
    const transport = await mount();
    const line = botLine();
    expect(chips(line)).toHaveLength(0);
    await act(async () => reactButton(line)!.click());
    const picker = line.querySelector('[role="menu"][aria-label="Pick a reaction"]')!;
    expect(picker).not.toBeNull();
    expect([...picker.querySelectorAll('button')].map((b) => b.textContent)).toEqual([...REACTIONS]);

    await act(async () => picker.querySelectorAll('button')[3].click());
    await act(async () => {});
    const call = transport.calls.find((c) => c.method === 'toggleReaction')!;
    expect(call.args).toEqual(['fish', expect.stringMatching(/^m-/), '🎉']);
    expect(line.querySelector('[role="menu"]')).toBeNull();
    const shown = chips(botLine());
    expect(shown.map((c) => c.textContent)).toEqual(['🎉']);
    expect(shown[0].getAttribute('aria-pressed')).toBe('true');
  });

  it('toggles off by pressing the chip again, and stacks a second emoji', async () => {
    const transport = await mount();
    await act(async () => reactButton(botLine())!.click());
    await act(async () => botLine().querySelectorAll<HTMLButtonElement>('[role="menu"] button')[0].click());
    await act(async () => {});
    await act(async () => reactButton(botLine())!.click());
    await act(async () => botLine().querySelectorAll<HTMLButtonElement>('[role="menu"] button')[5].click());
    await act(async () => {});
    expect(chips(botLine()).map((c) => c.textContent)).toEqual(['👍', '🙏']);

    await act(async () => chips(botLine())[0].click());
    await act(async () => {});
    expect(chips(botLine()).map((c) => c.textContent)).toEqual(['🙏']);
    expect(transport.calls.filter((c) => c.method === 'toggleReaction')).toHaveLength(3);
  });

  it('shows a reaction the app already holds, on the right message only', async () => {
    const transport = fakePotatoTransport({ bots: [bot()], messages: { fish: [you('hello'), msg({ text: 'Fish heard: hello', reactions: ['❤️'] })] } });
    await act(async () => root.render(<Harness transport={transport} />));
    await act(async () => {});
    expect(chips(botLine()).map((c) => c.textContent)).toEqual(['❤️']);
    expect(chips(host.querySelector('.ck-potato-line--you')!)).toHaveLength(0);
  });
});
