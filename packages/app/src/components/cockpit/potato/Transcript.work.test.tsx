// @vitest-environment jsdom
// The live step list inside the real Transcript: the thinking row for a plain turn, then the list going running, done,
// folded as the app reports it, and the X when the turn failed.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import type { WorkRunView, WorkStepView } from '../../../lib/potato-work';
import { Transcript } from './Transcript';
import { bot, msg, you } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

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

const NOW = new Date(2026, 10, 1, 15, 0).getTime();
const s = (id: string, tool: string, status: WorkStepView['status'], a: number, b: number | null = null): WorkStepView => ({ id, tool, status, startedMs: a, endedMs: b, log: [] });
const draw = (over: Parameters<typeof bot>[0]) =>
  act(() =>
    root.render(
      <Transcript bot={bot(over)} messages={[]} agentLabel="Claude" transport={fakePotatoTransport({ bots: [bot()] })} onChanged={() => {}} now={NOW} openLink={() => {}} />,
    ),
  );
const title = () => host.querySelector('.ck-work-title')?.textContent?.replace(/ /g, ' ');

describe('Transcript with live steps', () => {
  it('shows only the thinking row for a plain chat turn, then the list as steps arrive, then the fold', () => {
    draw({ typing: true, work: null });
    expect(host.textContent).toContain('Fish is typing…');
    expect(host.querySelector('[data-testid="work-steps"]')).toBeNull();

    const running: WorkRunView = { startedMs: NOW - 5000, steps: [s('a', 'computer_observe', 'running', NOW - 5000)], failed: false, endedMs: null };
    draw({ typing: true, work: running });
    expect(host.textContent).not.toContain('is typing');
    expect(title()).toBe('Looked at the screen');
    expect(host.querySelectorAll('.ck-work-arc')).toHaveLength(1);

    const second: WorkRunView = { ...running, steps: [s('a', 'computer_observe', 'done', NOW - 5000, NOW - 3000), s('b', 'browser_navigate', 'running', NOW - 3000)] };
    draw({ typing: true, work: second });
    expect(title()).toBe('Opened a page');
    expect(host.querySelectorAll('.ck-work-step')).toHaveLength(2);
    expect(host.querySelector('.ck-work-step.is-done .ck-work-time')?.textContent).toBe('2s');

    const done: WorkRunView = { ...second, steps: second.steps.map((x) => ({ ...x, status: 'done' as const, endedMs: NOW })), endedMs: NOW };
    draw({ typing: false, work: done });
    expect(title()).toBe('Worked for 5s · 2 steps');
    expect(host.querySelector('.ck-work-list')).toBeNull();
    act(() => host.querySelector<HTMLButtonElement>('.ck-work-head')!.click());
    expect(host.querySelectorAll('.ck-work-step')).toHaveLength(2);
  });

  it('shows the X on the step a failed job stopped at', () => {
    const failed: WorkRunView = {
      startedMs: NOW - 4000,
      steps: [s('a', 'computer_observe', 'done', NOW - 4000, NOW - 3000), s('b', 'computer_act', 'error', NOW - 3000, NOW - 2000)],
      failed: true,
      endedMs: null,
    };
    draw({ typing: false, work: failed });
    expect(title()).toBe('Stopped at: Used the mouse and keyboard');
    expect(host.querySelector('.ck-work-step.is-error')).not.toBeNull();
  });

  it('shows a message the bot wrote mid-work above the live step list, after the request', () => {
    const running: WorkRunView = { startedMs: NOW - 5000, steps: [s('a', 'computer_observe', 'done', NOW - 5000, NOW - 3000), s('b', 'browser_navigate', 'running', NOW - 3000)], failed: false, endedMs: null };
    const mid = msg({ text: 'I will open the page and check the price.' });
    act(() =>
      root.render(
        <Transcript bot={bot({ typing: true, work: running })} messages={[you('find the price'), mid]} agentLabel="Claude" transport={fakePotatoTransport({ bots: [bot()] })} onChanged={() => {}} now={NOW} openLink={() => {}} />,
      ),
    );
    const text = host.textContent ?? '';
    expect(text.indexOf('find the price')).toBeGreaterThanOrEqual(0);
    expect(text.indexOf('I will open the page')).toBeGreaterThan(text.indexOf('find the price'));
    const list = host.querySelector('[data-testid="work-steps"]')!;
    const line = [...host.querySelectorAll('*')].find((el) => el.children.length === 0 && el.textContent?.includes('I will open the page'))!;
    // the message comes before the live list in the page
    expect(line.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(host.textContent).not.toContain('is typing');
  });
});
