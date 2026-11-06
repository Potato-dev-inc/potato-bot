// @vitest-environment jsdom
// The Routines tab: the list (plain words, the next run, the pause switch), Run now, Delete (a question first), the
// New routine form (its values become the request), and what a failed call says in one sentence.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakeRoutinesTransport, ROUTINE_COPY, type FakeRoutinesTransport, type RoutineRunView, type RoutineView } from '../../../lib/potato-routines';
import { fakeTrashTransport, type FakeTrashTransport } from '../../../lib/potato-trash';
import { ROUTINE_DELETED_COPY, RoutinesTab } from './RoutinesTab';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Tuesday 2 November 2026, 8 am on the computer's clock.
const NOW = new Date(2026, 10, 2, 8, 0, 0).getTime();
const NEXT_MORNING = new Date(2026, 10, 3, 6, 0, 0).getTime();

const weekdays: RoutineView = {
  id: 'r-1',
  bot: 'nimbus',
  title: 'Morning sweep',
  prompt: 'Check my inbox.',
  schedule: { kind: 'weekdays', time: '06:00' },
  summary: 'Weekdays 6am',
  status: 'active',
  paused: false,
  done: false,
  lastRunMs: null,
  nextRunMs: NEXT_MORNING,
};
const hourly: RoutineView = {
  ...weekdays,
  id: 'r-2',
  title: 'Hourly check',
  schedule: { kind: 'hourly' },
  summary: 'Every hour',
  status: 'paused',
  paused: true,
  nextRunMs: null,
};
const someoneElses: RoutineView = { ...weekdays, id: 'r-3', bot: 'other', title: 'Not this bot' };

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

const flush = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

async function render(transport: FakeRoutinesTransport, trash: FakeTrashTransport = fakeTrashTransport()) {
  await act(async () => {
    root.render(<RoutinesTab bot={{ id: 'nimbus', name: 'Nimbus' }} transport={transport} trash={trash} now={NOW} />);
  });
  await flush();
}

const button = (label: string) => [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === label);
const rows = () => [...host.querySelectorAll<HTMLElement>('.ck-potato-routine')];

/** Sets a field's value the way React hears it (a select changes on `change`). */
function setField(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  const proto =
    el instanceof HTMLSelectElement
      ? HTMLSelectElement.prototype
      : el instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(el, value);
  el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}

const field = (selector: string) => host.querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>(selector)!;

async function click(el: HTMLElement | undefined) {
  expect(el, 'button to click').toBeDefined();
  await act(async () => {
    el!.click();
  });
  await flush();
}

describe('Routines tab', () => {
  it('lists this bot routines in plain words, with the next run and a pause switch', async () => {
    const transport = fakeRoutinesTransport([weekdays, hourly, someoneElses]);
    await render(transport);
    expect(rows()).toHaveLength(2);
    expect(rows()[0].textContent).toContain('Morning sweep');
    expect(rows()[0].textContent).toContain('Weekdays 6am');
    expect(rows()[0].textContent).toContain('Next run tomorrow at 6:00');
    expect(rows()[0].querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('true');
    expect(rows()[1].textContent).toContain('Paused');
    expect(rows()[1].querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('false');
    expect(host.textContent).not.toContain('Not this bot');
  });

  it('says in one line that there are no routines yet', async () => {
    await render(fakeRoutinesTransport([]));
    expect(host.textContent).toContain(ROUTINE_COPY.empty);
    expect(rows()).toHaveLength(0);
  });

  it('the pause switch pauses and resumes the routine', async () => {
    const transport = fakeRoutinesTransport([weekdays]);
    await render(transport);
    await click(rows()[0].querySelector<HTMLButtonElement>('[role="switch"]') ?? undefined);
    expect(transport.calls.find((c) => c.method === 'pause')?.args).toEqual(['r-1', true]);
    expect(rows()[0].textContent).toContain('Paused');
    expect(rows()[0].querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('false');
    await click(rows()[0].querySelector<HTMLButtonElement>('[role="switch"]') ?? undefined);
    expect(transport.calls.filter((c) => c.method === 'pause').at(-1)?.args).toEqual(['r-1', false]);
    expect(rows()[0].textContent).toContain('Next run');
  });

  it('Run now sends the prompt to the bot and says so', async () => {
    const transport = fakeRoutinesTransport([weekdays]);
    await render(transport);
    await click(button('Run now'));
    expect(transport.calls.find((c) => c.method === 'runNow')?.args).toEqual(['r-1']);
    expect(host.textContent).toContain('Sent to Nimbus.');
  });

  it('Delete removes the row at once, and Undo brings it back from Recently deleted', async () => {
    const transport = fakeRoutinesTransport([weekdays]);
    const trash = fakeTrashTransport();
    await render(transport, trash);
    await click(button('Delete'));
    expect(transport.calls.find((c) => c.method === 'remove')?.args).toEqual(['r-1']);
    expect(rows()).toHaveLength(0);
    expect(host.textContent).toContain(ROUTINE_DELETED_COPY);
    await click(button('Undo'));
    expect(trash.calls).toEqual([{ method: 'restore', args: ['trash-r-1'] }]);
  });

  it('the New routine form sends the values it was filled with, then shows the new row', async () => {
    const transport = fakeRoutinesTransport([]);
    await render(transport);
    await click(button(ROUTINE_COPY.newRoutine));
    const form = host.querySelector('form');
    expect(form).not.toBeNull();
    setField(field('input[type="text"]'), '  Morning sweep ');
    setField(field('textarea'), 'Check my inbox.');
    setField(field('select'), 'weekdays');
    setField(field('input[type="time"]'), '06:00');
    await act(async () => {
      form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    });
    await flush();
    expect(transport.calls.find((c) => c.method === 'create')?.args).toEqual([
      'nimbus',
      { title: 'Morning sweep', prompt: 'Check my inbox.', schedule: { kind: 'weekdays', time: '06:00' } },
    ]);
    expect(host.querySelector('form')).toBeNull();
    expect(rows()).toHaveLength(1);
    expect(rows()[0].textContent).toContain('Morning sweep');
  });

  it('a blank name is refused on the screen, and nothing is sent', async () => {
    const transport = fakeRoutinesTransport([]);
    await render(transport);
    await click(button(ROUTINE_COPY.newRoutine));
    setField(field('textarea'), 'Check my inbox.');
    await click(button(ROUTINE_COPY.save));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Give the routine a name.');
    expect(transport.calls.some((c) => c.method === 'create')).toBe(false);
  });

  it('every few minutes below 15 is refused, and once takes a date and time', async () => {
    const transport = fakeRoutinesTransport([]);
    await render(transport);
    await click(button(ROUTINE_COPY.newRoutine));
    setField(field('input[type="text"]'), 'Quick look');
    setField(field('textarea'), 'Look at the queue.');
    setField(field('select'), 'every');
    await flush();
    setField(field('input[type="number"]'), '10');
    await click(button(ROUTINE_COPY.save));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Pick every 15 minutes or more.');

    setField(field('select'), 'once');
    await flush();
    setField(field('input[type="datetime-local"]'), '2026-11-05T09:30');
    await click(button(ROUTINE_COPY.save));
    expect(transport.calls.find((c) => c.method === 'create')?.args[1]).toEqual({
      title: 'Quick look',
      prompt: 'Look at the queue.',
      schedule: { kind: 'once', at: '2026-11-05T09:30' },
    });
  });

  it('a failed load says why in one plain sentence', async () => {
    await render(fakeRoutinesTransport([], 'POTATO_STORE_INVALID: routines.json is damaged.'));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('routines.json is damaged.');
    expect(rows()).toHaveLength(0);
  });
});

describe('Routines tab: cron schedule', () => {
  const NINE = (day: number) => new Date(2026, 10, day, 9, 0, 0).getTime();
  const cronFake = (fn: (expr: string) => number[]) => fakeRoutinesTransport([], undefined, { cronPreview: fn });

  async function openCron(transport: FakeRoutinesTransport) {
    await render(transport);
    await click(button(ROUTINE_COPY.newRoutine));
    await act(async () => {
      setField(field('input[type="text"]'), 'Standup');
      setField(field('textarea'), 'Summarise the board.');
    });
    await act(async () => setField(field('select'), 'cron'));
    await flush();
  }
  const typeCron = async (value: string) => {
    await act(async () => setField(cronBox(), value));
    await flush();
  };
  const cronBox = () => host.querySelector<HTMLInputElement>('input[aria-label="Cron"]')!;
  const previewItems = () => [...host.querySelectorAll('.ck-potato-cron-preview li')].map((li) => li.textContent);

  it('Cron is a choice in How often, and its input shows the next three runs worked out by the Rust side', async () => {
    const transport = cronFake((expr) => (expr === '0 9 * * mon-fri' ? [NINE(3), NINE(4), NINE(5)] : []));
    await openCron(transport);
    expect(cronBox()).not.toBeNull();
    await typeCron('0 9 * * mon-fri');
    await flush();
    expect(transport.calls.filter((c) => c.method === 'cronPreview').at(-1)?.args).toEqual(['0 9 * * mon-fri']);
    const items = previewItems();
    expect(items).toHaveLength(3);
    expect(items[0]).toContain('Nov 3');
    expect(items[0]).toContain('9:00');
    expect(items[2]).toContain('Nov 5');
    expect(host.querySelector('.ck-potato-cron-preview')?.textContent).toContain('Next runs');
  });

  it('a cron the Rust side refuses shows its plain sentence, with no runs listed', async () => {
    const transport = cronFake(() => {
      throw new Error('POTATO_ROUTINE_INVALID: That schedule can run twice only 5 minutes apart. Runs must be at least 15 minutes apart.');
    });
    await openCron(transport);
    await typeCron('*/5 * * * *');
    await flush();
    expect(host.querySelector('.ck-potato-cron-error')?.textContent).toBe('That schedule can run twice only 5 minutes apart. Runs must be at least 15 minutes apart.');
    expect(previewItems()).toHaveLength(0);
  });

  it('typing a new cron replaces the old runs and an old slow answer never wins', async () => {
    const transport = cronFake((expr) => (expr === 'a' ? [NINE(3)] : [NINE(4)]));
    await openCron(transport);
    await act(async () => {
      setField(cronBox(), 'a');
      setField(cronBox(), 'b');
    });
    await flush();
    expect(previewItems()).toHaveLength(1);
    expect(previewItems()[0]).toContain('Nov 4');
  });

  it('Save sends the cron as the schedule, and a blank cron is refused on the screen', async () => {
    const transport = cronFake(() => [NINE(3)]);
    await openCron(transport);
    await click(button(ROUTINE_COPY.save));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('cron');
    expect(transport.calls.some((c) => c.method === 'create')).toBe(false);
    await typeCron('  0 9 * * mon-fri ');
    await click(button(ROUTINE_COPY.save));
    expect(transport.calls.find((c) => c.method === 'create')?.args).toEqual([
      'nimbus',
      { title: 'Standup', prompt: 'Summarise the board.', schedule: { kind: 'cron', expr: '0 9 * * mon-fri' } },
    ]);
    expect(rows()).toHaveLength(1);
  });

  it('a save the Rust side refuses keeps the form open with its sentence', async () => {
    const transport = cronFake(() => [NINE(3)]);
    await openCron(transport);
    await typeCron('0 9 * * mon-fri');
    await flush();
    // The same call now fails the way the real one does for a refused cron.
    (transport as { create: unknown }).create = () => Promise.reject(new Error('POTATO_ROUTINE_INVALID: The hour part "0 25 * * *" has 25, which is outside 0 to 23.'));
    await click(button(ROUTINE_COPY.save));
    expect(host.querySelector('form')).not.toBeNull();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('outside 0 to 23');
  });
});

describe('Routines tab: run history', () => {
  const at = (hour: number, minute = 0) => new Date(2026, 10, 2, hour, minute, 0).getTime();
  const runs: RoutineRunView[] = [
    { routineId: 'r-1', atMs: at(7), result: 'ok', text: 'The bot replied.', replyId: 'm-reply-2' },
    { routineId: 'r-1', atMs: at(6), result: 'failed', text: 'The model is not answering right now. Try again in a minute.', replyId: null },
    { routineId: 'r-1', atMs: at(5), result: 'skipped', text: 'Skipped: bots were paused.', replyId: null },
    { routineId: 'r-1', atMs: at(4), result: 'ok', text: 'The bot replied.', replyId: 'm-reply-1' },
    { routineId: 'r-2', atMs: at(3), result: 'ok', text: 'The bot replied.', replyId: 'm-other' },
  ];
  const history = (i = 0) => rows()[i].querySelector<HTMLButtonElement>('button[aria-expanded]');
  const runRows = (i = 0) => [...rows()[i].querySelectorAll<HTMLElement>('.ck-potato-routine-run')];

  async function renderWith(transport: FakeRoutinesTransport, onOpenMessage?: (id: string) => void) {
    await act(async () => {
      root.render(<RoutinesTab bot={{ id: 'nimbus', name: 'Nimbus' }} transport={transport} trash={fakeTrashTransport()} now={NOW} onOpenMessage={onOpenMessage} />);
    });
    await flush();
  }

  it('each routine has a History button with its count, closed at first', async () => {
    await renderWith(fakeRoutinesTransport([weekdays, hourly], undefined, { runs }));
    expect(history(0)?.textContent).toBe('History (4)');
    expect(history(0)?.getAttribute('aria-expanded')).toBe('false');
    expect(history(1)?.textContent).toBe('History (1)');
    expect(runRows(0)).toHaveLength(0);
  });

  it('opening it lists the runs newest first with the time, the result and a plain sentence', async () => {
    await renderWith(fakeRoutinesTransport([weekdays], undefined, { runs }));
    await click(history(0) ?? undefined);
    expect(history(0)?.getAttribute('aria-expanded')).toBe('true');
    const list = runRows(0);
    expect(list).toHaveLength(4);
    expect(list[0].textContent).toContain('7:00');
    expect(list[0].textContent).toContain('OK');
    expect(list[0].textContent).toContain('The bot replied.');
    expect(list[1].textContent).toContain('Failed');
    expect(list[1].textContent).toContain('The model is not answering right now.');
    expect(list[2].textContent).toContain('Skipped');
    expect(list[2].textContent).toContain('bots were paused');
    expect(list[0].getAttribute('data-result')).toBe('ok');
    expect(list[1].getAttribute('data-result')).toBe('failed');
    expect(host.textContent).not.toContain('POTATO_');
  });

  it('only a run with a reply has the link, and clicking it opens that reply in the thread', async () => {
    const opened: string[] = [];
    await renderWith(fakeRoutinesTransport([weekdays], undefined, { runs }), (id) => opened.push(id));
    await click(history(0) ?? undefined);
    const links = runRows(0).map((r) => r.querySelector<HTMLButtonElement>('button.ck-potato-routine-run-link'));
    expect(links.map((l) => l?.textContent ?? null)).toEqual([ROUTINE_COPY.showReply, null, null, ROUTINE_COPY.showReply]);
    await click(links[3] ?? undefined);
    await click(links[0] ?? undefined);
    expect(opened).toEqual(['m-reply-1', 'm-reply-2']);
  });

  it('a routine with no runs yet has no History button, and a history that fails to load does not hide the routines', async () => {
    await renderWith(fakeRoutinesTransport([weekdays, hourly], undefined, { runs: [] }));
    expect(history(0)).toBeNull();
    expect(rows()).toHaveLength(2);

    const broken = fakeRoutinesTransport([weekdays], undefined, { runs });
    broken.runs = () => Promise.reject(new Error('POTATO_ROUTINE_INVALID: routine_runs.json is damaged.'));
    await renderWith(broken);
    expect(rows()).toHaveLength(1);
    expect(history(0)).toBeNull();
  });

  it('Run now reads the history again', async () => {
    const transport = fakeRoutinesTransport([weekdays], undefined, { runs: [] });
    await renderWith(transport);
    await click(button('Run now'));
    expect(transport.calls.filter((c) => c.method === 'runs').length).toBeGreaterThanOrEqual(2);
  });
});
