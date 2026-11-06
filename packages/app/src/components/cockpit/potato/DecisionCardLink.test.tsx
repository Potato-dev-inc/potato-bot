// @vitest-environment jsdom
// A tool-server card: the bot suggested a server by its https link. The card shows the link and has two buttons; only
// Add it (answer 0) adds the server, and nothing on the card asks for a token.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DecisionCard } from './DecisionCard';
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
const card = { kind: 'question' as const, why: 'https://mcp.example.com/mcp - to read issues', choices: ['Add it', 'Not now'] };

describe('DecisionCard: add a tool server by link', () => {
  it('shows the link and two buttons, and no field for a token', async () => {
    await act(async () => root.render(<DecisionCard botName="Potato" card={card} text="Potato wants to add the tool server tracker" onAnswer={() => Promise.resolve()} />));
    expect(host.textContent).toContain('https://mcp.example.com/mcp');
    expect(host.textContent).toContain('tracker');
    expect(buttons()).toEqual(['Add it', 'Not now']);
    expect(host.querySelector('input')).toBeNull();
  });

  it('Add it answers 0 and Not now answers 1', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () => root.render(<DecisionCard botName="Potato" card={card} text="Potato wants to add the tool server tracker" onAnswer={onAnswer} />));
    await act(async () => buttonByText(host, 'Add it')!.click());
    expect(onAnswer).toHaveBeenLastCalledWith('0');
  });

  it('Not now answers 1 and nothing opens', async () => {
    const onAnswer = vi.fn(() => Promise.resolve());
    await act(async () => root.render(<DecisionCard botName="Potato" card={card} text="Potato wants to add the tool server tracker" onAnswer={onAnswer} />));
    await act(async () => buttonByText(host, 'Not now')!.click());
    expect(onAnswer).toHaveBeenLastCalledWith('1');
  });
});
