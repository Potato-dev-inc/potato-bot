// @vitest-environment jsdom
import { act, createElement, type ReactElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SpaceJoinRequest } from '../../lib/spaces';
import { JoinRequestSheet } from './JoinRequestSheet';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const REQUEST: SpaceJoinRequest = {
  id: 7,
  space: 's1',
  spaceName: 'Book club',
  name: 'Sam',
  short: '4F7Q-9K2M',
  receivedAtMs: 0,
  answerByMs: 1_800_000,
};

let host: HTMLDivElement | undefined;
let root: Root | undefined;

function render(el: ReactElement): HTMLDivElement {
  host = document.createElement('div');
  document.body.appendChild(host);
  const r = createRoot(host);
  root = r;
  act(() => {
    r.render(el);
  });
  return host;
}

function button(label: string): HTMLButtonElement {
  const found = [...(host?.querySelectorAll('button') ?? [])].find((b) => b.textContent === label);
  if (!found) throw new Error(`no button ${label}`);
  return found;
}

afterEach(() => {
  const r = root;
  root = undefined;
  if (r) act(() => r.unmount());
  host?.remove();
  host = undefined;
});

describe('JoinRequestSheet', () => {
  it('renders nothing without a request', () => {
    const el = render(createElement(JoinRequestSheet, { request: null, onAnswer: () => {} }));
    expect(el.innerHTML).toBe('');
  });

  it('shows who and the short id', () => {
    const el = render(createElement(JoinRequestSheet, { request: REQUEST, onAnswer: () => {} }));
    expect(el.querySelector('h2')?.textContent).toBe('Sam wants to join Book club');
    expect(el.textContent).toContain('Their computer: 4F7Q-9K2M');
  });

  it('Let in answers true', () => {
    const onAnswer = vi.fn();
    render(createElement(JoinRequestSheet, { request: REQUEST, onAnswer }));
    act(() => {
      button('Let in').click();
    });
    expect(onAnswer).toHaveBeenCalledWith('s1', 7, true);
  });

  it('No answers false', () => {
    const onAnswer = vi.fn();
    render(createElement(JoinRequestSheet, { request: REQUEST, onAnswer }));
    act(() => {
      button('No').click();
    });
    expect(onAnswer).toHaveBeenCalledWith('s1', 7, false);
  });

  it('Escape answers false', () => {
    const onAnswer = vi.fn();
    render(createElement(JoinRequestSheet, { request: REQUEST, onAnswer }));
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(onAnswer).toHaveBeenCalledWith('s1', 7, false);
  });
});
