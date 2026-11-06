// @vitest-environment jsdom
// The job bar: the goal, the step, the elapsed time, the spend, and Stop.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PotatoJob } from '../../../lib/potato';
import { JobBar } from './JobBar';
import { buttonByText } from './test-helpers';

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

const START = 1_700_000_000_000;
const job = (over: Partial<PotatoJob> = {}): PotatoJob => ({ goal: 'Fix the failing build', step: 'Running the tests', startedMs: START, spendNano: 250_000_000, state: 'running', ...over });

describe('JobBar', () => {
  it('shows the goal, the step, the time it has run and the spend', () => {
    act(() => root.render(<JobBar job={job()} now={START + 83_000} onStop={() => {}} />));
    expect(host.querySelector('.ck-potato-job-goal')?.textContent).toBe('Fix the failing build');
    expect(host.querySelector('.ck-potato-job-facts')?.textContent).toBe('Running the tests · 1m 23s · $0.25');
  });

  it('a waiting job says it waits for you instead of a step', () => {
    act(() => root.render(<JobBar job={job({ state: 'waiting', spendNano: 1_200_000_000 })} now={START + 5_000} onStop={() => {}} />));
    expect(host.querySelector('.ck-potato-job-facts')?.textContent).toBe('Waiting for you · 5s · $1.20');
  });

  it('a job with no step yet says it is starting', () => {
    act(() => root.render(<JobBar job={job({ step: '' })} now={START} onStop={() => {}} />));
    expect(host.querySelector('.ck-potato-job-facts')?.textContent).toContain('Starting');
  });

  it('Stop calls onStop once', () => {
    const onStop = vi.fn();
    act(() => root.render(<JobBar job={job()} now={START} onStop={onStop} />));
    act(() => buttonByText(host, 'Stop')!.click());
    expect(onStop).toHaveBeenCalledTimes(1);
  });
});
