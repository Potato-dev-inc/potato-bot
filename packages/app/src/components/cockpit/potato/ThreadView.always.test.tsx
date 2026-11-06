// @vitest-environment jsdom
// "Always allow" on approval cards, in the real thread: the button shows only for a card the engine marked as allowed
// (never rm or force-push, never a question), it goes through the app's own call and not the plain answer call, a failed
// answer shows its sentence, and an answered card shows a quiet line.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport, type PotatoSettings, type PotatoTransport } from '../../../lib/potato';
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
const push = msg({ id: 'card-1', text: 'git push origin dev', kind: 'decision_card', requestId: 'c1', card: { kind: 'approval', why: 'This command pushes to a remote.', choices: [], canAlwaysAllow: true } });
const force = msg({ id: 'card-2', text: 'git push --force origin main', kind: 'decision_card', requestId: 'c2', card: { kind: 'approval', why: 'This command force-pushes to a remote.', choices: [] } });
const rm = msg({ id: 'card-3', text: 'rm -rf /etc/stuff', kind: 'decision_card', requestId: 'c3', card: { kind: 'approval', why: 'This command deletes files outside the work folder.', choices: [] } });
const question = msg({ id: 'card-4', text: 'Which port?', kind: 'decision_card', requestId: 'c4', card: { kind: 'question', why: '', choices: ['3000', '8080'], canAlwaysAllow: true } });

function props(transport: PotatoTransport, messages = [push]): ThreadViewProps {
  return { bot: bot({ id: 'fish', talkPersona: 'pro' }), messages, settings: SETTINGS, avatar: 'mochi', agentLabel: 'Claude', transport, onOpenSettings: () => {}, onNotify: () => {}, onChanged: () => {}, now: NOW };
}
const flush = () => act(async () => {});

describe('ThreadView: Always allow', () => {
  it('shows the button only on a card the engine marked as allowed', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })] });
    await act(async () => root.render(<ThreadView {...props(fake, [push])} />));
    expect(buttonByText(host, 'Always allow')).toBeDefined();
    await act(async () => root.render(<ThreadView {...props(fake, [force, rm])} />));
    expect(buttonByText(host, 'Always allow')).toBeUndefined();
    expect(buttonByText(host, 'Allow once')).toBeDefined();
    expect(host.textContent).toContain('git push --force origin main');
    // a question never shows it, whatever its flag says
    await act(async () => root.render(<ThreadView {...props(fake, [question])} />));
    expect(buttonByText(host, 'Always allow')).toBeUndefined();
    expect(buttonByText(host, '3000')).toBeDefined();
  });

  it('the click goes through the app call, not the plain answer, and the card closes with a quiet line', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [push] } });
    const answerCard = vi.fn(fake.answerCard);
    const answerCardAlways = vi.fn(fake.answerCardAlways);
    const onChanged = vi.fn();
    await act(async () => root.render(<ThreadView {...{ ...props({ ...fake, answerCard, answerCardAlways }), onChanged }} />));
    await act(async () => buttonByText(host, 'Always allow')!.click());
    await flush();
    expect(answerCardAlways).toHaveBeenCalledWith('fish', 'c1');
    expect(answerCard).not.toHaveBeenCalled();
    expect(onChanged).toHaveBeenCalled();
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('Always allowed for this kind of command.');
    expect(buttonByText(host, 'Always allow')).toBeUndefined();
  });

  it('a refused answer shows its sentence and leaves the buttons on', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })] });
    const transport = { ...fake, answerCardAlways: () => Promise.reject(new Error('POTATO_CARD_BAD_ANSWER: Always allow is not offered for this kind of command. Nothing was answered.')) };
    await act(async () => root.render(<ThreadView {...props(transport)} />));
    await act(async () => buttonByText(host, 'Always allow')!.click());
    await flush();
    expect(host.querySelector('[role="status"]')?.textContent).toBe('Always allow is not offered for this kind of command. Nothing was answered.');
    expect(buttonByText(host, 'Always allow')).toBeDefined();
  });

  it('a card answered earlier shows the quiet line after a reload', async () => {
    const result = msg({ id: 'res-1', text: 'x', from: { kind: 'note' }, kind: 'decision_result', requestId: 'c1', card: { kind: 'approval', why: '', choices: [], answer: 'always_allow' } });
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })] });
    await act(async () => root.render(<ThreadView {...props(fake, [push, result])} />));
    expect(buttonByText(host, 'Always allow')).toBeUndefined();
    expect(host.querySelector('.ck-potato-decision-result')?.textContent).toBe('Always allowed for this kind of command.');
  });

  it('the plain answer call refuses the same words in the fake, like the engine', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [push] } });
    await expect(fake.answerCard('fish', 'c1', 'always_allow')).rejects.toThrow(/not one of this card's answers/);
  });
});
