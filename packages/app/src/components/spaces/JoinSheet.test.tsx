// @vitest-environment jsdom
import { act, createElement, type ReactElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SpaceJoining } from '../../lib/spaces';
import { countWords, JoinSheet } from './JoinSheet';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

/** Types into a controlled React input or textarea. */
function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
  act(() => {
    setter?.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function input(): HTMLInputElement {
  const el = host?.querySelector('input');
  if (!el) throw new Error('no link input');
  return el;
}

function textarea(): HTMLTextAreaElement {
  const el = host?.querySelector('textarea');
  if (!el) throw new Error('no words field');
  return el;
}

function button(label: string): HTMLButtonElement {
  const found = [...(host?.querySelectorAll('button') ?? [])].find((b) => b.textContent === label);
  if (!found) throw new Error(`no button ${label}`);
  return found;
}

const WORDS8 = 'alpha bravo cedar delta ember fable garden honey';

function joining(over: Partial<SpaceJoining>): SpaceJoining {
  return { space: 's1', spaceName: 'Book club', state: 'connecting', triesLeft: 5, answerByMs: null, ...over };
}

afterEach(() => {
  const r = root;
  root = undefined;
  if (r) act(() => r.unmount());
  host?.remove();
  host = undefined;
});

describe('JoinSheet', () => {
  it('ask is disabled until link and 8 words', () => {
    const onJoin = vi.fn();
    render(createElement(JoinSheet, { joining: null, error: null, onJoin, onCancel: () => {}, onOpenSpace: () => {} }));
    expect(button('Ask to join').disabled).toBe(true);
    typeInto(input(), 'pms1:abc');
    expect(button('Ask to join').disabled).toBe(true);
    typeInto(textarea(), 'alpha bravo cedar delta ember fable garden');
    expect(button('Ask to join').disabled).toBe(true);
    expect(host?.textContent).toContain('Type all 8 words. You typed 7.');
    typeInto(textarea(), WORDS8);
    expect(button('Ask to join').disabled).toBe(false);
  });

  it('qr text fills the words', () => {
    const onJoin = vi.fn();
    render(createElement(JoinSheet, { joining: null, error: null, onJoin, onCancel: () => {}, onOpenSpace: () => {} }));
    typeInto(input(), 'pms1:abc#alpha-bravo-cedar-delta-ember-fable-garden-honey');
    expect(button('Ask to join').disabled).toBe(false);
    act(() => {
      button('Ask to join').click();
    });
    expect(onJoin).toHaveBeenCalledWith(
      'pms1:abc#alpha-bravo-cedar-delta-ember-fable-garden-honey',
      'alpha-bravo-cedar-delta-ember-fable-garden-honey',
    );
  });

  it('bad link shows the plain error', () => {
    render(createElement(JoinSheet, { joining: null, error: null, onJoin: () => {}, onCancel: () => {}, onOpenSpace: () => {} }));
    typeInto(input(), 'hello');
    expect(host?.textContent).toContain("That doesn't look like a Spaces invite link.");
  });

  it('waiting state disables fields and explains', () => {
    render(createElement(JoinSheet, {
      joining: joining({ state: 'waiting' }), error: null, onJoin: () => {}, onCancel: () => {}, onOpenSpace: () => {},
    }));
    expect(input().disabled).toBe(true);
    expect(textarea().disabled).toBe(true);
    const status = host?.querySelector('[role="status"]');
    expect(status?.textContent?.startsWith('Waiting for the owner to let you in.')).toBe(true);
    expect(status?.className).not.toContain('is-error');
  });

  it('wrong words shows tries left', () => {
    render(createElement(JoinSheet, {
      joining: joining({ state: 'wrongWords', triesLeft: 3 }), error: null, onJoin: () => {}, onCancel: () => {}, onOpenSpace: () => {},
    }));
    expect(host?.textContent).toContain("Those words don't match. 3 tries left.");
  });

  it('joined shows open space', () => {
    const onOpenSpace = vi.fn();
    render(createElement(JoinSheet, {
      joining: joining({ state: 'joined' }), error: null, onJoin: () => {}, onCancel: () => {}, onOpenSpace,
    }));
    expect(host?.querySelector('h2')?.textContent).toBe("You're in Book club.");
    act(() => {
      button('Open space').click();
    });
    expect(onOpenSpace).toHaveBeenCalledWith('s1');
  });

  it('cancel while waiting cancels that join', () => {
    const onCancel = vi.fn();
    render(createElement(JoinSheet, {
      joining: joining({ state: 'waiting' }), error: null, onJoin: () => {}, onCancel, onOpenSpace: () => {},
    }));
    act(() => {
      button('Cancel').click();
    });
    expect(onCancel).toHaveBeenCalledWith('s1');
  });

  it('countWords splits like the backend', () => {
    expect(countWords('a-b, c.d\ne  f g h')).toBe(8);
  });
});
