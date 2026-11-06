// @vitest-environment jsdom
// The Routines tab shows the bot's open waits (`wait_until`): what each waits for, the check, how many looks so far, when the next
// look and the give-up are, and a Cancel. A wait costs nothing and keeps the computer awake, and the tab says so.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakeRoutinesTransport, nextLookWords, type WaitView } from '../../../lib/potato-routines';
import { fakeTrashTransport } from '../../../lib/potato-trash';
import { RoutinesTab } from './RoutinesTab';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 2, 8, 0, 0).getTime();
const MIN = 60_000;

const exportWait: WaitView = {
  id: 'w-1',
  bot: 'nimbus',
  title: 'the export',
  check: 'test -f /home/pm/out.zip',
  kind: 'command',
  everyWords: '2 minutes',
  checks: 3,
  nextCheckMs: NOW + 2 * MIN,
  deadlineMs: NOW + 60 * MIN,
  last: '',
};
const pageWait: WaitView = {
  id: 'w-2',
  bot: 'nimbus',
  title: 'https://example.com/up',
  check: 'https://example.com/up',
  kind: 'url',
  everyWords: '5 minutes',
  checks: 0,
  nextCheckMs: NOW + 10_000,
  deadlineMs: NOW + 3 * 60 * MIN,
  last: 'starting up',
};
const someoneElses: WaitView = { ...exportWait, id: 'w-3', bot: 'other', title: 'not this bot' };

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const flush = () => act(async () => void (await new Promise((resolve) => setTimeout(resolve, 0))));
async function render(transport = fakeRoutinesTransport([], undefined, { waits: [exportWait, pageWait, someoneElses] })) {
  await act(async () => {
    root.render(<RoutinesTab bot={{ id: 'nimbus', name: 'Nimbus' }} transport={transport} trash={fakeTrashTransport()} now={NOW} />);
  });
  await flush();
  return transport;
}
const waitRows = () => [...host.querySelectorAll<HTMLElement>('.ck-potato-wait')];
const text = (el: Element | null | undefined) => el?.textContent ?? '';

describe('open waits in the Routines tab', () => {
  it('shows nothing when the bot waits for nothing', async () => {
    await render(fakeRoutinesTransport());
    expect(host.querySelector('.ck-potato-waits')).toBeNull();
  });

  it('lists this bot’s waits in plain words, not another bot’s', async () => {
    await render();
    expect(waitRows()).toHaveLength(2);
    const [first, second] = waitRows();
    expect(text(first)).toContain('the export');
    expect(text(first)).toContain('test -f /home/pm/out.zip');
    expect(text(first)).toContain('Looked 3 times · every 2 minutes');
    expect(text(first)).toContain('next look in 2 minutes');
    expect(text(first)).toContain('gives up in 1 hour');
    expect(text(second)).toContain('Not looked at yet · every 5 minutes');
    expect(text(second)).toContain('next look any moment');
    expect(text(second)).toContain('starting up');
    expect(host.textContent).not.toContain('not this bot');
  });

  it('says it costs nothing, ends with a done wait or the time, and keeps the computer awake', async () => {
    await render();
    const note = text(host.querySelector('.ck-potato-waits .ck-sub'));
    expect(note).toContain('no cost');
    expect(note).toContain('starts again when one is done or runs out of time');
    expect(note).toContain('keeps the computer awake');
  });

  it('Cancel gives the wait up and the row goes', async () => {
    const t = await render();
    const cancel = [...waitRows()[0]!.querySelectorAll('button')].find((b) => b.textContent === 'Cancel')!;
    await act(async () => cancel.click());
    await flush();
    expect(t.calls.find((c) => c.method === 'cancelWait')?.args).toEqual(['w-1']);
    expect(waitRows()).toHaveLength(1);
    expect(text(waitRows()[0])).toContain('https://example.com/up');
  });

  it('a failed Cancel keeps the row and says so in one sentence', async () => {
    const t = await render();
    t.cancelWait = () => Promise.reject(new Error('nope'));
    const cancel = [...waitRows()[0]!.querySelectorAll('button')].find((b) => b.textContent === 'Cancel')!;
    await act(async () => cancel.click());
    await flush();
    expect(waitRows()).toHaveLength(2);
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('That wait could not be cancelled.');
  });

  it('a wait list that cannot be read changes nothing else', async () => {
    const t = fakeRoutinesTransport([], undefined, { waits: [exportWait] });
    t.waits = () => Promise.reject(new Error('gone'));
    await render(t);
    expect(host.querySelector('.ck-potato-waits')).toBeNull();
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });
});

describe('when the next look is', () => {
  it('is said in the unit a person thinks in', () => {
    const at = (ms: number) => nextLookWords(NOW + ms, NOW);
    expect([at(0), at(20_000), at(60_000), at(5 * MIN), at(59 * MIN), at(60 * MIN), at(90 * MIN), at(36 * 60 * MIN), at(72 * 60 * MIN)]).toEqual([
      'any moment',
      'any moment',
      'in 1 minute',
      'in 5 minutes',
      'in 59 minutes',
      'in 1 hour',
      'in 2 hours',
      'in 36 hours',
      'in 3 days',
    ]);
    expect(at(-5 * MIN)).toBe('any moment');
  });
});
