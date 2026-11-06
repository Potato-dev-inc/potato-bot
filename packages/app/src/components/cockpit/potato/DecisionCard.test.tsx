// @vitest-environment jsdom
// A decision card: an approval (three buttons, the action in a code line), a question (one button per choice), a Keep
// going card (its choices as given), the quiet line after an answer, a cancelled card, and a failed answer.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DecisionCard, answeredLine, choiceIndex } from './DecisionCard';
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

const buttons = () => [...host.querySelectorAll('button')].map((b) => b.textContent?.trim());

const approval = (over: Partial<Parameters<typeof DecisionCard>[0]> = {}) => (
  <DecisionCard
    botName="Potato"
    card={{ kind: 'approval', why: 'It would publish the package to npm.', choices: [] }}
    text="npm publish"
    onAnswer={() => Promise.resolve()}
    {...over}
  />
);

describe('DecisionCard: approval', () => {
  it('shows the action in a code line, the why, and the three buttons', () => {
    act(() => root.render(approval()));
    expect(host.querySelector('code')?.textContent).toBe('npm publish');
    expect(host.textContent).toContain('It would publish the package to npm.');
    expect(buttons()).toEqual(['Allow once', 'Allow for this job', 'Deny']);
  });

  it('sends allow_once, allow_job and deny for the matching buttons', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () => root.render(approval({ onAnswer })));
    await act(async () => buttonByText(host, 'Allow for this job')!.click());
    expect(onAnswer).toHaveBeenLastCalledWith('allow_job');
  });

  it('after an answer the buttons go and a quiet line says what was chosen', async () => {
    await act(async () => root.render(approval()));
    await act(async () => buttonByText(host, 'Allow once')!.click());
    expect(buttons()).toEqual([]);
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('Allowed once.');
    expect(host.querySelector('.ck-potato-decision.is-closed')).not.toBeNull();
  });

  it('is closed from the thread when the answer row is already there (after a reload)', () => {
    act(() => root.render(approval({ answer: 'deny' })));
    expect(buttons()).toEqual([]);
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('Denied.');
  });

  it('disables the buttons while the answer is sent', async () => {
    let finish: () => void = () => {};
    const onAnswer = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    await act(async () => root.render(approval({ onAnswer })));
    await act(async () => buttonByText(host, 'Deny')!.click());
    expect(host.querySelectorAll('button:disabled').length).toBe(3);
    await act(async () => finish());
  });

  it('a failed answer shows its sentence and leaves the buttons on', async () => {
    const onAnswer = vi.fn(() => Promise.reject(new Error('POTATO_CARD_ANSWERED: That card was already answered.')));
    await act(async () => root.render(approval({ onAnswer })));
    await act(async () => buttonByText(host, 'Allow once')!.click());
    expect(host.querySelector('[role="status"]')?.textContent).toBe('That card was already answered.');
    expect(buttons()).toEqual(['Allow once', 'Allow for this job', 'Deny']);
    expect(buttonByText(host, 'Allow once')!.disabled).toBe(false);
  });
});

describe('DecisionCard: question', () => {
  const question = (over: Partial<Parameters<typeof DecisionCard>[0]> = {}) => (
    <DecisionCard
      botName="Potato"
      card={{ kind: 'question', why: 'Two folders match.', choices: ['Use the staging one', 'Use the live one'] }}
      text="Which folder should I use?"
      onAnswer={() => Promise.resolve()}
      {...over}
    />
  );

  it('shows the question and a button per choice, in order', () => {
    act(() => root.render(question()));
    expect(host.textContent).toContain('Which folder should I use?');
    expect(buttons()).toEqual(['Use the staging one', 'Use the live one']);
  });

  it('sends the number of the chosen button as text', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () => root.render(question({ onAnswer })));
    await act(async () => buttonByText(host, 'Use the live one')!.click());
    expect(onAnswer).toHaveBeenCalledWith('1');
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('You chose: Use the live one.');
  });

  it('a Keep going card shows its choices as given', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () =>
      root.render(
        <DecisionCard
          botName="Potato"
          card={{ kind: 'question', why: 'The job is still running.', choices: ['Keep going', 'Stop'], keepGoing: true }}
          text="I hit the 4 hour limit."
          onAnswer={onAnswer}
        />,
      ),
    );
    expect(buttons()).toEqual(['Keep going', 'Stop']);
    await act(async () => buttonByText(host, 'Stop')!.click());
    expect(onAnswer).toHaveBeenCalledWith('1');
  });
});

describe('DecisionCard: results', () => {
  it('a cancelled card says Cancelled and has no buttons', () => {
    act(() => root.render(approval({ answer: 'cancelled' })));
    expect(buttons()).toEqual([]);
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('Cancelled');
  });

  it('answeredLine reads every answer form', () => {
    const choices = ['A', 'B'];
    expect(answeredLine('allow_job', choices)).toBe('Allowed for this job.');
    expect(answeredLine('choice:1', choices)).toBe('You chose: B.');
    expect(answeredLine('0', choices)).toBe('You chose: A.');
    expect(answeredLine('7', choices)).toBe('Answered.');
    expect(choiceIndex('choice:2')).toBe(2);
    expect(choiceIndex('allow_once')).toBeNull();
  });
});
