// @vitest-environment jsdom
// The channel column: active channels with unread counts, the owner's New channel button, and the new-channel sheet.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { SpaceChannel } from '../../lib/spaces';
import { ChannelList } from './ChannelList';
import { NewChannelSheet, previewChannelName } from './NewChannelSheet';

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

const ch = (over: Partial<SpaceChannel> = {}): SpaceChannel => ({ id: 'c1', name: 'general', archived: false, unread: 0, ...over });
const text = () => host.textContent ?? '';
const button = (label: string) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) ?? null;

describe('ChannelList', () => {
  it('lists active channels with unread', async () => {
    const channels = [
      ch({ id: 'c1', name: 'general', unread: 3 }),
      ch({ id: 'c2', name: 'books' }),
      ch({ id: 'c3', name: 'old', archived: true }),
    ];
    await act(async () => root.render(<ChannelList channels={channels} selected="c1" isOwner={false} onSelect={() => {}} onNew={() => {}} />));
    expect(text()).toContain('# general');
    expect(host.querySelector('.ck-spaces-badge')?.textContent).toBe('3');
    expect(text()).toContain('# books');
    expect(text()).not.toContain('# old');
  });

  it('owner sees New channel', async () => {
    await act(async () => root.render(<ChannelList channels={[ch()]} selected="c1" isOwner={true} onSelect={() => {}} onNew={() => {}} />));
    expect(button('New channel')).not.toBeNull();
    await act(async () => root.render(<ChannelList channels={[ch()]} selected="c1" isOwner={false} onSelect={() => {}} onNew={() => {}} />));
    expect(button('New channel')).toBeNull();
  });

  it('twenty channels disables New channel', async () => {
    const channels = Array.from({ length: 20 }, (_, i) => ch({ id: `c${i}`, name: `ch-${i}` }));
    await act(async () => root.render(<ChannelList channels={channels} selected="c0" isOwner={true} onSelect={() => {}} onNew={() => {}} />));
    const b = button('New channel') as HTMLButtonElement;
    expect(b.disabled).toBe(true);
    expect(b.title).toBe('This space has 20 channels. Archive one first.');
  });

  it('previewChannelName matches the Rust rule', () => {
    expect(previewChannelName('  Book Club! ')).toBe('book-club');
    expect(previewChannelName('---')).toBe('');
  });

  it('new channel sheet shows server error', async () => {
    const msg = 'There is already a channel called #books.';
    await act(async () => root.render(<NewChannelSheet onCreate={async () => msg} onClose={() => {}} />));
    const input = host.querySelector('input') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, 'books');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => button('Create')!.click());
    await act(async () => {});
    expect(text()).toContain(msg);
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
  });
});
