// @vitest-environment jsdom
// The step list says what each step did, opens to the full command, the output tail, the exit code and the real time,
// folds long runs, and only says "Stopped at" when the turn really stopped.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { foldLong, workTitle, type WorkRunView, type WorkStepView } from '../../../lib/potato-work';
import { WorkSteps } from './WorkSteps';

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

const step = (id: string, status: WorkStepView['status'], a: number, b: number | null, over: Partial<WorkStepView> = {}): WorkStepView => ({
  id,
  tool: 'run_terminal',
  status,
  startedMs: a,
  endedMs: b,
  log: [],
  ...over,
});
const run = (steps: WorkStepView[], over: Partial<WorkRunView> = {}): WorkRunView => ({ startedMs: 0, steps, failed: false, endedMs: null, ...over });
const show = (r: WorkRunView, now = 400_000) => act(() => root.render(<WorkSteps run={r} now={now} />));
const text = (sel: string) => host.querySelector(sel)?.textContent?.replace(/ /g, ' ') ?? '';
const click = (el: Element | null) =>
  act(() => {
    el?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });

describe('WorkSteps, what each step did', () => {
  it('writes the plain description instead of "Used run terminal"', () => {
    show(run([step('a', 'done', 0, 2_100, { detail: 'Ran: ffmpeg -i clip.mp4 -vf scale=720 out.mp4' }), step('b', 'running', 2_100, null, { detail: 'Wrote report.md (2.1 kB)' })]));
    const rows = [...host.querySelectorAll('.ck-work-label')].map((x) => x.textContent?.replace(/ /g, ' '));
    expect(rows).toEqual(['Ran: ffmpeg -i clip.mp4 -vf scale=720 out.mp4', 'Wrote report.md (2.1 kB)']);
    expect(host.textContent).not.toContain('Used run terminal');
    expect(text('.ck-work-title')).toBe('Wrote report.md (2.1 kB)');
  });

  it('shows the model\'s reason under the row, and the error line of a failed row', () => {
    show(run([step('a', 'error', 0, 800, { detail: 'Ran: make', why: 'build the project', error: 'make: *** No rule to make target', exit: 2 })], { endedMs: 900 }));
    act(() => root.render(<WorkSteps run={run([step('a', 'error', 0, 800, { detail: 'Ran: make', why: 'build the project', error: 'make: *** No rule to make target', exit: 2 })], { endedMs: 900 })} now={900} />));
    click(host.querySelector('.ck-work-head'));
    expect(text('.ck-work-why')).toBe('build the project');
    expect(text('.ck-work-error')).toBe('make: *** No rule to make target');
  });

  it('opens a row to the full command, the last lines of output, the exit code and the real duration', () => {
    const full = 'cd /work && ffmpeg -i clip.mp4 \\\n  -vf scale=720 out.mp4';
    show(run([step('a', 'done', 1_000, 5_300, { detail: 'Ran: ffmpeg -i clip.mp4', full, tail: 'frame=10\nframe=20\ndone', exit: 0 })]));
    expect(host.querySelector('.ck-work-full')).toBeNull();
    click(host.querySelector('.ck-work-label-btn'));
    expect(text('.ck-work-full')).toContain('-vf scale=720 out.mp4');
    expect(text('.ck-work-tail')).toBe('frame=10\nframe=20\ndone');
    expect(text('.ck-work-meta')).toBe('Exit 0 · 4s');
    click(host.querySelector('.ck-work-label-btn'));
    expect(host.querySelector('.ck-work-full')).toBeNull();
  });

  it('gives a fast step "<1s" only when it was fast, and a slow one its real time', () => {
    show(run([step('a', 'done', 0, 300, { detail: 'Ran: ls' }), step('b', 'done', 300, 330_000, { detail: 'Ran: cargo build' })], { endedMs: 331_000 }));
    click(host.querySelector('.ck-work-head'));
    expect([...host.querySelectorAll('.ck-work-time')].map((x) => x.textContent)).toEqual(['<1s', '5m 29s']);
  });
});

describe('WorkSteps, the header and the fold', () => {
  it('does not say "Stopped at" while the turn is still running after a failed step', () => {
    const r = run([step('a', 'error', 0, 400, { detail: 'Ran: false' }), step('b', 'done', 400, 900, { detail: 'Ran: ls' })]);
    show(r);
    expect(text('.ck-work-title')).toBe('Thinking');
    expect(host.textContent).not.toContain('Stopped at');
    expect(workTitle(r, 1000)).toBe('Thinking');
  });

  it('says "Stopped at" for a turn that really failed, and says the job goes on when it does', () => {
    const failed = run([step('a', 'done', 0, 100), step('b', 'error', 100, 500, { detail: 'Ran: make' })], { failed: true });
    show(failed);
    expect(text('.ck-work-title')).toBe('Stopped at: Ran: make');
    show({ ...failed, continues: true });
    expect(text('.ck-work-title')).toBe('Hit an error. The job goes on by itself');
    expect(host.textContent).not.toContain('Stopped at');
  });

  it('says the job goes on after a turn that ended well inside a running job', () => {
    show(run([step('a', 'done', 0, 1000)], { endedMs: 2000, continues: true }));
    expect(text('.ck-work-title')).toBe('Worked for 2s · 1 step · the job goes on');
  });

  it('folds a long run: the newest rows stay, the older ones wait behind a button', () => {
    const steps = Array.from({ length: 12 }, (_, i) => step(`s${i}`, i === 11 ? 'running' : 'done', i * 1000, i === 11 ? null : i * 1000 + 500, { detail: `Ran: step ${i}` }));
    show(run(steps));
    expect(host.querySelectorAll('.ck-work-step').length).toBe(5);
    expect(text('.ck-work-title')).toBe('Ran: step 11');
    expect(text('.ck-work-earlier-btn')).toBe('Show 7 earlier steps');
    click(host.querySelector('.ck-work-earlier-btn'));
    expect(host.querySelectorAll('.ck-work-step').length).toBe(12);
    expect(text('.ck-work-earlier-btn')).toBe('Hide earlier steps');
  });

  it('foldLong keeps a running step in view and leaves a short list alone', () => {
    const few = Array.from({ length: 5 }, (_, i) => step(`f${i}`, 'done', i, i + 1));
    expect(foldLong(few)).toEqual({ hidden: [], shown: few });
    const many = [step('r', 'running', 0, null), ...Array.from({ length: 9 }, (_, i) => step(`d${i}`, 'done', i + 1, i + 2))];
    const { hidden, shown } = foldLong(many);
    expect(shown.some((s) => s.id === 'r')).toBe(true);
    expect(hidden.every((s) => s.status === 'done')).toBe(true);
  });

  it('two rows that share an id still draw as two rows', () => {
    show(run([step('dsml-0', 'done', 0, 1000, { detail: 'Ran: a' }), step('dsml-0', 'done', 2000, 3000, { detail: 'Ran: b' })], { endedMs: 3000 }));
    click(host.querySelector('.ck-work-head'));
    expect(host.querySelectorAll('.ck-work-step')).toHaveLength(2);
  });
});
