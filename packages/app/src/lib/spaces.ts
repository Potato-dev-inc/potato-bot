// Spaces (Labs `spaces_enabled`, off by default): small-group chat between people's own computers.
// Nothing user-visible may appear while the flag is off; screens call `useSpacesEnabled()` and render nothing.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useState } from 'react';
import { loadSettings } from './settings';
import { subscribeSettingsRefresh } from './settings-sync';

/** Longest message in characters. Must equal `MAX_TEXT_CHARS` in spaces/limits.rs (a test compares them). */
export const MAX_TEXT_CHARS = 4000;
export const SPACES_LABS_KEY = 'spaces_enabled' as const;

export function spacesEnabled(settings: { spaces_enabled?: boolean } | null | undefined): boolean {
  return settings?.spaces_enabled === true;
}

export function useSpacesEnabled(): boolean {
  const [on, setOn] = useState(false);
  useEffect(() => {
    let live = true;
    // Read again whenever the settings may have changed, so the switch in Settings shows or hides the row at once.
    const read = () =>
      void loadSettings().then(
        (settings) => {
          if (live) setOn(spacesEnabled(settings));
        },
        () => {},
      );
    read();
    const unsubscribe = subscribeSettingsRefresh(read);
    return () => {
      live = false;
      unsubscribe();
    };
  }, []);
  return on;
}

export type PresenceState = 'online' | 'away' | 'offline';
export type BotMode = 'called' | 'helps';

export interface SpaceSummary {
  id: string;
  name: string;
  isOwner: boolean;
  unread: number;
  members: number;
  online: number;
}
export interface SpaceInvite {
  id: string;
  space: string;
  link: string;
  words: string[];
  qr: string;
  expiresAtMs: number;
  state: 'open' | 'pending' | 'used' | 'dead' | 'cancelled';
}
export interface SpaceJoinRequest {
  id: number;
  space: string;
  spaceName: string;
  name: string;
  short: string;
  receivedAtMs: number;
  answerByMs: number;
}
export type JoinState =
  | 'connecting' | 'waiting' | 'joined' | 'wrongWords' | 'gone' | 'no'
  | 'lockedOut' | 'busy' | 'full' | 'noAnswer' | 'unreachable' | 'bad';
export interface SpaceJoining {
  space: string;
  spaceName: string;
  state: JoinState;
  triesLeft: number;
  answerByMs: number | null;
}
export interface SpacesView {
  spaces: SpaceSummary[];
  requests: SpaceJoinRequest[];
  joining: SpaceJoining[];
  invites: SpaceInvite[];
  online: boolean;
  relayed: boolean;
  /** "Show when I'm online" (all spaces on this computer). */
  showOnline: boolean;
  /** My name in every space. null until I pick one (create and join ask for it first). */
  displayName?: string | null;
}

export const NEEDS_DISPLAY_NAME = 'needs_display_name';
export const spacesDisplayName = () => invoke<{ displayName: string | null }>('spaces_display_name');
export const spacesSetDisplayName = (name: string) =>
  invoke<{ displayName: string; view: SpacesView }>('spaces_set_display_name', { name });

export const spacesView = () => invoke<SpacesView>('spaces_view');
export const spaceCreate = (name: string) => invoke<{ space: string; view: SpacesView }>('space_create', { name });
export const spaceInviteCreate = (space: string) => invoke<SpaceInvite>('space_invite_create', { space });
export const spaceInviteCancel = (space: string, invite: string) =>
  invoke<SpacesView>('space_invite_cancel', { space, invite });
export const spaceJoin = (text: string, words: string) => invoke<SpacesView>('space_join', { text, words });
export const spaceJoinAnswer = (space: string, request: number, allow: boolean) =>
  invoke<SpacesView>('space_join_answer', { space, request, allow });
export const spaceJoinCancel = (space: string) => invoke<SpacesView>('space_join_cancel', { space });

/** Text for an error thrown by a Spaces command. Commands already return plain sentences. */
export function spacesErrorText(e: unknown): string {
  const text = typeof e === 'string' ? e : e instanceof Error ? e.message : '';
  if (text.trim() === NEEDS_DISPLAY_NAME) return 'Pick the name people will see first.';
  if (typeof e === 'string' && e.trim() !== '') return e;
  if (e instanceof Error && e.message.trim() !== '') return e.message;
  return 'Something went wrong. Try again.';
}

/** The joiner's line for each state (used by JoinSheet). */
export function joinStateText(j: SpaceJoining): string {
  switch (j.state) {
    case 'connecting': return "Reaching the owner's computer…";
    case 'waiting': return 'Waiting for the owner to let you in. Keep Potato Bot open. This can take up to 30 minutes.';
    case 'joined': return `You're in ${j.spaceName}.`;
    case 'wrongWords': return `Those words don't match. ${j.triesLeft} ${j.triesLeft === 1 ? 'try' : 'tries'} left.`;
    case 'gone': return 'This invite no longer works. Ask for a new one.';
    case 'no': return 'The owner said no.';
    case 'lockedOut': return 'Too many wrong tries from this computer. Try again in 10 minutes.';
    case 'busy': return "The owner's computer is busy. Try again in a minute.";
    case 'full': return 'This space is full (50 people).';
    case 'noAnswer': return "The owner didn't answer in time. Ask them for a new invite.";
    case 'unreachable': return "Can't reach the owner's computer. They need to be online with Potato Bot open.";
    case 'bad': return "Something in the invite didn't check out. Ask for a new one.";
  }
}

export interface SpacesApi {
  view: SpacesView | null;
  error: string | null;
  create: (name: string) => Promise<string | null>;
  invite: (space: string) => Promise<SpaceInvite | null>;
  cancelInvite: (space: string, invite: string) => Promise<void>;
  join: (text: string, words: string) => Promise<boolean>;
  answer: (space: string, request: number, allow: boolean) => Promise<void>;
  cancelJoin: (space: string) => Promise<void>;
}

/** How long to wait before asking for the view again after it failed (the service may still be starting). */
export const VIEW_RETRY_MS = 3000;

export function useSpaces(enabled: boolean): SpacesApi {
  const [view, setView] = useState<SpacesView | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!enabled) {
      setView(null);
      return;
    }
    let stop: (() => void) | null = null;
    let alive = true;
    void listen<SpacesView>('spaces://changed', (e) => {
      if (alive) setView(e.payload);
    }).then((u) => {
      if (alive) stop = u;
      else u();
    });
    // The service starts on its own task after the Labs switch goes on, so the first call can land before it is up
    // ("can't go online"). Nothing pushes an event until a command has succeeded once, so ask again until it answers.
    let retry: ReturnType<typeof setTimeout> | null = null;
    const load = () => {
      spacesView()
        .then((v) => {
          if (!alive) return;
          setError(null);
          setView(v);
        })
        .catch((e) => {
          if (!alive) return;
          setError(spacesErrorText(e));
          retry = setTimeout(load, VIEW_RETRY_MS);
        });
    };
    load();
    return () => {
      alive = false;
      if (retry) clearTimeout(retry);
      stop?.();
    };
  }, [enabled]);
  const run = useCallback(async <T,>(f: () => Promise<T>): Promise<T | null> => {
    try {
      setError(null);
      return await f();
    } catch (e) {
      setError(spacesErrorText(e));
      return null;
    }
  }, []);
  return {
    view,
    error,
    create: async (name) => (await run(() => spaceCreate(name)))?.space ?? null,
    invite: (space) => run(() => spaceInviteCreate(space)),
    cancelInvite: async (space, invite) => {
      const v = await run(() => spaceInviteCancel(space, invite));
      if (v) setView(v);
    },
    join: async (text, words) => {
      const v = await run(() => spaceJoin(text, words));
      if (v) setView(v);
      return v !== null;
    },
    answer: async (space, request, allow) => {
      const v = await run(() => spaceJoinAnswer(space, request, allow));
      if (v) setView(v);
    },
    cancelJoin: async (space) => {
      const v = await run(() => spaceJoinCancel(space));
      if (v) setView(v);
    },
  };
}

// ---- chat: detail, messages, send, read marks, channels (D4.1) -------------------------------------------

export type ReactionCode = 'thumbs_up' | 'heart' | 'laugh' | 'wow' | 'sad' | 'party' | 'eyes' | 'check';
export interface SpaceChannel {
  id: string;
  name: string;
  archived: boolean;
  unread: number;
}
export interface SpaceMember {
  member: string;
  name: string;
  role: 'owner' | 'member';
  presence: PresenceState;
  blocked: boolean;
  isMe: boolean;
  short: string;
}
export interface SpaceDetail {
  id: string;
  name: string;
  isOwner: boolean;
  myMember: string;
  ownerName: string;
  channels: SpaceChannel[];
  members: SpaceMember[];
  openInvites: number;
  relayed: boolean;
  anyoneOnline: boolean;
  /** My place here: `left` and `removed` spaces are read-only. */
  state: 'active' | 'left' | 'removed';
  /** Owner side: someone's Leave is stored but they are not removed yet. */
  leavePending: boolean;
  /** The bots in this space (names for the channel header, `muted` for hiding rows). */
  bots?: SpaceDetailBot[];
  botRules?: SpaceBotRules;
}
/** Why the one running a bot refused the last line it was handed (mine only). */
export type BotRefusal = 'unavailable' | 'limit' | 'busy' | 'not_allowed';
export interface SpaceDetailBot {
  id: string;
  name: string;
  ownerName: string;
  mine: boolean;
  /** The viewer hid this bot on this PC. */
  muted: boolean;
  /** Mine only: the bot's owner stopped it. */
  silenced?: boolean | null;
  /** Mine only: answers allowed per day, and how many were used. */
  dayCap?: number | null;
  usedToday?: number;
  /** Mine only: the last line it was handed was refused, and why. Cleared when a later line is taken. */
  refused?: BotRefusal | null;
}
export interface SpaceBotRules {
  perPerson: number;
  /** null means every channel; otherwise only these channel ids. */
  channels: string[] | null;
}
export interface SpaceReaction {
  code: ReactionCode;
  count: number;
  mine: boolean;
}
export interface SpaceFile {
  sha: string;
  name: string;
  size: number;
  have: boolean;
  state: 'idle' | 'getting' | 'failed';
  /** 0 to 100 while `state` is `getting`. */
  percent: number;
}
export interface SpaceMessage {
  id: string;
  channel: string;
  member: string;
  authorName: string;
  /** A bot spoke (the field keeps its old name). */
  isFish: boolean;
  botId?: string | null;
  botName?: string | null;
  /** Said by me. Never true for a bot line, even my own bot's. */
  isMine: boolean;
  /** A bot line: the name of the person who owns the bot, and whether that is me. */
  ownerName?: string | null;
  ownedByMe?: boolean;
  createdAtMs: number;
  text: string;
  replyTo: string | null;
  replyPreview: { authorName: string; text: string } | null;
  deleted: { byName: string; byOwner: boolean } | null;
  reactions: SpaceReaction[];
  file: SpaceFile | null;
  blocked: boolean;
}

const MESSAGE_PAGE = 50;

export const spaceDetail = (space: string) => invoke<SpaceDetail>('space_detail', { space });
export const spaceMessages = (space: string, channel: string, before: string | null, limit = MESSAGE_PAGE) =>
  invoke<SpaceMessage[]>('space_messages', { space, channel, before, limit });
export const spaceSend = (space: string, channel: string, text: string, reply: string | null) =>
  invoke<SpaceMessage>('space_send', { space, channel, text, reply });
export const spaceRead = (space: string, channel: string) => invoke<SpaceDetail>('space_read', { space, channel });
export const spaceChannelCreate = (space: string, name: string) =>
  invoke<SpaceDetail>('space_channel_create', { space, name });
export const spaceChannelRename = (space: string, channel: string, name: string) =>
  invoke<SpaceDetail>('space_channel_rename', { space, channel, name });
export const spaceChannelArchive = (space: string, channel: string) =>
  invoke<SpaceDetail>('space_channel_archive', { space, channel });

export const spaceRemoveMember = (space: string, member: string) =>
  invoke<SpaceDetail>('space_remove_member', { space, member });
export const spaceLeave = (space: string) => invoke<SpaceDetail>('space_leave', { space });
export const spaceForget = (space: string) => invoke<SpacesView>('space_forget', { space });
export const spaceBlock = (space: string, member: string, on: boolean) =>
  invoke<SpaceDetail>('space_block', { space, member, on });
export const spaceReact = (space: string, target: string, code: ReactionCode, on: boolean) =>
  invoke<SpaceMessage>('space_react', { space, target, code, on });
export const spaceDelete = (space: string, target: string) => invoke<SpaceMessage>('space_delete', { space, target });

// Presence (D8.3/D8.4). The window reports online or away; the checkbox hides me from everyone.
export const spacePresence = (state: 'online' | 'away') => invoke<void>('space_presence', { state });
export const spaceShowOnline = (on: boolean) => invoke<SpacesView>('space_show_online', { on });

export const AWAY_AFTER_MS = 10 * 60 * 1000;
const REPORT_GAP_MS = 30_000;

/**
 * Reports Online on activity in this window and Away after ten minutes without any. Looks only at this window
 * (pointer, keys, focus), never at the operating system. At most one call per 30 s for the same state.
 */
export function usePresenceReporter(enabled: boolean): void {
  useEffect(() => {
    if (!enabled) return;
    let state: 'online' | 'away' | null = null;
    let last = 0;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const report = (s: 'online' | 'away') => {
      const now = Date.now();
      if (s === state && now - last < REPORT_GAP_MS) return;
      state = s;
      last = now;
      void spacePresence(s).catch(() => undefined);
    };
    const active = () => {
      report('online');
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => report('away'), AWAY_AFTER_MS);
    };
    active();
    window.addEventListener('pointerdown', active);
    window.addEventListener('keydown', active);
    window.addEventListener('focus', active);
    return () => {
      if (timer) clearTimeout(timer);
      window.removeEventListener('pointerdown', active);
      window.removeEventListener('keydown', active);
      window.removeEventListener('focus', active);
    };
  }, [enabled]);
}

// Files (D8). The backend reads the file at `path`; the bytes never pass through the page.
export const spaceFileOffer = (space: string, channel: string, path: string) =>
  invoke<SpaceMessage>('space_file_offer', { space, channel, path });
export const spaceFileGet = (space: string, sha: string) => invoke<{ have: boolean }>('space_file_get', { space, sha });
export const spaceFileSave = (space: string, sha: string) => invoke<{ name: string }>('space_file_save', { space, sha });

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export const REACTION_LABEL: Record<ReactionCode, string> = {
  thumbs_up: 'Thumbs up', heart: 'Heart', laugh: 'Laugh', wow: 'Wow',
  sad: 'Sad', party: 'Party', eyes: 'Eyes', check: 'Check',
};

/** Merge a page into a list: by id, keep server order (oldest first), no duplicates. */
export function mergeMessages(list: SpaceMessage[], page: SpaceMessage[]): SpaceMessage[] {
  const byId = new Map<string, SpaceMessage>();
  for (const m of list) byId.set(m.id, m);
  for (const m of page) byId.set(m.id, m);
  const seen = new Set(list.map((m) => m.id));
  const order = [...list.map((m) => m.id), ...page.map((m) => m.id).filter((id) => !seen.has(id))];
  return order.map((id) => byId.get(id)!);
}

/** The space's detail. Reloads when Spaces changes. Null when no space is picked. */
export function useSpace(space: string | null): {
  detail: SpaceDetail | null;
  error: string | null;
  reload: () => void;
} {
  const [detail, setDetail] = useState<SpaceDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  useEffect(() => {
    if (!space) return;
    let alive = true;
    let stop: (() => void) | null = null;
    void listen('spaces://changed', () => {
      if (alive) setTick((t) => t + 1);
    }).then((u) => {
      if (alive) stop = u;
      else u();
    });
    return () => {
      alive = false;
      stop?.();
    };
  }, [space]);
  useEffect(() => {
    if (!space) {
      setDetail(null);
      setError(null);
      return;
    }
    let alive = true;
    spaceDetail(space)
      .then((d) => {
        if (alive) {
          setDetail(d);
          setError(null);
        }
      })
      .catch((e: unknown) => {
        if (alive) setError(spacesErrorText(e));
      });
    return () => {
      alive = false;
    };
  }, [space, tick]);
  return { detail, error, reload };
}

/**
 * The newest messages of one channel. Refetches on a message event for this channel and marks it read after each load.
 * `loadOlder` fetches the page before the oldest shown message. `send` returns an error text or null.
 */
export function useChannel(
  space: string | null,
  channel: string | null,
): {
  messages: SpaceMessage[];
  atStart: boolean;
  loadOlder: () => Promise<void>;
  send: (text: string, reply: string | null) => Promise<string | null>;
  error: string | null;
} {
  const [messages, setMessages] = useState<SpaceMessage[]>([]);
  const [atStart, setAtStart] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setMessages([]);
    setAtStart(false);
    setError(null);
    if (!space || !channel) return;
    let alive = true;
    const loadNewest = (first: boolean) => {
      spaceMessages(space, channel, null, MESSAGE_PAGE)
        .then(async (page) => {
          if (!alive) return;
          setMessages((list) => mergeMessages(list, page));
          if (first) setAtStart(page.length < MESSAGE_PAGE);
          setError(null);
          await spaceRead(space, channel);
        })
        .catch((e: unknown) => {
          if (alive) setError(spacesErrorText(e));
        });
    };
    loadNewest(true);
    let stop: (() => void) | null = null;
    void listen<{ space: string; channel?: string }>('spaces://message', (e) => {
      if (!alive || e.payload.space !== space) return;
      if (e.payload.channel !== undefined && e.payload.channel !== channel) return;
      loadNewest(false);
    }).then((u) => {
      if (alive) stop = u;
      else u();
    });
    return () => {
      alive = false;
      stop?.();
    };
  }, [space, channel]);

  const loadOlder = useCallback(async () => {
    if (!space || !channel || atStart || messages.length === 0) return;
    try {
      const page = await spaceMessages(space, channel, messages[0].id, MESSAGE_PAGE);
      setAtStart(page.length < MESSAGE_PAGE);
      setMessages((list) => mergeMessages(page, list));
    } catch (e) {
      setError(spacesErrorText(e));
    }
  }, [space, channel, atStart, messages]);

  const send = useCallback(
    async (text: string, reply: string | null): Promise<string | null> => {
      if (!space || !channel) return 'Pick a channel first.';
      try {
        await spaceSend(space, channel, text, reply);
        return null;
      } catch (e) {
        return spacesErrorText(e);
      }
    },
    [space, channel],
  );

  return { messages, atStart, loadOlder, send, error };
}

// ---- bots in a space ----------------------------------------------------------------------------------

export interface SpaceBotRow {
  id: string;
  name: string;
  ownerName: string;
  ownerMember: string;
  mine: boolean;
  /** The viewer hid this bot on this PC. */
  muted: boolean;
  /** Mine only: the bot's owner stopped it. */
  silenced: boolean | null;
  mode: BotMode | null;
  memory: boolean | null;
  dayCap: number | null;
  potato: string | null;
  usedToday: number;
  heldBackToday: number;
  /** Mine only: the last line it was handed was refused, and why. */
  refused?: BotRefusal | null;
}
export interface SpaceBotList {
  space: string;
  isOwner: boolean;
  rules: SpaceBotRules;
  mineCount: number;
  /** A sentence saying why I cannot add a bot now, or null. */
  addBlocked: string | null;
  /** Names of my bots waiting for the space owner's computer. */
  pendingAdd: string[];
  bots: SpaceBotRow[];
}
export interface SpaceBotCandidate {
  id: string;
  name: string;
  suggestedName: string;
}
export interface SpaceBotPatch {
  mode?: BotMode;
  memory?: boolean;
  dayCap?: number;
  silenced?: boolean;
  muted?: boolean;
}

export const spaceBotList = (space: string) => invoke<SpaceBotList>('space_bot_list', { space });
export const spaceBotCandidates = (space: string) =>
  invoke<{ space: string; candidates: SpaceBotCandidate[] }>('space_bot_candidates', { space });
export const spaceBotAdd = (space: string, potatoBot: string, name: string) =>
  invoke<SpaceBotList>('space_bot_add', { space, potatoBot, name });
export const spaceBotRemove = (space: string, bot: string) => invoke<SpaceBotList>('space_bot_remove', { space, bot });
export const spaceBotRules = (space: string, perPerson: number, channels: string[] | null) =>
  invoke<SpaceBotList>('space_bot_rules', { space, perPerson, channels });
export const spaceBotLocal = (space: string, bot: string, patch: SpaceBotPatch) =>
  invoke<SpaceBotList>('space_bot_local', { space, bot, patch });

/** The bot list of one space. Reloads when Spaces changes. `set` takes the list a command answered with. */
export function useSpaceBots(space: string | null): {
  list: SpaceBotList | null;
  error: string | null;
  set: (l: SpaceBotList) => void;
} {
  const [list, setList] = useState<SpaceBotList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!space) return;
    let alive = true;
    let stop: (() => void) | null = null;
    try {
      listen('spaces://changed', () => {
        if (alive) setTick((t) => t + 1);
      }).then(
        (u) => {
          if (alive) stop = u;
          else u();
        },
        () => {},
      );
    } catch {
      /* no event bridge (tests): the list still loads once */
    }
    return () => {
      alive = false;
      stop?.();
    };
  }, [space]);
  useEffect(() => {
    if (!space) {
      setList(null);
      setError(null);
      return;
    }
    let alive = true;
    spaceBotList(space).then(
      (l) => {
        if (alive && l) {
          setList(l);
          setError(null);
        }
      },
      (e: unknown) => {
        if (alive) setError(spacesErrorText(e));
      },
    );
    return () => {
      alive = false;
    };
  }, [space, tick]);
  return { list, error, set: setList };
}
