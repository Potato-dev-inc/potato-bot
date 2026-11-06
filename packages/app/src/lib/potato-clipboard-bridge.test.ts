// The viewer page's clipboard bridge (packages/potato-sandbox/viewer/clipboard-bridge.js), run against stand-ins for noVNC
// and the browser: the app's Paste message and the computer's copy message, who is believed, and what a view-only page does.
import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import {
  attachParentClipboard,
  attachRemoteClipboardCopy,
  CLIPBOARD_MESSAGE_TYPE,
  MAX_CLIPBOARD_CHARS,
  PASTE_MESSAGE_TYPE,
  type RfbLike,
} from '../../../potato-sandbox/viewer/clipboard-bridge.js';

const ORIGIN = 'http://127.0.0.1:41999';

function rig(over: Partial<RfbLike> = {}) {
  const listeners = new Map<string, (event: unknown) => void>();
  const rfb: RfbLike = {
    viewOnly: false,
    clipboardPasteFrom: vi.fn(),
    sendKey: vi.fn(),
    addEventListener: (type, listener) => void listeners.set(type, listener),
    removeEventListener: (type) => void listeners.delete(type),
    ...over,
  };
  const winListeners = new Map<string, (event: unknown) => void>();
  const parent = { postMessage: vi.fn() };
  const win = {
    parent,
    addEventListener: (type: string, l: (event: unknown) => void) => void winListeners.set(type, l),
    removeEventListener: (type: string) => void winListeners.delete(type),
  };
  return {
    rfb,
    parent,
    win,
    message: (data: unknown, from: { source?: unknown; origin?: string } = {}) =>
      winListeners.get('message')?.({ data, source: 'source' in from ? from.source : parent, origin: from.origin ?? ORIGIN }),
    remoteCopied: (text: unknown) => listeners.get('clipboard')?.({ detail: { text } }),
    attached: () => ({ message: winListeners.has('message'), clipboard: listeners.has('clipboard') }),
  };
}
const attach = (r: ReturnType<typeof rig>, parentOrigins: string[] = [ORIGIN]) =>
  attachParentClipboard(r.rfb, { parentOrigins, window: r.win, parent: r.win.parent });

describe('the app pastes into the computer', () => {
  it('puts the text on the remote clipboard and sends Ctrl+V, from the parent window and an allowed origin', () => {
    const r = rig();
    attach(r);
    r.message({ type: PASTE_MESSAGE_TYPE, text: 'hello' });
    expect(r.rfb.clipboardPasteFrom).toHaveBeenCalledWith('hello');
    expect(r.rfb.sendKey).toHaveBeenCalled();
  });

  it.each([
    ['another window', { type: PASTE_MESSAGE_TYPE, text: 'x' }, { source: {} }],
    ['another origin', { type: PASTE_MESSAGE_TYPE, text: 'x' }, { origin: 'http://evil.example' }],
    ['another type', { type: 'pm-sbx-other', text: 'x' }, {}],
    ['no text', { type: PASTE_MESSAGE_TYPE }, {}],
    ['text that is not text', { type: PASTE_MESSAGE_TYPE, text: { a: 1 } }, {}],
    ['empty text', { type: PASTE_MESSAGE_TYPE, text: '' }, {}],
    ['text that is too long', { type: PASTE_MESSAGE_TYPE, text: 'x'.repeat(MAX_CLIPBOARD_CHARS + 1) }, {}],
  ])('ignores a message from %s', (_name, data, from) => {
    const r = rig();
    attach(r);
    r.message(data, from);
    expect(r.rfb.clipboardPasteFrom).not.toHaveBeenCalled();
  });

  it('attaches nothing on a view-only page, with no allowed origin, or with only a wildcard', () => {
    const viewOnly = rig({ viewOnly: true });
    attach(viewOnly);
    expect(viewOnly.attached()).toEqual({ message: false, clipboard: false });
    const none = rig();
    attach(none, []);
    expect(none.attached()).toEqual({ message: false, clipboard: false });
    const wild = rig();
    attach(wild, ['*']);
    expect(wild.attached()).toEqual({ message: false, clipboard: false });
  });

  it('stops listening when detached', () => {
    const r = rig();
    const detach = attach(r);
    expect(r.attached()).toEqual({ message: true, clipboard: true });
    detach();
    expect(r.attached()).toEqual({ message: false, clipboard: false });
  });
});

describe('the computer copies', () => {
  it('tells the allowed origins only, and writes nothing to a clipboard by itself', () => {
    const r = rig();
    attach(r, [ORIGIN, 'https://app.example.test']);
    r.remoteCopied('copied in the computer');
    expect(r.parent.postMessage.mock.calls).toEqual([
      [{ type: CLIPBOARD_MESSAGE_TYPE, text: 'copied in the computer' }, ORIGIN],
      [{ type: CLIPBOARD_MESSAGE_TYPE, text: 'copied in the computer' }, 'https://app.example.test'],
    ]);
  });

  it('drops empty, non-text and too long copies', () => {
    const r = rig();
    attach(r);
    for (const text of ['', 5, null, 'x'.repeat(MAX_CLIPBOARD_CHARS + 1)]) r.remoteCopied(text);
    expect(r.parent.postMessage).not.toHaveBeenCalled();
  });

  it('a page that is not in control sends nothing even if the flag flips later', () => {
    const r = rig();
    attach(r);
    r.rfb.viewOnly = true;
    r.remoteCopied('late');
    r.message({ type: PASTE_MESSAGE_TYPE, text: 'late' });
    expect(r.parent.postMessage).not.toHaveBeenCalled();
    expect(r.rfb.clipboardPasteFrom).not.toHaveBeenCalled();
  });
});

describe('the page wiring', () => {
  it('embed.html attaches every clipboard path only when the person has control', () => {
    const html = readFileSync(new URL('../../../potato-sandbox/viewer/embed.html', import.meta.url), 'utf8');
    const start = html.indexOf('if (!rfb.viewOnly) {');
    expect(start).toBeGreaterThan(0);
    const inside = html.slice(start, html.indexOf('}', start) + 1);
    expect(inside).toContain('attachRemoteClipboardCopy(rfb)');
    expect(inside).toContain('attachHostClipboardPaste(rfb)');
    expect(inside).toContain('attachParentClipboard(rfb');
    // and the computer-to-Windows copy is attached nowhere else (a view-only page must not write to the person's clipboard)
    expect(html.match(/attachRemoteClipboardCopy\(rfb\)/g)).toHaveLength(1);
  });

  it('the browser-side copy (writeText) works for the person in control', async () => {
    const writeText = vi.fn(async () => undefined);
    const r = rig();
    attachRemoteClipboardCopy(r.rfb, { clipboard: { writeText } });
    r.remoteCopied('abc');
    await Promise.resolve();
    expect(writeText).toHaveBeenCalledWith('abc');
  });
});
