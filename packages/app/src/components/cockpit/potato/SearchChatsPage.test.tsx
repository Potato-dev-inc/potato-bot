// @vitest-environment jsdom
// The Search chats page: typing searches after a pause, the answer shows the bot, the speaker, the date and the snippet with
// the words marked, a stale answer is dropped, and a result opens its thread.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeSearchTransport, type SearchHit, type SearchTransport } from '../../../lib/potato-search';
import { SearchChatsPage } from './SearchChatsPage';

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

const NOW = new Date(2026, 10, 6, 12, 0).getTime();
const hit = (over: Partial<SearchHit> = {}): SearchHit => ({
  kind: 'bot',
  ownerId: 'fish',
  ownerName: 'Fish',
  messageId: 'm-1',
  at: new Date(2026, 10, 2, 9, 30).getTime(),
  speaker: 'You',
  snippet: 'plan the Lisbon trip',
  marks: [[9, 15]],
  fromEnd: 3,
  ...over,
});

const settle = () => act(async () => new Promise((r) => setTimeout(r, 5)));
const box = () => host.querySelector<HTMLInputElement>('input.ck-search-input')!;
function type(text: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(box(), text);
  act(() => {
    box().dispatchEvent(new Event('input', { bubbles: true }));
  });
}
function mount(transport: SearchTransport, over: Partial<Parameters<typeof SearchChatsPage>[0]> = {}) {
  const onOpenHit = vi.fn();
  const onClose = vi.fn();
  act(() => {
    root.render(<SearchChatsPage transport={transport} onOpenHit={onOpenHit} onClose={onClose} debounceMs={0} now={NOW} {...over} />);
  });
  return { onOpenHit, onClose };
}

describe('SearchChatsPage', () => {
  it('asks for words first, and focuses the box', () => {
    mount(fakeSearchTransport([hit()]));
    expect(host.textContent).toContain('Type a word or two');
    expect(document.activeElement).toBe(box());
  });

  it('shows each match with the bot, who wrote it, the date and the snippet with the words marked', async () => {
    const t = fakeSearchTransport([hit(), hit({ kind: 'group', ownerId: 'r1', ownerName: 'Trip crew', speaker: 'Nimbus', messageId: 'l-9', snippet: 'who books the Lisbon hotel', marks: [[14, 20]] })]);
    mount(t);
    type('lisbon');
    await settle();
    const rows = [...host.querySelectorAll('.ck-search-hit')];
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector('.ck-search-owner')?.textContent).toBe('Fish');
    expect(rows[0].querySelector('.ck-search-who')?.textContent).toBe('You');
    expect(rows[0].querySelector('.ck-search-when')?.textContent).toBe('Nov 2');
    expect(rows[0].querySelector('mark')?.textContent).toBe('Lisbon');
    expect(rows[1].querySelector('.ck-search-kind')?.textContent).toBe('Group');
    expect(rows[1].querySelector('mark')?.textContent).toBe('Lisbon');
    expect(t.calls).toEqual([{ query: 'lisbon' }]);
  });

  it('choosing a result hands it to the page, and Back to chat and Escape close', async () => {
    const t = fakeSearchTransport([hit()]);
    const { onOpenHit, onClose } = mount(t);
    type('lisbon');
    await settle();
    act(() => (host.querySelector('.ck-search-hit') as HTMLElement).click());
    expect(onOpenHit).toHaveBeenCalledWith(expect.objectContaining({ messageId: 'm-1', ownerId: 'fish' }));
    act(() => ([...host.querySelectorAll('button')].find((b) => b.textContent === 'Back to chat') as HTMLElement).click());
    expect(onClose).toHaveBeenCalledTimes(1);
    act(() => {
      box().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('says so when nothing matches, and when the search fails', async () => {
    mount(fakeSearchTransport([hit()]));
    type('zebra');
    await settle();
    expect(host.textContent).toContain('No message has those words');
    act(() => root.unmount());
    root = createRoot(host);
    mount(fakeSearchTransport([], { fail: 'POTATO_SEARCH_BAD_QUERY: Type the words to look for.' }));
    type('a');
    await settle();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Type the words to look for.');
  });

  it('drops an answer that is no longer for what is typed', async () => {
    let release: () => void = () => {};
    const slow = new Promise<void>((r) => (release = r));
    const answer = (snippet: string) => ({ hits: [hit({ snippet, marks: [] })], matches: 1, searched: 1 });
    const t: SearchTransport = {
      search: (q) => (q === 'old' ? slow.then(() => answer('OLD answer')) : Promise.resolve(answer('NEW answer'))),
    };
    mount(t);
    type('old');
    await settle();
    type('new');
    await settle();
    release();
    await settle();
    expect(host.textContent).toContain('NEW answer');
    expect(host.textContent).not.toContain('OLD answer');
  });

  it('waits for a pause in typing before it searches (the UI never searches per key)', async () => {
    vi.useFakeTimers();
    try {
      const t = fakeSearchTransport([hit()]);
      mount(t, { debounceMs: 250 });
      type('l');
      type('li');
      type('lis');
      await act(async () => {
        vi.advanceTimersByTime(249);
      });
      expect(t.calls).toHaveLength(0);
      await act(async () => {
        vi.advanceTimersByTime(2);
      });
      expect(t.calls).toEqual([{ query: 'lis' }]);
    } finally {
      vi.useRealTimers();
    }
  });

  it('says when the list was cut', async () => {
    const t: SearchTransport = { search: async () => ({ hits: [hit()], matches: 120, searched: 4000 }) };
    mount(t);
    type('lisbon');
    await settle();
    expect(host.textContent).toContain('Showing the newest 1 of 120 matches');
  });
});
