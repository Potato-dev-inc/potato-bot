// @vitest-environment jsdom
import { act, createElement, type ReactElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceInvite } from '../../lib/spaces';
import { InviteSheet } from './InviteSheet';

vi.mock('qrcode', () => ({
  default: { toDataURL: vi.fn(() => Promise.resolve('data:image/png;base64,QRTEST')) },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_000_000;
const WORDS = ['able', 'bench', 'cloud', 'delta', 'ember', 'fable', 'garden', 'honey'];

function invite(over: Partial<SpaceInvite> = {}): SpaceInvite {
  return {
    id: 'i1',
    space: 's1',
    link: 'pms1:abc123',
    words: WORDS,
    qr: 'pms1:abc123#able-bench-cloud-delta-ember-fable-garden-honey',
    expiresAtMs: NOW + 2 * 60 * 60 * 1000,
    state: 'open',
    ...over,
  };
}

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

beforeEach(() => {
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: vi.fn(() => Promise.resolve()) },
    configurable: true,
  });
});

afterEach(() => {
  const r = root;
  root = undefined;
  if (r) act(() => r.unmount());
  host?.remove();
  host = undefined;
});

describe('InviteSheet', () => {
  it('shows link, 8 numbered words and the rules', () => {
    const el = render(createElement(InviteSheet, {
      spaceName: 'Book club', invite: invite(), nowMs: NOW, onCancelInvite: () => {}, onClose: () => {},
    }));
    expect(el.querySelector('h2')?.textContent).toBe('Invite someone to Book club');
    expect(el.querySelectorAll('li')).toHaveLength(8);
    expect(el.textContent).toContain('Works once. Expires in');
    expect(el.textContent).toContain('After 5 wrong tries it stops working.');
  });

  it('copy link writes only the link', async () => {
    const el = render(createElement(InviteSheet, {
      spaceName: 'Book club', invite: invite(), nowMs: NOW, onCancelInvite: () => {}, onClose: () => {},
    }));
    await act(async () => {
      button('Copy link').click();
    });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('pms1:abc123');
    expect(button('Copied')).toBeTruthy();
    expect(el.textContent).not.toContain('#');
  });

  it('QR button shows the QR image', async () => {
    render(createElement(InviteSheet, {
      spaceName: 'Book club', invite: invite(), nowMs: NOW, onCancelInvite: () => {}, onClose: () => {},
    }));
    await act(async () => {
      button('Show QR code').click();
    });
    await act(async () => {});
    const img = host?.querySelector('img');
    expect(img?.getAttribute('src')).toBe('data:image/png;base64,QRTEST');
    expect(img?.getAttribute('alt')).toBe('Invite QR code');
  });

  it('cancel invite calls back', () => {
    const onCancelInvite = vi.fn();
    render(createElement(InviteSheet, {
      spaceName: 'Book club', invite: invite(), nowMs: NOW, onCancelInvite, onClose: () => {},
    }));
    act(() => {
      button('Cancel invite').click();
    });
    expect(onCancelInvite).toHaveBeenCalledTimes(1);
  });

  it('dead invite after strikes explains why', () => {
    const el = render(createElement(InviteSheet, {
      spaceName: 'Book club', invite: invite({ state: 'dead' }), nowMs: NOW, onCancelInvite: () => {}, onClose: () => {},
    }));
    expect(el.textContent).toContain('This invite stopped working after 5 wrong tries.');
  });
});
