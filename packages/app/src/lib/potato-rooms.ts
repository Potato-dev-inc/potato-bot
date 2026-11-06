// Bot groups on the desktop (plan 12, slice G8). The names mirror the Rust serde shapes in
// src-tauri/src/commands/potato_rooms.rs and src-tauri/src/potato/rooms/store.rs (camelCase). Desktop only.
// Every call refuses while the Labs switch `potato_groups_enabled` is off. `fakeRoomsTransport` is the in-memory twin for tests.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { plainError, type PotatoFileCard, type PotatoHandoff } from './potato';
import type { RoomWorkView } from './potato-work';

export interface BotChip {
  id: string;
  name: string;
}

export interface RoomView {
  id: string;
  name: string;
  bots: BotChip[];
  lastMs: number;
  unread: number;
  /** What you wrote for the bots of this group. */
  instructions?: string;
  /** The shared folder as chosen (empty: the default) and its full path inside the bots' computer. */
  folder?: string;
  folderPath?: string;
  /** `listen`: one bot answers a line to nobody. `mentions`: a bot speaks only when called. */
  mode?: RoomMode;
  /** Bots switched off in this group only. */
  offBots?: string[];
  /** The names of the bots now typing in this group. */
  typing: string[];
  /** The live step list of each bot working in this group. */
  work?: RoomWorkView[];
}

export interface RoomMember {
  kind: 'person' | 'bot';
  id: string;
  name: string;
}

export interface RoomLine {
  id: string;
  from: RoomMember;
  text: string;
  /** Milliseconds since 1970 (the Rust field is `at`). */
  at: number;
  /** 0 for the person's line; a bot's reply is one hop deeper than the line it answers. */
  hop: number;
  inReplyTo: string | null;
  /** A file the bot sent into the group (send_file). It opens with the sending bot's id (from.id). */
  file?: PotatoFileCard | null;
  /** A task this line hands to another bot (`hand_off`). The line is drawn as a hand-off card. */
  handoff?: PotatoHandoff | null;
}

/** A line with this hop is a grey note from the app (a bot stopped, is over its limit...), not something a bot said. Same as Rust NOTE_HOP. */
export const ROOM_NOTE_HOP = 255;

export type RoomMode = 'listen' | 'mentions';

/** What the group settings screen saves in one call. Same checks as Rust `rstore::apply_settings`. */
export interface RoomSettingsInput {
  instructions: string;
  folder: string;
  mode: RoomMode;
  offBots: string[];
}

/** Same numbers as Rust `MAX_INSTRUCTIONS` and `MAX_FOLDER`. */
export const ROOM_MAX_INSTRUCTIONS = 2000;
export const ROOM_MAX_FOLDER = 40;
/** Where the groups' shared folders are inside the bots' computer (Rust `FOLDER_ROOT`). */
export const ROOM_FOLDER_ROOT = '/home/pm/groups/';

/** The same check as Rust `clean_folder`, for instant feedback. Null when fine (empty means the default folder). */
export function folderProblem(folder: string): string | null {
  const f = folder.trim().toLowerCase();
  if (f === '') return null;
  if ([...f].length > ROOM_MAX_FOLDER || !/^[a-z0-9][a-z0-9_-]*$/.test(f)) return 'Use letters, numbers, - or _ (up to 40).';
  return null;
}

/** The word that calls every bot in the group. */
export const ROOM_EVERYONE = 'everyone';

/** Same numbers as Rust `rstore::MIN_BOTS`, `MAX_BOTS` and `MAX_NAME`. */
export const ROOM_MIN_BOTS = 2;
export const ROOM_MAX_BOTS = 10;
export const ROOM_MAX_NAME = 60;
/** Same as Rust `rstore::MAX_TEXT`: a longer line is refused before it is sent. */
export const ROOM_MAX_TEXT = 8000;

/** The words of the group list, the New group sheet and the placeholder. Plain English, shown as written. */
export const ROOM_COPY = {
  listHeader: 'Groups',
  botsHeader: 'Bots',
  newGroup: 'New group',
  botsCount: (n: number) => `${n} bots`,
  sheetTitle: 'New group',
  nameLabel: 'Name',
  namePlaceholder: 'Example: Weekend plans',
  botsLabel: 'Bots in this group',
  botsHint: 'Pick 2 to 10 bots.',
  make: 'Make group',
  cancel: 'Cancel',
  emptyNoBots: 'You need at least 2 bots to make a group.',
  placeholder: 'Message the group',
  send: 'Send',
  menu: 'Group menu',
  menuRename: 'Rename group',
  menuBots: 'Change bots',
  menuDelete: 'Delete group',
  menuSettings: 'Group settings',
  settingsTitle: 'Group settings',
  settingsNameLabel: 'Group name',
  settingsWhoLabel: 'Who answers',
  settingsListen: 'Listen',
  settingsListenHint: 'One bot answers when you talk to nobody. Call a bot with @ to pick it.',
  settingsMentions: 'Mentions',
  settingsMentionsHint: 'A bot speaks only when you call it with @ (or reply to it).',
  settingsMembers: 'Bots in this group',
  settingsOnLabel: (name: string) => `${name} answers in this group`,
  settingsRemove: (name: string) => `Remove ${name}`,
  settingsAdd: 'Add a bot',
  settingsAddNone: 'Pick a bot…',
  settingsAddButton: 'Add',
  settingsInstructions: 'Group instructions',
  settingsInstructionsHint: 'Every bot in this group reads these. They cannot switch a tool on.',
  settingsFolder: 'Shared folder',
  settingsFolderHint: (path: string) => `Bots keep the group's files in ${path}. Leave empty to use the default.`,
  settingsSaved: 'Saved',
  settingsLimit: (n: number) => `A group can have up to ${n} bots.`,
  save: 'Save',
  typing: (name: string) => `${name} is typing…`,
  empty: (bot: string) => `Say hi. Call a bot with @ and its name, for example @${bot}. Say everyone to hear from all of them.`,
  deleteTitle: 'Delete this group?',
  deleteBody: 'The chat is removed. The bots stay.',
  deleteConfirm: 'Delete group',
  changeBotsTitle: 'Change bots',
  stop: 'Stop',
  jump: 'Jump to latest',
  gone: 'That group no longer exists.',
} as const;

export interface RoomsTransport {
  list(): Promise<RoomView[]>;
  create(name: string, botIds: string[]): Promise<RoomView>;
  rename(roomId: string, name: string): Promise<RoomView>;
  setBots(roomId: string, botIds: string[]): Promise<RoomView>;
  setSettings(roomId: string, settings: RoomSettingsInput): Promise<RoomView>;
  remove(roomId: string): Promise<void>;
  lines(roomId: string, limit: number): Promise<RoomLine[]>;
  send(roomId: string, text: string): Promise<RoomLine>;
  markRead(roomId: string): Promise<void>;
  onChanged(cb: (roomId: string | null) => void): Promise<UnlistenFn>;
}

export const tauriRoomsTransport: RoomsTransport = {
  list: () => invoke<RoomView[]>('potato_rooms_list'),
  create: (name, botIds) => invoke<RoomView>('potato_room_create', { name, botIds }),
  rename: (roomId, name) => invoke<RoomView>('potato_room_rename', { roomId, name }),
  setBots: (roomId, botIds) => invoke<RoomView>('potato_room_set_bots', { roomId, botIds }),
  setSettings: (roomId, s) => invoke<RoomView>('potato_room_set_settings', { roomId, instructions: s.instructions, folder: s.folder, mode: s.mode, offBots: s.offBots }),
  remove: (roomId) => invoke<void>('potato_room_delete', { roomId }),
  lines: (roomId, limit) => invoke<RoomLine[]>('potato_room_lines', { roomId, limit }),
  send: (roomId, text) => invoke<RoomLine>('potato_room_send', { roomId, text }),
  markRead: (roomId) => invoke<void>('potato_room_mark_read', { roomId }),
  onChanged: (cb) =>
    listen<{ roomId?: string } | null>('potato://room-changed', (e) => cb(e.payload?.roomId ?? null)),
};

/** The same checks as Rust `rstore::validate`, for instant feedback. The sentences equal `RoomError::to_wire`. Null when valid. */
export function roomProblem(name: string, botIds: string[]): string | null {
  const n = name.trim();
  if (n.length === 0) return 'Give the group a name.';
  if ([...n].length > ROOM_MAX_NAME) return 'Use 60 characters or fewer.';
  if (botIds.length < ROOM_MIN_BOTS) return 'Pick at least 2 bots.';
  if (botIds.length > ROOM_MAX_BOTS) return 'A group can have up to 10 bots.';
  if (new Set(botIds).size !== botIds.length) return 'Each bot can be in a group once.';
  return null;
}

/** One plain sentence for each room error code (the app puts the same sentence after the code). */
export const ROOM_ERROR_TEXT: Record<string, string> = {
  POTATO_GROUPS_OFF: 'Turn on Bot groups in Labs first.',
  POTATO_ROOM_NAME: 'Give the group a name.',
  POTATO_ROOM_BOTS: 'Pick 2 to 10 bots that still exist.',
  POTATO_ROOM_LIMIT: 'You can have up to 50 groups.',
  POTATO_ROOM_GONE: 'This group no longer exists.',
  POTATO_ROOM_ID: 'That group id is not valid.',
  POTATO_ROOM_IO: 'Could not save the group. Try again.',
  POTATO_ROOM_FOLDER: 'Use letters, numbers, - or _ (up to 40).',
  POTATO_ROOM_MODE: 'Pick Listen or Mentions.',
};

/** The words for a failed group call: the app's own sentence when it sent one, else the plain sentence of its code, else the fallback. Never the code. */
export function roomErrorText(error: unknown, fallback: string): string {
  const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const code = /^([A-Z][A-Z0-9_]{2,})(?::|$)/.exec(raw.trim())?.[1];
  const sentence = code && raw.trim() === code ? '' : plainError(error, '');
  if (sentence) return sentence;
  return (code && ROOM_ERROR_TEXT[code]) || fallback;
}

/** True when the error says the group is gone. */
export function isRoomGone(error: unknown): boolean {
  const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return raw.startsWith('POTATO_ROOM_GONE');
}

/** In-memory rooms for tests. A send adds the person's line, then a reply from the first bot one hop deeper. */
export interface FakeRoomsOptions {
  /** Bot id to the name a group shows (default: the id). */
  names?: Record<string, string>;
  /** Every bot answers a send (the first with "Got it.", the others with "Me too."). Default: only the first. */
  everyone?: boolean;
}

export interface FakeRooms extends RoomsTransport {
  rooms: RoomView[];
  sent: string[];
  /** What each Save of the settings screen sent, in order. */
  settingsSaved: Array<{ roomId: string } & RoomSettingsInput>;
  /** The groups markRead was called for, in order. */
  read: string[];
  /** Puts a line in a group as if the app saved it, and announces the change. */
  addLine(roomId: string, line: Partial<RoomLine> & Pick<RoomLine, 'from' | 'text'>): RoomLine;
  setTyping(roomId: string, names: string[]): void;
  /** Test hook: the live step lists of the bots working in a group. */
  setWork(roomId: string, work: RoomWorkView[]): void;
  /** The next call of that name ('send', 'lines', ...) fails with this message. */
  failNext(call: keyof RoomsTransport, message: string): void;
}

export function fakeRoomsTransport(options: FakeRoomsOptions = {}): FakeRooms {
  let seq = 0;
  const rooms: RoomView[] = [];
  const linesByRoom = new Map<string, RoomLine[]>();
  const sent: string[] = [];
  const read: string[] = [];
  const settingsSaved: Array<{ roomId: string } & RoomSettingsInput> = [];
  const failures = new Map<string, string>();
  const nameOf = (id: string) => options.names?.[id] ?? id;
  const check = (call: string) => {
    const message = failures.get(call);
    if (message === undefined) return;
    failures.delete(call);
    throw new Error(message);
  };
  const listeners = new Set<(roomId: string | null) => void>();
  const notify = (roomId: string | null) => listeners.forEach((cb) => cb(roomId));
  const find = (roomId: string): RoomView => {
    const room = rooms.find((r) => r.id === roomId);
    if (!room) throw new Error('POTATO_ROOM_GONE: This group no longer exists.');
    return room;
  };
  const nextId = (prefix: string) => `${prefix}${String(++seq).padStart(12, '0')}`;
  const fail = (message: string): never => {
    throw new Error(message);
  };
  return {
    rooms,
    sent,
    read,
    settingsSaved,
    addLine(roomId, partial) {
      find(roomId);
      const list = linesByRoom.get(roomId) ?? [];
      linesByRoom.set(roomId, list);
      const line: RoomLine = { id: nextId('l'), at: Date.now(), hop: 1, inReplyTo: null, ...partial };
      list.push(line);
      notify(roomId);
      return line;
    },
    setWork(roomId, work) {
      find(roomId).work = work.map((w) => ({ bot: w.bot, run: { ...w.run, steps: [...w.run.steps] } }));
      notify(roomId);
    },
    setTyping(roomId, names) {
      find(roomId).typing = [...names];
      notify(roomId);
    },
    failNext(call, message) {
      failures.set(call, message);
    },
    async list() {
      check('list');
      return rooms.map((r) => ({ ...r, bots: [...r.bots], typing: [...r.typing], work: [...(r.work ?? [])] }));
    },
    async create(name, botIds) {
      const problem = roomProblem(name, botIds);
      if (problem) fail(`POTATO_ROOM_BOTS: ${problem}`);
      const id = nextId('r');
      const room: RoomView = {
        id,
        name: name.trim(),
        bots: botIds.map((id) => ({ id, name: nameOf(id) })),
        lastMs: Date.now(),
        unread: 0,
        instructions: '',
        folder: '',
        folderPath: `${ROOM_FOLDER_ROOT}${id}`,
        mode: 'listen',
        offBots: [],
        typing: [],
      };
      rooms.push(room);
      notify(room.id);
      return { ...room, bots: [...room.bots] };
    },
    async rename(roomId, name) {
      const room = find(roomId);
      if (name.trim().length === 0) fail('POTATO_ROOM_NAME: Give the group a name.');
      if ([...name.trim()].length > ROOM_MAX_NAME) fail('POTATO_ROOM_NAME: Use 60 characters or fewer.');
      room.name = name.trim();
      notify(roomId);
      return { ...room, bots: [...room.bots] };
    },
    async setBots(roomId, botIds) {
      const room = find(roomId);
      const problem = roomProblem(room.name, botIds);
      if (problem) fail(`POTATO_ROOM_BOTS: ${problem}`);
      room.bots = botIds.map((id) => ({ id, name: nameOf(id) }));
      room.offBots = (room.offBots ?? []).filter((id) => botIds.includes(id));
      notify(roomId);
      return { ...room, bots: [...room.bots] };
    },
    async setSettings(roomId, s) {
      check('setSettings');
      const room = find(roomId);
      const folderBad = folderProblem(s.folder);
      if (folderBad) fail(`POTATO_ROOM_FOLDER: ${folderBad}`);
      if (s.mode !== 'listen' && s.mode !== 'mentions') fail('POTATO_ROOM_MODE: Pick Listen or Mentions.');
      const off = room.bots.map((b) => b.id).filter((id) => s.offBots.includes(id));
      if (off.length >= room.bots.length) fail('POTATO_ROOM_BOTS: Keep at least one bot on in this group.');
      room.instructions = s.instructions.trim().slice(0, ROOM_MAX_INSTRUCTIONS);
      room.folder = s.folder.trim().toLowerCase();
      room.folderPath = `${ROOM_FOLDER_ROOT}${room.folder || room.id}`;
      room.mode = s.mode;
      room.offBots = off;
      settingsSaved.push({ roomId, ...s });
      notify(roomId);
      return { ...room, bots: [...room.bots], offBots: [...off] };
    },
    async remove(roomId) {
      find(roomId);
      rooms.splice(rooms.findIndex((r) => r.id === roomId), 1);
      linesByRoom.delete(roomId);
      notify(roomId);
    },
    async lines(roomId, limit) {
      check('lines');
      find(roomId);
      return (linesByRoom.get(roomId) ?? []).slice(-limit);
    },
    async send(roomId, text) {
      check('send');
      const room = find(roomId);
      const trimmed = text.trim();
      if (trimmed.length === 0) fail('POTATO_EMPTY: Write a message first.');
      if (trimmed.length > ROOM_MAX_TEXT) fail('POTATO_TOO_LONG: This message is too long.');
      sent.push(trimmed);
      const list = linesByRoom.get(roomId) ?? [];
      linesByRoom.set(roomId, list);
      const line: RoomLine = {
        id: nextId('l'),
        from: { kind: 'person', id: 'owner', name: 'You' },
        text: trimmed,
        at: Date.now(),
        hop: 0,
        inReplyTo: null,
      };
      list.push(line);
      room.lastMs = line.at;
      notify(roomId);
      const speakers = options.everyone ? room.bots : room.bots.slice(0, 1);
      speakers.forEach((speaker, i) => {
        queueMicrotask(() => {
          list.push({
            id: nextId('l'),
            from: { kind: 'bot', id: speaker.id, name: speaker.name },
            text: i === 0 ? 'Got it.' : 'Me too.',
            at: Date.now(),
            hop: 1,
            inReplyTo: line.id,
          });
          room.unread += 1;
          notify(roomId);
        });
      });
      return line;
    },
    async markRead(roomId) {
      read.push(roomId);
      find(roomId).unread = 0;
    },
    async onChanged(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
  };
}
