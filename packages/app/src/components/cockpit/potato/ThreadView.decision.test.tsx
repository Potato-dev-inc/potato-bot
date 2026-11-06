// @vitest-environment jsdom
// The thread with decision cards and the job bar (Professional mode): a card is answered from the thread, stays
// answered after a reload (its answer row is in the messages), and the job bar shows the job with a Stop that is the
// person Stop.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport, type PotatoJob, type PotatoSettings, type PotatoTransport } from '../../../lib/potato';
import { ThreadView, type ThreadViewProps } from './ThreadView';
import { bot, buttonByText, msg } from './test-helpers';

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

const NOW = new Date(2026, 10, 6, 15, 0).getTime();
const SETTINGS: PotatoSettings = { v: 1, brain: 'deepseek', defaultPersona: 'pro', dailyLimitCents: 50 };
const CARD = msg({
  id: 'card-1',
  text: 'npm publish',
  kind: 'decision_card',
  requestId: 'c1',
  card: { kind: 'approval', why: 'It ships the package to npm.', choices: [] },
});
const ANSWER = msg({
  id: 'res-1',
  text: 'Answered.',
  from: { kind: 'note' },
  kind: 'decision_result',
  requestId: 'c1',
  card: { kind: 'approval', why: '', choices: [], answer: 'allow_once' },
});

function setup(transport: PotatoTransport, messages = [CARD], over: Partial<ThreadViewProps> = {}) {
  const props: ThreadViewProps = {
    bot: bot({ id: 'fish', talkPersona: 'pro' }),
    messages,
    settings: SETTINGS,
    avatar: 'mochi',
    agentLabel: 'Claude',
    transport,
    onOpenSettings: () => {},
    onNotify: () => {},
    onChanged: () => {},
    now: NOW,
    ...over,
  };
  return props;
}
const flush = () => act(async () => {});
const job = (over: Partial<PotatoJob> = {}): PotatoJob => ({ goal: 'Fix the failing build', step: 'Running the tests', startedMs: NOW - 90_000, spendNano: 0, state: 'running', ...over });

describe('ThreadView: decision cards', () => {
  it('shows the card with its buttons; an answer sends it and asks the thread to read again', async () => {
    // The fake keeps its own thread: the card must be in it for the answer to find it.
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] } });
    const answerCard = vi.fn(fake.answerCard);
    const onChanged = vi.fn();
    const transport = { ...fake, answerCard };
    await act(async () => root.render(<ThreadView {...setup(transport, [CARD], { onChanged })} />));
    expect(host.querySelector('[aria-label="Approval from Fish"]')?.textContent).toContain('npm publish');
    await act(async () => buttonByText(host, 'Allow once')!.click());
    await flush();
    expect(answerCard).toHaveBeenCalledWith('fish', 'c1', 'allow_once');
    expect(onChanged).toHaveBeenCalled();
  });

  it('a card answered earlier stays answered after a reload: no buttons, a quiet line', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })] });
    await act(async () => root.render(<ThreadView {...setup(fake, [CARD, ANSWER])} />));
    expect(buttonByText(host, 'Allow once')).toBeUndefined();
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('Allowed once.');
  });

  it('a failed answer shows the sentence and the buttons stay for another try', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })] });
    const transport = { ...fake, answerCard: () => Promise.reject(new Error('POTATO_CARD_NONE: There is no such card.')) };
    await act(async () => root.render(<ThreadView {...setup(transport)} />));
    await act(async () => buttonByText(host, 'Deny')!.click());
    await flush();
    expect(host.querySelector('[role="status"]')?.textContent).toBe('There is no such card.');
    expect(buttonByText(host, 'Deny')).toBeDefined();
  });
});

describe('ThreadView: job bar', () => {
  it('pins the job at the top while it runs, and Stop is the person Stop', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs: { fish: job() } });
    const stopBot = vi.fn(fake.stopBot);
    const transport = { ...fake, stopBot };
    await act(async () => root.render(<ThreadView {...setup(transport, [])} />));
    await flush();
    expect(host.querySelector('.ck-potato-job-goal')?.textContent).toBe('Fix the failing build');
    expect(host.querySelector('.ck-potato-job-facts')?.textContent).toContain('1m 30s');
    await act(async () => buttonByText(host, 'Stop')!.click());
    await flush();
    expect(stopBot).toHaveBeenCalledWith('fish');
  });

  it('a finished job shows no bar', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs: { fish: job({ state: 'done' }) } });
    await act(async () => root.render(<ThreadView {...setup(fake, [])} />));
    await flush();
    expect(host.querySelector('.ck-potato-job')).toBeNull();
  });

  it('a bot with no job shows no bar', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })] });
    await act(async () => root.render(<ThreadView {...setup(fake, [])} />));
    await flush();
    expect(host.querySelector('.ck-potato-job')).toBeNull();
  });

  it('a transport without a job call shows no bar and does not fail', async () => {
    const fake: PotatoTransport = { ...fakePotatoTransport({ bots: [bot({ id: 'fish' })] }), job: undefined };
    await act(async () => root.render(<ThreadView {...setup(fake, [])} />));
    await flush();
    expect(host.querySelector('.ck-potato-job')).toBeNull();
  });
});

describe('ThreadView: job bar refresh', () => {
  it('a job that appears on the bot-changed event shows, and goes away when the job ends', async () => {
    const jobs: Record<string, PotatoJob | null> = {};
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs });
    await act(async () => root.render(<ThreadView {...setup(fake, [])} />));
    await flush();
    expect(host.querySelector('.ck-potato-job')).toBeNull();

    jobs.fish = job();
    await act(async () => fake.emit('fish'));
    await flush();
    expect(host.querySelector('.ck-potato-job-goal')?.textContent).toBe('Fix the failing build');

    jobs.fish = job({ state: 'done' });
    await act(async () => fake.emit('fish'));
    await flush();
    expect(host.querySelector('.ck-potato-job')).toBeNull();
  });
});
