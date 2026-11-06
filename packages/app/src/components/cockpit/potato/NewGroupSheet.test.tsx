// @vitest-environment jsdom
// The New group sheet: it blocks an invalid group with the engine's sentences, makes a valid one, and closes on Escape.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeRoomsTransport } from '../../../lib/potato-rooms';
import { NewGroupSheet } from './NewGroupSheet';

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

const BOTS = [
  { id: 'fish', name: 'Fish' },
  { id: 'potato', name: 'Potato' },
  { id: 'kiwi', name: 'Kiwi' },
];

function setInput(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

function makeButton(): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find((b) => b.textContent === 'Make group') as HTMLButtonElement | undefined;
}

async function mount(bots = BOTS, onDone = vi.fn(), onCancel = vi.fn(), transport = fakeRoomsTransport()) {
  await act(async () => root.render(<NewGroupSheet bots={bots} transport={transport} onDone={onDone} onCancel={onCancel} />));
  return { onDone, onCancel, transport };
}

describe('NewGroupSheet', () => {
  it('keeps Make group off until the group is valid, and says why', async () => {
    await mount();
    expect(makeButton()?.disabled).toBe(true);
    expect(host.textContent).toContain('Give the group a name.');
  });

  it('says the 2-bot rule when only one bot is picked', async () => {
    await mount();
    await act(async () => setInput(host.querySelector('input[type="text"]') as HTMLInputElement, 'Weekend'));
    await act(async () => (host.querySelector('input[type="checkbox"]') as HTMLInputElement).click());
    expect(host.textContent).toContain('Pick at least 2 bots.');
    expect(makeButton()?.disabled).toBe(true);
  });

  it('makes the group and hands it to onDone', async () => {
    const { onDone, transport } = await mount();
    await act(async () => setInput(host.querySelector('input[type="text"]') as HTMLInputElement, 'Weekend'));
    const boxes = host.querySelectorAll('input[type="checkbox"]');
    await act(async () => (boxes[0] as HTMLInputElement).click());
    await act(async () => (boxes[1] as HTMLInputElement).click());
    expect(makeButton()?.disabled).toBe(false);
    await act(async () => makeButton()?.click());
    await act(async () => {});
    expect(transport.rooms.map((r) => r.name)).toEqual(['Weekend']);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('shows only the sentence and Cancel when there are fewer than 2 bots', async () => {
    await mount([{ id: 'fish', name: 'Fish' }]);
    expect(host.textContent).toContain('You need at least 2 bots to make a group.');
    expect(makeButton()).toBeUndefined();
  });

  it('closes on Escape', async () => {
    const { onCancel } = await mount();
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })));
    expect(onCancel).toHaveBeenCalled();
  });
});
