// @vitest-environment jsdom
// The people column: order, presence labels, the (you) mark, blocked, the count, and the New space button rule.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { SpaceMember } from '../../lib/spaces';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { MemberList, sortMembers } from './MemberList';
import { NewSpaceSheet } from './NewSpaceSheet';

const m = (over: Partial<SpaceMember> & { name: string }): SpaceMember => ({
  member: over.name.toLowerCase(),
  role: 'member',
  presence: 'offline',
  blocked: false,
  isMe: false,
  short: 'ab12',
  ...over,
});

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

describe('MemberList', () => {
  it('owner first then online then name', () => {
    const sorted = sortMembers([
      m({ name: 'Bo', presence: 'offline' }),
      m({ name: 'Ren', role: 'owner', presence: 'offline' }),
      m({ name: 'Ana', presence: 'online' }),
    ]);
    expect(sorted.map((x) => x.name)).toEqual(['Ren', 'Ana', 'Bo']);
  });

  it('presence dot labels', () => {
    act(() =>
      root.render(
        <MemberList
          members={[
            m({ name: 'On', presence: 'online' }),
            m({ name: 'Away', presence: 'away' }),
            m({ name: 'Off', presence: 'offline' }),
          ]}
        />,
      ),
    );
    expect(host.querySelector('[aria-label="Online"]')?.className).toBe('ck-spaces-dot is-online');
    expect(host.querySelector('[aria-label="Away"]')?.className).toBe('ck-spaces-dot is-away');
    expect(host.querySelector('[aria-label="Offline"]')?.className).toBe('ck-spaces-dot is-offline');
  });

  it('me is marked', () => {
    act(() => root.render(<MemberList members={[m({ name: 'Ren', isMe: true })]} />));
    expect(host.textContent).toContain('Ren (you)');
  });

  it('blocked member is marked', () => {
    act(() => root.render(<MemberList members={[m({ name: 'Sam', blocked: true })]} />));
    expect(host.textContent).toContain('Blocked');
    expect(host.querySelector('.ck-spaces-member')?.className).toContain('is-blocked');
  });

  it('count in heading', () => {
    act(() =>
      root.render(
        <MemberList members={[m({ name: 'A' }), m({ name: 'B' }), m({ name: 'C' }), m({ name: 'D' })]} />,
      ),
    );
    expect(host.querySelector('.ck-spaces-feed-title')?.textContent).toBe('People (4)');
  });

  it('new space create disabled when empty', () => {
    act(() =>
      root.render(
        <NewSpaceSheet error={null} onCreate={async () => 's1'} onClose={() => {}} />,
      ),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, '   ');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const create = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Create');
    expect(create?.disabled).toBe(true);
  });
});
