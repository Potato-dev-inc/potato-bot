// @vitest-environment jsdom
// Live steering, the person's side: the box stays usable while the bot works, a message sent then shows at once with a
// quiet "Queued" label, and the label clears when the bot has picked it up.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport, type PotatoSettings } from '../../../lib/potato';
import type { WorkRunView } from '../../../lib/potato-work';
import { ThreadView, type ThreadViewProps } from './ThreadView';
import { bot, msg, typeInto, you } from './test-helpers';

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

const SETTINGS: PotatoSettings = { v: 1, brain: 'deepseek', defaultPersona: 'fish', dailyLimitCents: 50 };
const NOW = new Date(2026, 10, 1, 15, 0).getTime();

function props(over: Partial<ThreadViewProps> = {}): ThreadViewProps {
  return {
    bot: bot(),
    messages: [],
    settings: SETTINGS,
    avatar: 'mochi',
    agentLabel: 'Claude',
    transport: fakePotatoTransport({ bots: [bot()] }),
    onOpenSettings: () => {},
    onNotify: () => {},
    onChanged: () => {},
    now: NOW,
    ...over,
  };
}

const working: WorkRunView = {
  startedMs: NOW - 60_000,
  steps: [{ id: 'a', tool: 'run_terminal', status: 'running', startedMs: NOW - 30_000, endedMs: null, log: [], detail: 'Ran: npm install' }],
  failed: false,
  endedMs: null,
};

describe('ThreadView while the bot works', () => {
  it('keeps the message box usable during a turn and sends what is typed', async () => {
    const p = props({ bot: bot({ typing: true, work: working }), messages: [you('do the big thing')] });
    const sent: string[] = [];
    p.transport.send = async (id, text) => {
      sent.push(text);
      return { id: 'm-new', state: 'ready', delivered: true } as never;
    };
    act(() => root.render(<ThreadView {...p} />));
    const field = host.querySelector<HTMLTextAreaElement>('textarea')!;
    expect(field.disabled).toBe(false);
    typeInto(field, 'status?');
    await act(async () => {
      field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    });
    expect(sent).toEqual(['status?']);
  });

  it('shows a Queued label on a message the bot has not picked up, and clears it once it has', () => {
    const waiting = { ...you('status?'), queued: true };
    const p = props({ bot: bot({ typing: true, work: working }), messages: [you('do the big thing'), waiting] });
    act(() => root.render(<ThreadView {...p} />));
    const labels = [...host.querySelectorAll('.ck-potato-queued')];
    expect(labels).toHaveLength(1);
    expect(labels[0]?.textContent).toBe('Queued');
    // the label belongs to the waiting bubble, not the first one
    const bubbles = [...host.querySelectorAll('.ck-potato-bubble--you')];
    expect(bubbles[0]?.querySelector('.ck-potato-queued')).toBeNull();
    expect(bubbles[1]?.querySelector('.ck-potato-queued')).not.toBeNull();
    // the bot picked it up: the app stops saying queued
    act(() => root.render(<ThreadView {...p} messages={[you('do the big thing'), { ...waiting, queued: false }]} />));
    expect(host.querySelector('.ck-potato-queued')).toBeNull();
    expect(host.textContent).toContain('status?');
  });

  it('never shows the label on the bot\'s own lines', () => {
    const p = props({ messages: [msg({ text: 'working on it', queued: true })] });
    act(() => root.render(<ThreadView {...p} />));
    expect(host.querySelector('.ck-potato-queued')).toBeNull();
  });
});
