// @vitest-environment jsdom
// The clipboard buttons of the full-window computer: only while the person has control, text only, the computer's clipboard
// held until Copy is pressed, and messages believed only from the iframe's own window and origin.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeScreenTransport, FAKE_ORIGIN, MAX_CLIPBOARD_CHARS, type ScreenTransport } from '../../../lib/potatoScreen';
import { ComputerPanel } from './ComputerPanel';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
let readText: ReturnType<typeof vi.fn>;
let writeText: ReturnType<typeof vi.fn>;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  readText = vi.fn(async () => 'from windows');
  writeText = vi.fn(async () => undefined);
  Object.defineProperty(navigator, 'clipboard', { value: { readText, writeText }, configurable: true });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const BOT = { id: 'fish', name: 'Fish' };
const overlay = () => document.querySelector<HTMLElement>('.ck-potato-overlay');
const frame = () => document.querySelector('iframe')!;
const names = (within: ParentNode) => [...within.querySelectorAll<HTMLButtonElement>('button')].map((b) => b.getAttribute('aria-label') ?? b.textContent?.trim());
const button = (label: string) => [...(overlay() ?? host).querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === label);
const press = async (label: string) => {
  const b = button(label);
  expect(b, `a button named ${label}`).toBeTruthy();
  await act(async () => b!.click());
  await act(async () => {});
};
async function show(t: ScreenTransport) {
  await act(async () => root.render(<ComputerPanel bot={BOT} transport={t} />));
  await act(async () => {});
}
async function fromViewer(data: unknown, over: { origin?: string; source?: Window | null } = {}) {
  await act(async () => {
    window.dispatchEvent(new MessageEvent('message', { data, origin: over.origin ?? FAKE_ORIGIN, source: over.source === undefined ? frame().contentWindow : over.source }));
  });
}
const copied = (text: string) => ({ type: 'pm-sbx-clipboard', text });
const status = () => overlay()!.querySelector('.ck-potato-overlay-clip-note')?.textContent;

describe('the clipboard buttons', () => {
  it('are not there while the person only looks, and are there with control', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    expect(document.body.textContent).not.toContain('Paste');
    await press('Take control');
    expect(names(overlay()!)).toEqual(['Paste', 'Copy', "I'm done", 'Close']);
    expect(button('Copy')!.disabled).toBe(true);
  });

  it('Paste reads the Windows clipboard on the press and sends it to the iframe, and only to its own origin', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    await press('Take control');
    const send = vi.spyOn(frame().contentWindow!, 'postMessage');
    await press('Paste');
    expect(readText).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith({ type: 'pm-sbx-paste', text: 'from windows' }, FAKE_ORIGIN);
    expect(status()).toBe('Pasted.');
  });

  it('Paste says so when the clipboard has no text, or Windows says no', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    await press('Take control');
    const send = vi.spyOn(frame().contentWindow!, 'postMessage');
    readText.mockResolvedValueOnce('');
    await press('Paste');
    expect(status()).toBe('Your clipboard has no text.');
    readText.mockRejectedValueOnce(new Error('denied'));
    await press('Paste');
    expect(status()).toContain('Ctrl+V');
    expect(send).not.toHaveBeenCalled();
  });

  it('holds what the computer copied until Copy is pressed, then puts it on the Windows clipboard', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    await press('Take control');
    await fromViewer(copied('a secret link'));
    expect(writeText).not.toHaveBeenCalled();
    expect(button('Copy')!.disabled).toBe(false);
    await press('Copy');
    expect(writeText).toHaveBeenCalledWith('a secret link');
    expect(status()).toBe('Copied.');
  });

  it('believes a clipboard message only from the iframe itself, from its origin, as text of a fair size', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    await press('Take control');
    await fromViewer(copied('wrong origin'), { origin: 'http://evil.example' });
    await fromViewer(copied('wrong window'), { source: window });
    await fromViewer({ type: 'pm-sbx-clipboard', text: 42 });
    await fromViewer(copied(''));
    await fromViewer(copied('x'.repeat(MAX_CLIPBOARD_CHARS + 1)));
    await fromViewer({ type: 'something-else', text: 'nope' });
    expect(button('Copy')!.disabled).toBe(true);
    await fromViewer(copied('x'.repeat(MAX_CLIPBOARD_CHARS)));
    expect(button('Copy')!.disabled).toBe(false);
  });

  it('ignores a clipboard message while the person only looks (nothing is held, nothing can be copied)', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    await fromViewer(copied('the bot copied this'));
    await press('Take control');
    expect(button('Copy')!.disabled).toBe(true);
    expect(writeText).not.toHaveBeenCalled();
  });

  it('drops what was held when the person hands the computer back', async () => {
    const t = fakeScreenTransport({ phase: 'bot', bot: 'fish' });
    await show(t);
    await press('Take control');
    await fromViewer(copied('held'));
    await press("I'm done");
    await press('Take control');
    expect(button('Copy')!.disabled).toBe(true);
  });
});
