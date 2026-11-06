// @vitest-environment jsdom
// The live step list: steps go running, done and fold; a retry reads as a step; a failed job puts the X where it
// stopped; a log line stays small; the CSS holds still under reduced motion; nothing stray (">>>") is drawn.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { announce, elapsedLabel, workLabel, type WorkRunView, type WorkStepView } from '../../../lib/potato-work';
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

const step = (id: string, tool: string, status: WorkStepView['status'], startedMs: number, endedMs: number | null = null, log: string[] = []): WorkStepView => ({
  id,
  tool,
  status,
  startedMs,
  endedMs,
  log,
});
const run = (steps: WorkStepView[], over: Partial<WorkRunView> = {}): WorkRunView => ({ startedMs: 0, steps, failed: false, endedMs: null, ...over });
const show = (r: WorkRunView, now = 20_000, owner?: string) => act(() => root.render(<WorkSteps run={r} now={now} owner={owner} />));
const text = (sel: string) => host.querySelector(sel)?.textContent?.replace(/ /g, ' ') ?? '';
const click = (el: Element | null) =>
  act(() => {
    el?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });

describe('WorkSteps', () => {
  it('shows the running step in the header with a live time, a spinning arc, its log, and the done step with its time', () => {
    show(run([step('a', 'computer_observe', 'done', 0, 3000), step('b', 'computer_act', 'running', 3000, null, ['Started computer act'])]));
    expect(text('.ck-work-title')).toBe('Used the mouse and keyboard');
    expect(text('.ck-work-clock')).toBe('20s');
    expect(host.querySelectorAll('.ck-work-step')).toHaveLength(2);
    expect(host.querySelectorAll('.ck-work-arc')).toHaveLength(1);
    expect(text('.ck-work-step.is-done .ck-work-time')).toBe('3s');
    expect(text('.ck-work-log')).toBe('Started computer act');
    expect(host.querySelector('.ck-work-head')?.getAttribute('aria-expanded')).toBe('true');
    expect((host.querySelector('.ck-work-rail-fill') as HTMLElement).style.transform).toBe('scaleY(0.5)');
    expect(text('output')).toBe('Used the mouse and keyboard');
  });

  it('folds to "Worked for 1m 20s · 4 steps" when the turn ends, and opens again on a click', () => {
    const steps = ['a', 'b', 'c', 'd'].map((id, i) => step(id, 'browser_act', 'done', i * 1000, i * 1000 + 900));
    show(run(steps, { endedMs: 80_000 }));
    expect(text('.ck-work-title')).toBe('Worked for 1m 20s · 4 steps');
    expect(host.querySelector('.ck-work-list')).toBeNull();
    expect(host.querySelector('.ck-work-clock')).toBeNull();
    click(host.querySelector('.ck-work-head'));
    expect(host.querySelectorAll('.ck-work-step')).toHaveLength(4);
    click(host.querySelector('.ck-work-head'));
    expect(host.querySelector('.ck-work-list')).toBeNull();
  });

  it('folds by itself when the run settles', () => {
    show(run([step('a', 'browser_act', 'running', 0)]));
    expect(host.querySelector('.ck-work-list')).not.toBeNull();
    show(run([step('a', 'browser_act', 'done', 0, 2000)], { endedMs: 2000 }));
    expect(host.querySelector('.ck-work-list')).toBeNull();
    expect(text('.ck-work-title')).toBe('Worked for 2s · 1 step');
  });

  it('writes a retry as its own step', () => {
    show(run([step('a', 'browser_act', 'done', 0, 1000), step('retry-1', 'retry', 'retry', 1000)]));
    expect(text('.ck-work-title')).toBe('Model stopped answering, retrying');
    expect(workLabel({ tool: 'retry', status: 'retry' })).toBe('Model stopped answering, retrying');
  });

  it('puts the X on the step the job stopped at and keeps the list open', () => {
    show(run([step('a', 'browser_act', 'done', 0, 1000), step('b', 'computer_act', 'error', 1000, 1500)], { failed: true }));
    expect(text('.ck-work-title')).toBe('Stopped at: Used the mouse and keyboard');
    expect(host.querySelector('.ck-work-step.is-error [data-status="error"] svg')).not.toBeNull();
    expect(host.querySelector('.ck-work-step.is-done [data-status="done"] svg')).not.toBeNull();
    expect(host.querySelector('.ck-work-list')).not.toBeNull();
    expect(text('output')).toBe('Stopped at: Used the mouse and keyboard');
  });

  it('labels the list with the bot name in a group', () => {
    show(run([step('a', 'browser_act', 'running', 0)]), 5000, 'Potato');
    expect(text('.ck-work-owner')).toBe('Potato');
    expect(text('output')).toBe('Potato: Used the page');
  });

  it('has no stray marks in what it draws', () => {
    show(run([step('a', 'browser_act', 'done', 0, 1000), step('b', 'run_command', 'running', 1000, null, ['Started run command'])]));
    expect(host.textContent).not.toMatch(/>>>|undefined|NaN|\[object/);
  });

  it('holds still under reduced motion and uses no colour classes of its own', () => {
    const css = readFileSync(join(process.cwd(), 'src/styles/cockpit-potato-work.css'), 'utf8');
    const block = css.slice(css.indexOf('@media (prefers-reduced-motion: reduce)'));
    for (const cls of ['.ck-work-arc', '.ck-work-draw', '.ck-work-wave > span', '.ck-work-logline']) expect(block).toContain(cls);
    expect(block).toMatch(/animation:\s*none/);
    expect(css).not.toMatch(/zinc|#[0-9a-fA-F]{3,8}\b(?!\s*45%)/);
    const tsx = readFileSync(join(process.cwd(), 'src/components/cockpit/potato/WorkSteps.tsx'), 'utf8');
    expect(tsx).not.toMatch(/zinc-|^import .*(motion|lucide-react)/m);
  });
});

describe('the words and times', () => {
  it('writes seconds, minutes and hours', () => {
    expect(elapsedLabel(400)).toBe('<1s');
    expect(elapsedLabel(8_000)).toBe('8s');
    expect(elapsedLabel(80_000)).toBe('1m 20s');
    expect(elapsedLabel(3_900_000)).toBe('1h 5m');
  });
  it('announces nothing for an empty run', () => {
    expect(announce(run([]))).toBe('');
  });
});
