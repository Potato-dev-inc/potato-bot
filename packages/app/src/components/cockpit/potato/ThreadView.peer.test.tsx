// @vitest-environment jsdom
// Peer cards in a bot's private chat, through the real ThreadView: the owner sees that their bot wrote to another of their bots,
// that one was written to, and the answer. Each is a card with the message, and none of them looks like the owner's own line.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport, type PotatoSettings } from '../../../lib/potato';
import { ThreadView } from './ThreadView';
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
  window.localStorage.clear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const NOW = new Date(2026, 10, 7, 15, 0).getTime();
const SETTINGS: PotatoSettings = { v: 1, brain: 'deepseek', defaultPersona: 'fish', dailyLimitCents: 50 };
const peer = (dir: 'out' | 'in' | 'reply', text: string, botName: string) =>
  msg({ text, kind: 'note', from: { kind: 'note', name: 'peer' }, peer: { dir, botId: botName.toLowerCase(), botName } });

async function show(messages = [peer('out', 'what is the plan for Friday?', 'Mochi')]) {
  const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: messages } });
  await act(async () =>
    root.render(<ThreadView bot={bot({ id: 'fish' })} messages={messages} settings={SETTINGS} avatar="mochi" agentLabel="Claude" transport={fake} onOpenSettings={() => {}} onNotify={() => {}} onChanged={() => {}} now={NOW} />),
  );
}

describe('ThreadView: peer cards', () => {
  it('shows a message this bot wrote to another bot', async () => {
    await show();
    const card = host.querySelector('[role="group"][aria-label="Message between bots"]')!;
    expect(card).not.toBeNull();
    expect(card.textContent).toContain('You asked this bot to write to Mochi');
    expect(card.textContent).toContain('what is the plan for Friday?');
  });

  it('shows a message another bot wrote to this one, and the answer that came back', async () => {
    await show([peer('in', 'can you check the invoice?', 'Nimbus'), peer('reply', 'Checked: it is paid.', 'Mochi')]);
    const cards = [...host.querySelectorAll('[aria-label="Message between bots"]')];
    expect(cards).toHaveLength(2);
    expect(cards[0]!.textContent).toContain('Nimbus wrote to this bot');
    expect(cards[0]!.textContent).toContain('can you check the invoice?');
    expect(cards[1]!.textContent).toContain('Mochi answered');
    expect(cards[1]!.textContent).toContain('Checked: it is paid.');
  });

  it('a note that is not a peer card stays a plain grey note', async () => {
    await show([msg({ text: 'Mochi is not running.', kind: 'note', from: { kind: 'note' } })]);
    expect(host.querySelector('[aria-label="Message between bots"]')).toBeNull();
    expect(host.textContent).toContain('Mochi is not running.');
  });
});
