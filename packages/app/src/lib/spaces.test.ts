// @vitest-environment jsdom
import { act, createElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  joinStateText,
  MAX_TEXT_CHARS,
  mergeMessages,
  REACTION_LABEL,
  spaceBlock,
  spaceDelete,
  spaceForget,
  spaceLeave,
  spacePresence,
  spaceReact,
  spaceRemoveMember,
  spacesEnabled,
  spacesErrorText,
  AWAY_AFTER_MS,
  useChannel,
  usePresenceReporter,
  useSpaces,
  type JoinState,
  type SpaceMessage,
  type SpacesApi,
  type SpacesView,
} from './spaces';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const VIEW: SpacesView = { spaces: [], requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true };

let host: HTMLDivElement;
let root: Root | undefined;
let api: SpacesApi | null;

function Probe({ enabled }: { enabled: boolean }) {
  api = useSpaces(enabled);
  return null;
}

async function mount(enabled: boolean): Promise<void> {
  host = document.createElement('div');
  const r = createRoot(host);
  root = r;
  await act(async () => {
    r.render(createElement(Probe, { enabled }));
  });
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockClear();
  api = null;
});

afterEach(async () => {
  const r = root;
  root = undefined;
  if (r) await act(async () => r.unmount());
});

describe('Spaces helpers', () => {
  it('spacesEnabled is true only for true', () => {
    expect(spacesEnabled({ spaces_enabled: true })).toBe(true);
    expect(spacesEnabled({})).toBe(false);
    expect(spacesEnabled(null)).toBe(false);
  });

  it('spacesErrorText passes command sentences through', () => {
    expect(spacesErrorText('Give the space a name.')).toBe('Give the space a name.');
  });

  it('spacesErrorText falls back', () => {
    expect(spacesErrorText({})).toBe('Something went wrong. Try again.');
  });

  it('joinStateText wrongWords singular', () => {
    expect(joinStateText({ space: 's1', spaceName: 'Book club', state: 'wrongWords', triesLeft: 1, answerByMs: null }))
      .toBe("Those words don't match. 1 try left.");
  });

  it('joinStateText covers every state', () => {
    const states: JoinState[] = [
      'connecting', 'waiting', 'joined', 'wrongWords', 'gone', 'no',
      'lockedOut', 'busy', 'full', 'noAnswer', 'unreachable', 'bad',
    ];
    for (const state of states) {
      const text = joinStateText({ space: 's1', spaceName: 'Book club', state, triesLeft: 3, answerByMs: null });
      expect(text.length).toBeGreaterThan(0);
      expect(text).not.toContain('undefined');
    }
  });
});

describe('useSpaces', () => {
  it('does nothing when disabled', async () => {
    await mount(false);
    expect(invoke).not.toHaveBeenCalled();
    expect(listen).not.toHaveBeenCalled();
    expect(api?.view).toBeNull();
  });

  it('loads the view and listens', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(VIEW);
    await mount(true);
    expect(invoke).toHaveBeenCalledWith('spaces_view');
    expect(listen).toHaveBeenCalledWith('spaces://changed', expect.any(Function));
    expect(api?.view).toEqual(VIEW);
  });

  it('join returns false and sets error on failure', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(VIEW);
    await mount(true);
    vi.mocked(invoke).mockRejectedValueOnce("That doesn't look like a Spaces invite link.");
    let ok = true;
    await act(async () => {
      ok = await api!.join('hello', 'a b c d e f g h');
    });
    expect(ok).toBe(false);
    expect(api?.error).toBe("That doesn't look like a Spaces invite link.");
  });
});

function msg(id: string, text = 'hi'): SpaceMessage {
  return {
    id, channel: 'c1', member: 'm1', authorName: 'Sam', isFish: false, isMine: false, createdAtMs: 0, text,
    replyTo: null, replyPreview: null, deleted: null, reactions: [], file: null, blocked: false,
  };
}

let chan: ReturnType<typeof useChannel> | null = null;

function ChannelProbe({ space, channel }: { space: string | null; channel: string | null }) {
  chan = useChannel(space, channel);
  return null;
}

async function mountChannel(space: string | null, channel: string | null): Promise<void> {
  host = document.createElement('div');
  const r = createRoot(host);
  root = r;
  await act(async () => {
    r.render(createElement(ChannelProbe, { space, channel }));
  });
}

describe('Spaces chat', () => {
  it('mergeMessages appends new ids only', () => {
    const merged = mergeMessages([msg('a'), msg('b')], [msg('b'), msg('c')]);
    expect(merged.map((m) => m.id)).toEqual(['a', 'b', 'c']);
  });

  it('mergeMessages older page goes first', () => {
    const merged = mergeMessages([msg('x'), msg('y')], [msg('a'), msg('b')]);
    expect(merged.map((m) => m.id)).toEqual(['x', 'y', 'a', 'b']);
  });

  it('mergeMessages replaces changed rows', () => {
    const merged = mergeMessages([msg('a', 'text 1')], [msg('a', 'text 2')]);
    expect(merged).toEqual([msg('a', 'text 2')]);
  });

  it('useChannel loads newest 50 and marks read', async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await mountChannel('s1', 'c1');
    expect(invoke).toHaveBeenNthCalledWith(1, 'space_messages', { space: 's1', channel: 'c1', before: null, limit: 50 });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('space_read', { space: 's1', channel: 'c1' }));
    expect(chan?.messages).toEqual([]);
  });

  it('useChannel refetches on message event for its channel only', async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    const sink: { fn?: (e: { payload: unknown }) => void } = {};
    vi.mocked(listen).mockImplementationOnce((_name, handler) => {
      sink.fn = handler as unknown as (e: { payload: unknown }) => void;
      return Promise.resolve(() => {});
    });
    await mountChannel('s1', 'c1');
    const fetches = () => vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'space_messages').length;
    expect(fetches()).toBe(1);
    await act(async () => {
      sink.fn?.({ payload: { space: 's1', channel: 'c2' } });
    });
    expect(fetches()).toBe(1);
    await act(async () => {
      sink.fn?.({ payload: { space: 's1', channel: 'c1' } });
    });
    expect(fetches()).toBe(2);
  });
});

describe('limits shared with Rust', () => {
  it('MAX_TEXT_CHARS equals the Rust MAX_TEXT_CHARS in spaces/limits.rs', async () => {
    const { default: rs } = await import('../../src-tauri/src/spaces/limits.rs?raw');
    const m = /pub const MAX_TEXT_CHARS: usize = (\d+);/.exec(rs);
    expect(m).not.toBeNull();
    expect(MAX_TEXT_CHARS).toBe(Number(m![1]));
    expect(MAX_TEXT_CHARS).toBe(4000);
  });
});

function Reporter({ enabled }: { enabled: boolean }) {
  usePresenceReporter(enabled);
  return null;
}

describe('usePresenceReporter', () => {
  const calls = () => vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'space_presence').map((c) => (c[1] as { state: string }).state);
  const mount = (enabled = true) => {
    host = document.createElement('div');
    root = createRoot(host);
    act(() => root!.render(createElement(Reporter, { enabled })));
  };
  const press = () => act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' })); });

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(null);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('wrapper calls the command', async () => {
    await spacePresence('away');
    expect(vi.mocked(invoke).mock.calls).toEqual([['space_presence', { state: 'away' }]]);
  });

  it('reporter sends online at start', () => {
    mount();
    expect(calls()).toEqual(['online']);
  });

  it('reporter sends nothing when disabled', () => {
    mount(false);
    expect(calls()).toEqual([]);
  });

  it('reporter goes away after ten minutes', () => {
    mount();
    act(() => { vi.advanceTimersByTime(AWAY_AFTER_MS - 1); });
    expect(calls()).toEqual(['online']);
    act(() => { vi.advanceTimersByTime(1); });
    expect(calls()).toEqual(['online', 'away']);
  });

  it('activity brings it back online', () => {
    mount();
    act(() => { vi.advanceTimersByTime(AWAY_AFTER_MS); });
    press();
    expect(calls()).toEqual(['online', 'away', 'online']);
  });

  it('reporter does not spam', () => {
    mount();
    for (let i = 0; i < 20; i++) press();
    expect(calls()).toEqual(['online']);
  });

  it('stops listening when it unmounts', () => {
    mount();
    act(() => root!.unmount());
    root = undefined;
    vi.mocked(invoke).mockClear();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    act(() => { vi.advanceTimersByTime(AWAY_AFTER_MS * 2); });
    expect(calls()).toEqual([]);
  });
});

describe('moderation wrappers', () => {
  it('wrappers call the right commands', async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    await spaceRemoveMember('s', 'm');
    await spaceLeave('s');
    await spaceForget('s');
    await spaceBlock('s', 'm', true);
    await spaceReact('s', 't', 'heart', true);
    await spaceDelete('s', 't');
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ['space_remove_member', { space: 's', member: 'm' }],
      ['space_leave', { space: 's' }],
      ['space_forget', { space: 's' }],
      ['space_block', { space: 's', member: 'm', on: true }],
      ['space_react', { space: 's', target: 't', code: 'heart', on: true }],
      ['space_delete', { space: 's', target: 't' }],
    ]);
  });

  it('reaction labels cover every code', () => {
    expect(Object.keys(REACTION_LABEL).sort()).toEqual(
      ['check', 'eyes', 'heart', 'laugh', 'party', 'sad', 'thumbs_up', 'wow'],
    );
    expect(REACTION_LABEL.thumbs_up).toBe('Thumbs up');
  });
});
