// @vitest-environment jsdom
// Settings from chat, as the owner sees it in the thread: a note says what the bot changed about itself, with an Undo
// button; Undo goes to the app (not to the bot), the card then says Undone and has no button, and an error shows as a sentence.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport, type FakePotatoTransport, type PotatoMessage } from '../../../lib/potato';
import { Transcript } from './Transcript';
import { bot } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

const NOW = new Date(2026, 10, 5, 15, 0).getTime();

const card = (over: Partial<NonNullable<PotatoMessage['change']>> = {}): PotatoMessage => ({
  id: 'm-change',
  at: NOW - 60_000,
  from: { kind: 'note' },
  kind: 'note',
  text: 'Changed its name from “Fish” to “Cloud”.',
  change: { id: 'sc-1', field: 'name', undone: false, ...over },
});

async function show(messages: PotatoMessage[], transport: FakePotatoTransport, onChanged = () => {}) {
  await act(async () => {
    root.render(<Transcript bot={bot()} messages={messages} agentLabel="Claude" transport={transport} onChanged={onChanged} now={NOW} openLink={() => {}} />);
  });
}
const undoButton = () => [...host.querySelectorAll('button')].find((b) => b.textContent === 'Undo');
const settle = () => act(async () => new Promise((r) => setTimeout(r, 0)));

describe('Transcript: a settings change card', () => {
  it('says what changed and has an Undo button', async () => {
    const transport = fakePotatoTransport({ bots: [bot()], messages: { fish: [card()] } });
    await show([card()], transport);
    const note = host.querySelector('.ck-potato-change');
    expect(note?.textContent).toContain('Changed its name from “Fish” to “Cloud”.');
    expect(undoButton()).toBeTruthy();
  });

  it('Undo asks the app to restore the old value, once, and the card then says Undone', async () => {
    let changed = 0;
    const transport = fakePotatoTransport({ bots: [bot()], messages: { fish: [card()] } });
    await show([card()], transport, () => changed++);
    await act(async () => undoButton()!.click());
    await settle();
    expect(transport.calls.filter((c) => c.method === 'undoSetting').map((c) => c.args)).toEqual([['fish', 'sc-1']]);
    expect(changed).toBe(1);
    // the thread is read again: the same card, now undone
    await show([card({ undone: true })], transport);
    expect(undoButton()).toBeUndefined();
    expect(host.querySelector('.ck-potato-change')?.textContent).toContain('Undone');
  });

  it('shows the app\'s plain sentence when Undo is refused', async () => {
    const transport = fakePotatoTransport({ bots: [bot()], messages: { fish: [] } });
    // the fake has no such change in its thread, so it refuses like the app does for a change that is gone
    await show([card()], transport);
    await act(async () => undoButton()!.click());
    await settle();
    expect(host.querySelector('[role="alert"]')?.textContent?.trim()).toBe('That change is not in the record any more.');
    expect(undoButton()).toBeTruthy();
  });

  it('a change that was undone before the thread was opened shows no button', async () => {
    await show([card({ undone: true })], fakePotatoTransport({ bots: [bot()] }));
    expect(undoButton()).toBeUndefined();
    expect(host.textContent).toContain('Undone');
  });
});
