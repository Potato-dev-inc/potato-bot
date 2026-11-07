// @vitest-environment jsdom
// The message menu and the member menu: exact items, confirm sheets with the honest words, nothing else.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceMember, SpaceMessage } from '../../lib/spaces';
import { MemberMenu } from './MemberMenu';
import { MessageMenu } from './MessageMenu';

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

const message: SpaceMessage = {
  id: 'm1', channel: 'c1', member: 'mem-sam', authorName: 'Sam', isFish: false, isMine: true,
  createdAtMs: 0, text: 'hi', replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false,
};
const sam: SpaceMember = {
  member: 'mem-sam', name: 'Sam', role: 'member', presence: 'online', blocked: false, isMe: false, short: 'AB12-CD34',
};

const text = () => document.body.textContent ?? '';
const byLabel = (label: string) => [...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === label) ?? null;
const byText = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) ?? null;
const click = async (el: Element | null) => { await act(async () => (el as HTMLElement).click()); };
const items = () => [...document.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent?.trim());

function mountMessage(over: Partial<Parameters<typeof MessageMenu>[0]> = {}) {
  const props = {
    message, canDelete: true, onReply: vi.fn(), onReact: vi.fn(), onDelete: vi.fn(), ...over,
  };
  act(() => root.render(<MessageMenu {...props} />));
  return props;
}

function mountMember(over: Partial<Parameters<typeof MemberMenu>[0]> = {}) {
  const props = {
    member: sam, spaceName: 'Book club', canRemove: true, onRemove: vi.fn(), onBlock: vi.fn(), ...over,
  };
  act(() => root.render(<MemberMenu {...props} />));
  return props;
}

describe('message menu', () => {
  it('message menu has reply react and delete for own', async () => {
    mountMessage();
    await click(byLabel('Message actions'));
    expect(items()).toEqual(['Reply', 'React', 'Delete']);
  });

  it('no delete for others', async () => {
    mountMessage({ canDelete: false });
    await click(byLabel('Message actions'));
    expect(items()).toEqual(['Reply', 'React']);
  });

  it('delete asks first with the honest text', async () => {
    const props = mountMessage();
    await click(byLabel('Message actions'));
    await click(byText('Delete'));
    expect(text()).toContain('Delete this message?');
    expect(text()).toContain('People who already saw it may still have a copy.');
    expect(props.onDelete).not.toHaveBeenCalled();
    await click(document.querySelector('.ck-spaces-sheet .is-danger'));
    expect(props.onDelete).toHaveBeenCalledTimes(1);
    expect(document.querySelector('.ck-spaces-sheet')).toBeNull();
  });

  it('cancel in the delete sheet deletes nothing', async () => {
    const props = mountMessage();
    await click(byLabel('Message actions'));
    await click(byText('Delete'));
    await click(byText('Cancel'));
    expect(props.onDelete).not.toHaveBeenCalled();
  });

  it('react picker has 8 labelled choices', async () => {
    const props = mountMessage();
    await click(byLabel('Message actions'));
    await click(byText('React'));
    const labels = [...document.querySelectorAll('.ck-spaces-picker button')].map((b) => b.getAttribute('aria-label'));
    expect(labels).toEqual(['Thumbs up', 'Heart', 'Laugh', 'Wow', 'Sad', 'Party', 'Eyes', 'Check']);
    await click(byLabel('Heart'));
    expect(props.onReact).toHaveBeenCalledWith('heart');
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it('reply closes the menu and calls onReply', async () => {
    const props = mountMessage();
    await click(byLabel('Message actions'));
    await click(byText('Reply'));
    expect(props.onReply).toHaveBeenCalledTimes(1);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it('deleted message has no menu', () => {
    mountMessage({ message: { ...message, deleted: { byName: 'Sam', byOwner: false } } });
    expect(host.innerHTML).toBe('');
  });

  it('no actions besides reply react delete', async () => {
    mountMessage();
    await click(byLabel('Message actions'));
    await click(byText('React'));
    expect(items()).toEqual(['Reply', 'React', 'Delete']);
    expect(text()).not.toMatch(/merge|approve|run|open link/i);
  });
});

describe('member menu', () => {
  it('member menu remove confirm', async () => {
    const props = mountMember();
    await click(byLabel('Actions for Sam'));
    expect(items()).toEqual(['Remove from space', 'Block']);
    await click(byText('Remove from space'));
    expect(text()).toContain('Remove Sam from Book club?');
    expect(text()).toContain('They will stop getting new messages. Messages they already have stay on their computer.');
    expect(props.onRemove).not.toHaveBeenCalled();
    await click(byText('Remove'));
    expect(props.onRemove).toHaveBeenCalledTimes(1);
  });

  it('only the owner is offered Remove', async () => {
    mountMember({ canRemove: false });
    await click(byLabel('Actions for Sam'));
    expect(items()).toEqual(['Block']);
  });

  it('block confirm says they are not told', async () => {
    const props = mountMember();
    await click(byLabel('Actions for Sam'));
    await click(byText('Block'));
    expect(text()).toContain('Block Sam?');
    expect(text()).toContain("You won't see their messages in Book club. They are not told. Bots won't answer them on your computer.");
    await click(document.querySelector('.ck-spaces-sheet .is-danger'));
    expect(props.onBlock).toHaveBeenCalledWith(true);
  });

  it('unblock has no sheet', async () => {
    const props = mountMember({ member: { ...sam, blocked: true } });
    await click(byLabel('Actions for Sam'));
    await click(byText('Unblock'));
    expect(props.onBlock).toHaveBeenCalledWith(false);
    expect(document.querySelector('.ck-spaces-sheet')).toBeNull();
  });

  it('no menu on myself', () => {
    mountMember({ member: { ...sam, isMe: true } });
    expect(host.innerHTML).toBe('');
  });
});
