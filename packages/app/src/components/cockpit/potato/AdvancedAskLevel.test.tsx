// @vitest-environment jsdom
// "When to ask": three levels, one plain line each, the saved one checked, a pick saves that level.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ASK_LEVELS, ASK_LEVEL_COPY, AdvancedAskLevel, type AskLevel } from './AdvancedAskLevel';

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

const radios = () => [...host.querySelectorAll<HTMLInputElement>('input[type="radio"]')];

describe('AdvancedAskLevel', () => {
  it('offers Ask first, Ask when risky and Full, each with one plain line', () => {
    act(() => root.render(<AdvancedAskLevel value="ask_when_risky" onChange={() => {}} />));
    expect(radios().map((r) => r.value)).toEqual(['ask_first', 'ask_when_risky', 'full']);
    const labels = [...host.querySelectorAll('.ck-potato-radio')].map((l) => l.querySelector('.ck-potato-adv-switch-label')?.textContent);
    expect(labels).toEqual(['Ask first', 'Ask when risky', 'Full']);
    for (const level of ASK_LEVELS) expect(level.line.length).toBeGreaterThan(20);
    expect(host.textContent).toContain(ASK_LEVEL_COPY.title);
  });

  it('checks the saved level', () => {
    act(() => root.render(<AdvancedAskLevel value="full" onChange={() => {}} />));
    expect(radios().map((r) => r.checked)).toEqual([false, false, true]);
  });

  it('a pick saves that level', () => {
    const onChange = vi.fn<(level: AskLevel) => void>();
    act(() => root.render(<AdvancedAskLevel value="ask_when_risky" onChange={onChange} />));
    act(() => radios()[0].click());
    expect(onChange).toHaveBeenCalledWith('ask_first');
  });

  it('is disabled while a save is on its way', () => {
    act(() => root.render(<AdvancedAskLevel value="ask_when_risky" onChange={() => {}} disabled />));
    expect(host.querySelector('fieldset')?.disabled).toBe(true);
  });

  it('says passwords and logins stay with the person at every level', () => {
    act(() => root.render(<AdvancedAskLevel value="full" onChange={() => {}} />));
    expect(host.textContent).toContain('Passwords and logins are always yours to type');
  });
});
