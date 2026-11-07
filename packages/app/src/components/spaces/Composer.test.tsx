// @vitest-environment jsdom
// The composer: Enter sends, Shift+Enter makes a new line, empty text is blocked, the 4000-character cap, server errors, cancel reply.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceMessage } from '../../lib/spaces';
import { Composer } from './Composer';

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

const replyTarget: SpaceMessage = {
  id: 'm1', channel: 'c1', member: 'mem-sam', authorName: 'Sam', isFish: false, isMine: false,
  createdAtMs: 0, text: 'hi', replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false,
};

function mount(over: Partial<Parameters<typeof Composer>[0]> = {}) {
  const props = {
    channelName: 'general',
    replyTo: null as SpaceMessage | null,
    onCancelReply: vi.fn(),
    onSend: vi.fn(async (_text: string) => null as string | null),
    ...over,
  };
  act(() => {
    root.render(<Composer {...props} />);
  });
  return props;
}

const box = () => host.querySelector('textarea') as HTMLTextAreaElement;
const sendButton = () =>
  Array.from(host.querySelectorAll('button')).find((b) => b.textContent === 'Send') as HTMLButtonElement;

function type(text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
  act(() => {
    setter.call(box(), text);
    box().dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function key(k: string, shiftKey = false) {
  act(() => {
    box().dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true }));
  });
}

describe('Composer', () => {
  it('enter sends and clears', async () => {
    const props = mount();
    type('hi');
    key('Enter');
    await act(async () => {});
    expect(props.onSend).toHaveBeenCalledWith('hi');
    expect(box().value).toBe('');
  });

  it('shift enter adds a new line', async () => {
    const props = mount();
    type('a');
    key('Enter', true);
    await act(async () => {});
    expect(props.onSend).not.toHaveBeenCalled();
    type('a\nb');
    expect(box().value).toContain('\n');
  });

  it('empty text cannot be sent', async () => {
    const props = mount();
    type('   ');
    expect(sendButton().disabled).toBe(true);
    key('Enter');
    await act(async () => {});
    expect(props.onSend).not.toHaveBeenCalled();
  });

  it('counter shows above 3600 characters', () => {
    mount();
    type('x'.repeat(3700));
    expect(host.querySelector('.ck-spaces-count')?.textContent).toBe('3700 / 4000');
  });

  it('too long blocks send', () => {
    mount();
    type('x'.repeat(4001));
    expect(host.textContent).toContain('This message is too long. The limit is 4000 characters.');
    expect(sendButton().disabled).toBe(true);
  });

  it('counts characters, not bytes: 4000 emoji is allowed, 4001 is not', () => {
    mount();
    type('\u{1F600}'.repeat(4000));
    expect(sendButton().disabled).toBe(false);
    type('\u{1F600}'.repeat(4001));
    expect(sendButton().disabled).toBe(true);
  });

  it('server error is shown and text kept', async () => {
    const props = mount({
      onSend: vi.fn(async () => "You're sending messages too fast. Wait a few seconds."),
    });
    type('hi');
    key('Enter');
    await act(async () => {});
    expect(props.onSend).toHaveBeenCalledWith('hi');
    expect(host.querySelector('[role="alert"]')?.textContent).toBe("You're sending messages too fast. Wait a few seconds.");
    expect(box().value).toBe('hi');
  });

  it('cancel reply', () => {
    const props = mount({ replyTo: replyTarget });
    const cancel = Array.from(host.querySelectorAll('button')).find((b) => b.textContent === 'Cancel reply') as HTMLButtonElement;
    act(() => cancel.click());
    expect(props.onCancelReply).toHaveBeenCalledTimes(1);
  });
});
