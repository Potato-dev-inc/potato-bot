// @vitest-environment jsdom
// "Run only when something changes" in the New routine form (watching routines): the box, the check field and its plain hints,
// the request it builds, what the list shows for a watching routine, and the sentence for a check the app refuses.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { buildRoutineInput, DEFAULT_ROUTINE_FORM, fakeRoutinesTransport, ROUTINE_COPY, type FakeRoutinesTransport, type RoutineView } from '../../../lib/potato-routines';
import { fakeTrashTransport } from '../../../lib/potato-trash';
import { RoutinesTab } from './RoutinesTab';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 2, 8, 0, 0).getTime();

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
async function render(transport: FakeRoutinesTransport) {
  await act(async () => {
    root.render(<RoutinesTab bot={{ id: 'nimbus', name: 'Nimbus' }} transport={transport} trash={fakeTrashTransport()} now={NOW} />);
  });
  await flush();
}
function setField(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(el, value);
  el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}
const q = <T extends Element = HTMLInputElement>(selector: string) => host.querySelector<T>(selector)!;
const button = (label: string) => [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === label);
async function click(el: HTMLElement | undefined) {
  expect(el, 'something to click').toBeDefined();
  await act(async () => el!.click());
  await flush();
}
const watchBox = () => q('input[type="checkbox"]');
const labelOf = (text: string) => [...host.querySelectorAll('label')].find((l) => l.textContent?.includes(text));
const submit = async () => {
  await act(async () => q<HTMLFormElement>('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
  await flush();
};

async function openForm(transport = fakeRoutinesTransport([])) {
  await render(transport);
  await click(button(ROUTINE_COPY.newRoutine));
  setField(q('input[type="text"]'), 'Inbox watch');
  setField(q<HTMLTextAreaElement>('textarea'), 'Tell me what changed.');
  setField(q<HTMLSelectElement>('select'), 'every');
  setField(q('input[type="number"]'), '30');
  return transport;
}

describe('the watch box in the New routine form', () => {
  it('is there, unticked, and the form sends no watch until it is ticked', async () => {
    const t = await openForm();
    expect(watchBox().checked).toBe(false);
    expect(host.textContent).toContain('Run only when something changes');
    expect(labelOf('What to watch')).toBeUndefined();
    await submit();
    const sent = t.calls.find((c) => c.method === 'create')!.args[1] as Record<string, unknown>;
    expect('watch' in sent).toBe(false);
  });

  it('says in plain words that the look is free, that the first look only remembers, and what a check may be', async () => {
    await openForm();
    expect(host.querySelector('form')?.textContent).toContain('looks at the check below with no cost');
    expect(host.querySelector('form')?.textContent).toContain('The first look just remembers what it saw');
    await click(watchBox());
    const form = host.querySelector('form')!.textContent ?? '';
    expect(form).toContain('What to watch');
    expect(form).toContain('A command that only looks');
    expect(form).toContain('Nothing that changes things.');
    expect(form).toContain('What to do when it changes');
  });

  it('sends the check with the routine and shows the new row as watching, not looked at yet', async () => {
    const t = await openForm();
    await click(watchBox());
    setField(labelOf('What to watch')!.querySelector('input')!, ' ls /home/pm/inbox | wc -l ');
    await submit();
    expect(t.calls.find((c) => c.method === 'create')?.args).toEqual([
      'nimbus',
      { title: 'Inbox watch', prompt: 'Tell me what changed.', schedule: { kind: 'everyMinutes', minutes: 30 }, watch: { check: 'ls /home/pm/inbox | wc -l' } },
    ]);
    const row = host.querySelector('.ck-potato-routine')!.textContent ?? '';
    expect(row).toContain('Watching: ls /home/pm/inbox | wc -l');
    expect(row).toContain('not looked at yet');
  });

  it('offers "Page must also say" only for a web address, and sends it only then', async () => {
    const t = await openForm();
    await click(watchBox());
    expect(labelOf('Page must also say')).toBeUndefined();
    setField(labelOf('What to watch')!.querySelector('input')!, 'https://example.com/status');
    expect(labelOf('Page must also say')).toBeDefined();
    setField(labelOf('Page must also say')!.querySelector('input')!, ' ready ');
    await submit();
    expect((t.calls.find((c) => c.method === 'create')!.args[1] as { watch: unknown }).watch).toEqual({ check: 'https://example.com/status', contains: 'ready' });
    // typing words and then turning the address into a command drops them
    expect(buildRoutineInput({ ...DEFAULT_ROUTINE_FORM, title: 'a', prompt: 'b', watchOn: true, watchCheck: 'ls', watchContains: 'stale words' })).toMatchObject({ watch: { check: 'ls' } });
    expect((buildRoutineInput({ ...DEFAULT_ROUTINE_FORM, title: 'a', prompt: 'b', watchOn: true, watchCheck: 'ls', watchContains: 'stale words' }) as { watch: object }).watch).not.toHaveProperty('contains');
  });

  it('a ticked box with nothing to watch is refused on the screen and nothing is sent', async () => {
    const t = await openForm();
    await click(watchBox());
    await submit();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe(ROUTINE_COPY.watchMissing);
    expect(t.calls.some((c) => c.method === 'create')).toBe(false);
  });

  it('unticking the box takes the watch out of the request', async () => {
    const t = await openForm();
    await click(watchBox());
    setField(labelOf('What to watch')!.querySelector('input')!, 'ls');
    await click(watchBox());
    await submit();
    expect('watch' in (t.calls.find((c) => c.method === 'create')!.args[1] as object)).toBe(false);
  });

  it('shows the app’s sentence when it refuses the check, and keeps the form', async () => {
    const t = await openForm(fakeRoutinesTransport([]));
    t.create = () => Promise.reject(new Error('POTATO_CHECK_INVALID: A check may only look, never change anything. Use ls, cat, grep, stat, test, git status and the like, or give a web address.'));
    await click(watchBox());
    setField(labelOf('What to watch')!.querySelector('input')!, 'rm -rf /home/pm');
    await submit();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('A check may only look, never change anything. Use ls, cat, grep, stat, test, git status and the like, or give a web address.');
    expect(host.querySelector('form')).not.toBeNull();
  });
});

describe('the list', () => {
  const base: RoutineView = {
    id: 'r-1',
    bot: 'nimbus',
    title: 'Inbox watch',
    prompt: 'x',
    schedule: { kind: 'everyMinutes', minutes: 30 },
    summary: 'Every 30 minutes',
    status: 'active',
    paused: false,
    done: false,
    lastRunMs: null,
    nextRunMs: NOW + 30 * 60_000,
    watch: { check: 'cat status.txt', kind: 'command', contains: null, lastCheckedMs: new Date(2026, 10, 2, 7, 30, 0).getTime() },
  };

  it('shows what a watching routine watches and when it last looked', async () => {
    await render(fakeRoutinesTransport([base]));
    const row = host.querySelector('.ck-potato-routine')!.textContent ?? '';
    expect(row).toContain('Watching: cat status.txt');
    expect(row).toContain('last look Nov 2, 7:30');
  });

  it('shows nothing extra for a routine that is not watching', async () => {
    await render(fakeRoutinesTransport([{ ...base, watch: null }, { ...base, id: 'r-2', watch: undefined }]));
    expect(host.textContent).not.toContain('Watching:');
  });
});
