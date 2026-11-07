// @vitest-environment jsdom
// The Spaces page: the empty state, the rail, the offline line, the owner's invite button, and the request sheet.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceDetail, SpaceInvite, SpaceJoinRequest, SpaceMember, SpaceMessage, SpacesView } from '../../lib/spaces';

const state = vi.hoisted(() => ({
  view: null as SpacesView | null,
  detail: null as SpaceDetail | null,
  invite: null as unknown,
  answer: vi.fn(),
  create: vi.fn(async () => 's9'),
  messages: [] as unknown[],
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => null as unknown),
  pick: vi.fn(async (_opts?: unknown) => null as unknown),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: state.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: state.pick }));

vi.mock('../../lib/spaces', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../lib/spaces')>();
  return {
    ...orig,
    useSpaces: () => ({
      view: state.view,
      error: null,
      create: state.create,
      invite: async () => state.invite,
      cancelInvite: async () => {},
      join: async () => true,
      answer: state.answer,
      cancelJoin: async () => {},
    }),
    useSpace: () => ({ detail: state.detail, error: null, reload: () => {} }),
    useChannel: () => ({ messages: state.messages, atStart: true, loadOlder: async () => {}, send: async () => null, error: null }),
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

import { SpacesPage } from './SpacesPage';

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.view = null;
  state.detail = null;
  state.invite = null;
  state.messages = [];
  state.pick.mockReset();
  state.pick.mockImplementation(async () => null);
  state.invoke.mockReset();
  state.invoke.mockImplementation(async () => null);
  state.answer.mockReset();
  state.create.mockReset();
  state.create.mockImplementation(async () => 's9');
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const summary = (over: Partial<SpacesView['spaces'][number]> = {}) => ({
  id: 's1',
  name: 'Book club',
  isOwner: true,
  unread: 0,
  members: 3,
  online: 0,
  ...over,
});
const view = (over: Partial<SpacesView> = {}): SpacesView => ({
  spaces: [],
  requests: [],
  joining: [],
  invites: [],
  online: true,
  relayed: false,
  showOnline: true,
  ...over,
});
const detail = (over: Partial<SpaceDetail> = {}): SpaceDetail => ({
  id: 's1',
  name: 'Book club',
  isOwner: true,
  myMember: 'm1',
  ownerName: 'Ren',
  channels: [],
  members: [],
  openInvites: 0,
  relayed: false,
  anyoneOnline: true,
  state: 'active',
  leavePending: false,
  ...over,
});

async function render() {
  await act(async () => root.render(<SpacesPage />));
  await act(async () => {});
}
const text = () => host.textContent ?? '';
const button = (label: string) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) ?? null;

describe('Spaces page', () => {
  it('empty state explains spaces', async () => {
    state.view = view();
    await render();
    expect(host.querySelector('h1')?.textContent).toBe('Spaces');
    expect(text()).toContain('Chat with friends in small groups, computer to computer. No server keeps your messages.');
    expect(button('New space')).not.toBeNull();
    expect(button('Join a space')).not.toBeNull();
  });

  it('rail lists spaces with unread and online', async () => {
    state.view = view({
      spaces: [summary({ id: 's1', name: 'Book club', unread: 3, online: 2 }), summary({ id: 's2', name: 'Running', online: 0 })],
    });
    await render();
    expect(text()).toContain('Book club');
    expect(host.querySelector('.ck-spaces-badge')?.textContent).toBe('3');
    expect(text()).toContain('2 online');
    expect(text()).toContain('No one online');
  });

  it('first space is selected by default', async () => {
    state.view = view({ spaces: [summary({ id: 's1', name: 'Book club' }), summary({ id: 's2', name: 'Running' })] });
    state.detail = detail();
    await render();
    expect(host.querySelector('[aria-current="page"]')?.textContent).toContain('Book club');
  });

  it('offline banner', async () => {
    state.view = view({ online: false, spaces: [summary()] });
    state.detail = detail();
    await render();
    expect(text()).toContain("You're offline. New messages will go out when you're back online.");
  });

  it('owner sees the invite button and the invite sheet opens', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ isOwner: true });
    state.invite = {
      id: 'i1',
      space: 's1',
      link: 'pms1:abc',
      words: ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'],
      qr: '',
      expiresAtMs: Date.now() + 3_600_000,
      state: 'open',
    } as SpaceInvite;
    await render();
    await act(async () => button('Invite people')!.click());
    await act(async () => {});
    expect(text()).toContain('Invite someone to Book club');
  });

  it('member does not see the invite button', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false });
    await render();
    expect(button('Invite people')).toBeNull();
  });

  it('join request sheet shows when the backend has a request', async () => {
    const request: SpaceJoinRequest = {
      id: 7,
      space: 's1',
      spaceName: 'Book club',
      name: 'Sam',
      short: 'ab12',
      receivedAtMs: 0,
      answerByMs: Date.now() + 60_000,
    };
    state.view = view({ spaces: [summary()], requests: [request] });
    state.detail = detail();
    await render();
    expect(text()).toContain('Sam wants to join Book club');
  });

  it('join sheet opens from the rail', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail();
    await render();
    await act(async () => button('Join a space')!.click());
    await act(async () => {});
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it('first channel opens with the feed', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({
      channels: [
        { id: 'c1', name: 'general', archived: false, unread: 0 },
        { id: 'c2', name: 'books', archived: false, unread: 0 },
      ],
    });
    await render();
    expect(host.querySelector('[aria-label="#general"]')).not.toBeNull();
    expect(host.querySelector('.ck-spaces-feed-title')?.textContent).toBe('# general');
  });

  it('clicking a channel switches the feed', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({
      channels: [
        { id: 'c1', name: 'general', archived: false, unread: 0 },
        { id: 'c2', name: 'books', archived: false, unread: 0 },
      ],
    });
    await render();
    const books = [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('# books')) ?? null;
    await act(async () => books!.click());
    await act(async () => {});
    expect(host.querySelector('.ck-spaces-feed-title')?.textContent).toBe('# books');
  });

  it('new space sheet creates and selects', async () => {
    state.view = view({ spaces: [summary({ id: 's1', name: 'Book club' })] });
    state.detail = detail();
    await render();
    await act(async () => button('New space')!.click());
    await act(async () => {});
    const input = host.querySelector('.ck-spaces-sheet input') as HTMLInputElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'Book club');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => button('Create')!.click());
    await act(async () => {});
    expect(state.create).toHaveBeenCalledWith('Book club');
    expect(host.querySelector('.ck-spaces-sheet')).toBeNull();
  });

  it('space info shows the honest storage note', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail();
    await render();
    await act(async () => button('Space info')!.click());
    await act(async () => {});
    expect(text()).toContain(
      "Messages live on members' computers. If everyone is offline, new messages wait. This computer keeps the last 90 days, up to 200 MB per space.",
    );
  });

  it('show online checkbox calls the command and follows the answer', async () => {
    state.view = view({ spaces: [summary()], showOnline: true });
    state.detail = detail();
    state.invoke.mockImplementation(async (cmd: string) => (cmd === 'space_show_online' ? view({ spaces: [summary()], showOnline: false }) : null));
    await render();
    await act(async () => button('Space info')!.click());
    await act(async () => {});
    const box = host.querySelector('.ck-spaces-sheet input[type="checkbox"]') as HTMLInputElement;
    expect(box.checked).toBe(true);
    expect(text()).toContain("Show when I'm online");
    await click(box);
    expect(state.invoke).toHaveBeenCalledWith('space_show_online', { on: false });
    expect((host.querySelector('.ck-spaces-sheet input[type="checkbox"]') as HTMLInputElement).checked).toBe(false);
  });

  it('the page reports that this window is online', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail();
    await render();
    expect(state.invoke).toHaveBeenCalledWith('space_presence', { state: 'online' });
  });

  // ---- D5.4: menus, leave and the removed state, all through the page ----------------------------------------

  const sam: SpaceMember = {
    member: 'mem-sam', name: 'Sam', role: 'member', presence: 'online', blocked: false, isMe: false, short: 'AB12-CD34',
  };
  const ren: SpaceMember = {
    member: 'm1', name: 'Ren', role: 'owner', presence: 'online', blocked: false, isMe: true, short: 'EF56-GH78',
  };
  const samMessage: SpaceMessage = {
    id: 'msg1', channel: 'c1', member: 'mem-sam', authorName: 'Sam', isFish: false, isMine: false, createdAtMs: 0,
    text: 'hello there', replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false,
  };
  const general = [{ id: 'c1', name: 'general', archived: false, unread: 0 }];
  const byLabel = (label: string) =>
    [...host.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === label) ?? null;
  const click = async (el: Element | null) => {
    await act(async () => (el as HTMLElement).click());
    await act(async () => {});
  };
  const menuItems = () => [...host.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent);

  it('owner leave button is disabled with reason', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ isOwner: true });
    await render();
    const leave = button('Leave space')!;
    expect(leave.disabled).toBe(true);
    expect(leave.title).toBe("You own this space. Owners can't leave in this version.");
  });

  it('member leaves after confirm', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false });
    await render();
    await click(button('Leave space'));
    expect(text()).toContain('Leave Book club?');
    expect(text()).toContain('You will stop getting new messages. Your copy stays on this computer until you delete it.');
    expect(state.invoke).not.toHaveBeenCalledWith('space_leave', expect.anything());
    await click(button('Leave'));
    expect(state.invoke).toHaveBeenCalledWith('space_leave', { space: 's1' });
  });

  it('removed banner hides composer and offers delete', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false, state: 'removed', channels: general });
    await render();
    expect(text()).toContain('You were removed from Book club. You can still read what you have here.');
    expect(host.querySelector('textarea')).toBeNull();
    expect(button('Leave space')).toBeNull();
    await click(button('Delete my copy'));
    expect(text()).toContain("This removes its messages and files from this computer. It can't be undone.");
    await click(document.querySelectorAll('.ck-spaces-sheet button')[1]);
    expect(state.invoke).toHaveBeenCalledWith('space_forget', { space: 's1' });
  });

  it('left banner says you left', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false, state: 'left' });
    await render();
    expect(text()).toContain('You left Book club.');
    expect(button('Delete my copy')).not.toBeNull();
  });

  it('message menu opens on the page and deletes after the sheet', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ isOwner: true, channels: general, members: [ren, sam] });
    state.messages = [samMessage];
    await render();
    await click(byLabel('Message actions'));
    expect(menuItems()).toEqual(['Reply', 'React', 'Delete']);
    await click(button('React'));
    await click(byLabel('Heart'));
    expect(state.invoke).toHaveBeenCalledWith('space_react', { space: 's1', target: 'msg1', code: 'heart', on: true });
    await click(byLabel('Message actions'));
    await click(button('Delete'));
    expect(text()).toContain('Delete this message?');
    await click(document.querySelectorAll('.ck-spaces-sheet button')[1]);
    expect(state.invoke).toHaveBeenCalledWith('space_delete', { space: 's1', target: 'msg1' });
  });

  it('a member sees Delete only on their own messages', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false, channels: general, members: [sam] });
    state.messages = [samMessage, { ...samMessage, id: 'msg2', isMine: true }];
    await render();
    const menus = [...host.querySelectorAll('[aria-label="Message actions"]')] as HTMLElement[];
    expect(menus).toHaveLength(2);
    await click(menus[0]);
    expect(menuItems()).toEqual(['Reply', 'React']);
    await click(menus[0]);
    await click(menus[1]);
    expect(menuItems()).toEqual(['Reply', 'React', 'Delete']);
  });

  it('member menu opens on the page: owner removes, anyone blocks', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ isOwner: true, channels: general, members: [ren, sam] });
    await render();
    expect(byLabel('Actions for Ren')).toBeNull();
    await click(byLabel('Actions for Sam'));
    expect(menuItems()).toEqual(['Remove from space', 'Block']);
    await click(button('Remove from space'));
    expect(text()).toContain('Remove Sam from Book club?');
    await click(button('Remove'));
    expect(state.invoke).toHaveBeenCalledWith('space_remove_member', { space: 's1', member: 'mem-sam' });
    await click(byLabel('Actions for Sam'));
    await click(button('Block'));
    await click(document.querySelectorAll('.ck-spaces-sheet button')[1]);
    expect(state.invoke).toHaveBeenCalledWith('space_block', { space: 's1', member: 'mem-sam', on: true });
  });

  it('a member is not offered Remove', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false, channels: general, members: [{ ...ren, isMe: false }, sam] });
    await render();
    await click(byLabel('Actions for Sam'));
    expect(menuItems()).toEqual(['Block']);
  });

  it('a failed action shows its plain sentence', async () => {
    state.view = view({ spaces: [summary({ isOwner: false })] });
    state.detail = detail({ isOwner: false });
    state.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'space_leave') throw 'You are not in this space any more.';
      return null;
    });
    await render();
    await click(button('Leave space'));
    await click(button('Leave'));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('You are not in this space any more.');
  });

  // ---- D8.2: files, through the page ---------------------------------------------------------------------------

  const fileMessage = (file: Partial<NonNullable<SpaceMessage['file']>> = {}): SpaceMessage => ({
    ...samMessage, id: 'msg-f', text: 'notes.txt',
    file: { sha: 'ab12', name: 'notes.txt', size: 2048, have: false, state: 'idle', percent: 0, ...file },
  });

  it('a file message shows a card with Get file, and the click asks the backend', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ channels: general, members: [ren, sam] });
    state.messages = [fileMessage()];
    await render();
    expect(host.querySelector('.ck-spaces-file')?.textContent).toContain('notes.txt · 2 KB');
    expect(text()).toContain('Files from other people can be harmful. Only open files you expect.');
    await click(button('Get file'));
    expect(state.invoke).toHaveBeenCalledWith('space_file_get', { space: 's1', sha: 'ab12' });
  });

  it('a failed Get file shows the plain sentence on the card', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ channels: general, members: [ren, sam] });
    state.messages = [fileMessage()];
    state.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'space_file_get') throw 'No one online has this file right now. Try again later.';
      return null;
    });
    await render();
    await click(button('Get file'));
    expect(host.querySelector('.ck-spaces-file [role="alert"]')?.textContent).toBe(
      'No one online has this file right now. Try again later.',
    );
  });

  it('a file that is here saves to Downloads and says where', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ channels: general, members: [ren, sam] });
    state.messages = [fileMessage({ have: true })];
    state.invoke.mockImplementation(async (cmd: string) => (cmd === 'space_file_save' ? { name: 'notes (1).txt' } : null));
    await render();
    await click(button('Save to Downloads'));
    expect(state.invoke).toHaveBeenCalledWith('space_file_save', { space: 's1', sha: 'ab12' });
    expect(text()).toContain('Saved to Downloads as notes (1).txt.');
  });

  it('Attach file picks a file and offers it to the channel', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ channels: general, members: [ren, sam] });
    state.pick.mockImplementation(async () => 'C:\Users\ren\notes.txt');
    await render();
    await click(button('Attach file'));
    expect(state.pick).toHaveBeenCalledWith({ multiple: false, directory: false });
    expect(state.invoke).toHaveBeenCalledWith('space_file_offer', { space: 's1', channel: 'c1', path: 'C:\Users\ren\notes.txt' });
  });

  it('a file that is too big shows the limit under the composer', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ channels: general, members: [ren, sam] });
    state.pick.mockImplementation(async () => '/big.bin');
    state.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'space_file_offer') throw 'Files can be up to 25 MB.';
      return null;
    });
    await render();
    await click(button('Attach file'));
    expect(host.querySelector('.ck-spaces-composer [role="alert"]')?.textContent).toBe('Files can be up to 25 MB.');
  });

  it('a space I left has no Attach file button', async () => {
    state.view = view({ spaces: [summary()] });
    state.detail = detail({ channels: general, members: [ren, sam], state: 'left' });
    await render();
    expect(button('Attach file')).toBeNull();
  });
});
