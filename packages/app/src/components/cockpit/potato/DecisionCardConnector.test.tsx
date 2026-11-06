// @vitest-environment jsdom
// A connector card: the bot wants a service it is not connected to. The first button opens Settings > Connectors and
// answers the card; the other only answers. The card never asks for a token.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DecisionCard } from './DecisionCard';
import { buttonByText } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
let opened: string[];
const listen = (e: Event) => opened.push((e as CustomEvent<{ tab: string }>).detail.tab);
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  opened = [];
  window.addEventListener('pm:open-settings', listen);
});
afterEach(() => {
  window.removeEventListener('pm:open-settings', listen);
  act(() => root.unmount());
  host.remove();
});

const buttons = () => [...host.querySelectorAll('button')].map((b) => b.textContent?.trim());
const card = { kind: 'question' as const, why: 'To read your issues.', choices: ['Open Connectors', 'Not now'], connector: 'GitHub' };

describe('DecisionCard: connector', () => {
  it('the first button opens Settings > Connectors and answers', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () => root.render(<DecisionCard botName="Potato" card={card} text="Potato needs GitHub" onAnswer={onAnswer} />));
    expect(buttons()).toEqual(['Open Connectors', 'Not now']);
    expect(host.textContent).not.toMatch(/token|password|key/i);
    await act(async () => buttonByText(host, 'Open Connectors')!.click());
    expect(opened).toContain('potato-connectors');
    expect(onAnswer).toHaveBeenLastCalledWith('0');
  });

  it('Not now does not open Settings', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () => root.render(<DecisionCard botName="Potato" card={card} text="Potato needs GitHub" onAnswer={onAnswer} />));
    await act(async () => buttonByText(host, 'Not now')!.click());
    expect(opened).toEqual([]);
    expect(onAnswer).toHaveBeenLastCalledWith('1');
  });
});
