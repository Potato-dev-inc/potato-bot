// Potato Bot on the desktop (spec section 4). The names mirror the Rust serde shapes in
// src-tauri/src/potato/{service,store}.rs and src-tauri/src/commands/potato.rs, in camelCase. Desktop only:
// the page talks to Tauri commands and to the `potato://changed` event; the phone and the bridge never see this.
// Nothing here draws. `fakePotatoTransport` is the in-memory twin for tests (the pattern of `fakeCompanionTransport`).
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isTauri } from './cockpit-api';
import type { WorkRunView } from './potato-work';
import { fakeArtifactTransport, tauriArtifactTransport, type ArtifactKind, type ArtifactTransport, type FakeArtifactSeed } from './potato-artifacts';

/** The two voices of a bot. */
export type TalkPersona = 'fish' | 'potato' | 'pro';

/** What the page shows for a bot's brain (`BrainState` in service.rs). */
export type BrainState =
  | 'off'
  | 'idle'
  | 'starting'
  | 'ready'
  | 'restarting'
  | 'stopped'
  | 'needs_key'
  | 'needs_node'
  | 'missing_program'
  | 'daily_limit'
  | 'agents';

export type BrainKind = 'deepseek' | 'go' | 'agents' | 'custom';

export interface MoodNow {
  mood: string;
  intensity: number;
  energy: number;
  hunger: number;
}

export interface PotatoBotView {
  id: string;
  name: string;
  talkPersona: TalkPersona;
  brainState: BrainState;
  /** The bot's own talk brain, or null for the Settings brain. Absent in older state: read it as null. */
  talkBrain?: BrainKind | null;
  /** The person archived this chat: the list hides it under Archived. Absent: not archived. Nothing is deleted. */
  archived?: boolean;
  /** The last error code the brain reported and that nothing has cleared since. */
  lastError: string | null;
  typing: boolean;
  /** The live step list of the turn running now (or the one that just ended). Absent or null: just the thinking row. */
  work?: WorkRunView | null;
  mood: MoodNow | null;
  provider: string | null;
  unread: number;
  lastLine: string | null;
  lastAt: number | null;
  usedTodayNano: number;
  limitCents: number;
  /** No limit is on: no limit line, no strip, no "of $X". Missing: off. */
  limitOff?: boolean;
  openCards: number;
}

export interface PotatoSettings {
  v: number;
  brain: BrainKind;
  defaultPersona: TalkPersona;
  dailyLimitCents: number;
  /** No limit: the money pause never fires and a job's $5 cap does not apply. Set only here, by the owner. Missing: off. */
  dailyLimitOff?: boolean;
  /** Actions one bot may take on its sandbox computer in a day (default 500). The settings page for it comes with the sandbox settings. */
  sandboxActionsPerDay?: number;
  /** The brains to try, in order, when the talk brain is out of balance or not answering. Never `agents`; empty: none. */
  fallbacks?: BrainKind[];
  /** The endpoint of a `custom` brain. Its key is kept in the OS credential store. */
  custom?: { baseUrl: string; model: string };
  /** Read replies aloud: the system voice, its speed, and the bots that read their replies. Missing: the defaults. */
  voice?: PotatoVoice;
}

/** Potato Bot's voice settings (`potato/voice.rs`). The webview speaks with the system voices; no key, nothing leaves the PC. */
export interface PotatoVoice {
  /** The chosen voice's URI. Empty: the system default voice. */
  voiceUri: string;
  /** Speed in percent of normal, 50 to 200. */
  ratePercent: number;
  /** The bots whose new replies are read aloud while the page is visible. */
  readAloudBots: string[];
}

export interface PotatoState {
  /** The Labs switch `potato_bot_enabled`. Off: no row, no palette entry, no page. */
  enabled: boolean;
  settings: PotatoSettings;
  key: { set: boolean; last4: string | null };
  node: { ready: boolean; found: string | null };
  bots: PotatoBotView[];
}

export type FromKind = 'you' | 'bot' | 'person' | 'note';
export type MessageKind = 'text' | 'helper_card' | 'helper_result' | 'note' | 'decision_card' | 'decision_result';

/** A decision card the bot raised (Professional mode). On a `decision_card` row the card id is the row's `requestId` and
 *  the action (or the question) is its `text`. On a `decision_result` row (same `requestId`) `answer` says how it ended. */
export interface PotatoDecisionCard {
  /** `approval`: three fixed buttons (Allow once, Allow for this job, Deny). `question`: one button per `choices`. */
  kind: 'approval' | 'question';
  /** Why the bot asks. Empty on result rows. */
  why: string;
  /** The buttons of a question, in order; empty for an approval. */
  choices: string[];
  /** A Keep going card (a job reached a limit or got stuck): choices are ["Keep going", "Stop"]. Absent otherwise. */
  keepGoing?: boolean;
  /** A connector card: the service the bot wants (for example GitHub). The first button opens Settings > Connectors. */
  connector?: string;
  /** A sign-in card: the https origin the bot asked to sign in to. Its buttons are Sign in, Take over and Not now. */
  login?: string;
  /** An approval that may be answered Always allow: it names exactly one command class and that class is not a destroying
   *  one (never rm or force-push). Absent otherwise; the app shows the button only when this is true. */
  canAlwaysAllow?: boolean;
  /** Result rows only: `allow_once`, `allow_job`, `always_allow`, `deny`, `choice:N` (N from 0) or `cancelled`. */
  answer?: string;
}

/** What a sign-in card shows about the site: the card's own, and the origin the app reads from the browser's front tab. */
export interface PotatoLoginOrigin {
  /** The card's site, as the bot asked for it (`https://github.com`). */
  site: string;
  /** The origin the browser really has in front. Null: no page could be read, or the computer is off. */
  origin: string | null;
  /** The front page is exactly the card's site. */
  matches: boolean;
}

/** What `answerCard` gives back. */
export interface PotatoCardAnswered {
  cardId: string;
  answer: string;
  /** The bot's job was started again with the decision. */
  jobResumed: boolean;
  /** A normal turn carrying the decision was sent (no job was waiting). */
  turnStarted: boolean;
}

/** A card as the app keeps it (`potato_open_cards`). */
export interface PotatoOpenCard {
  id: string;
  kind: 'approval' | 'question';
  action: string;
  why: string;
  choices: string[];
  createdMs: number;
  state: 'open' | 'answered' | 'cancelled';
  keepGoing?: string | null;
  job?: string | null;
}

/** One file the bot changed during its job with its file tools: what Restore will do (`potato/file_undo.rs`). */
export interface PotatoJobFile {
  id: string;
  /** The path inside the bot's computer, under `/home/pm/`. */
  path: string;
  /** `saved`: the old content is kept. `created`: the job made the file. `too_big` and `over_limit`: not kept, no Restore. */
  state: 'saved' | 'created' | 'too_big' | 'over_limit';
  bytes: number;
  at: number;
  op: 'write' | 'edit';
  restored: boolean;
}

/** The files a job changed. Changes made by a terminal command are never in it. */
export interface PotatoJobFiles {
  jobId: string;
  files: PotatoJobFile[];
  /** More files were changed than the list holds. */
  overflow: boolean;
}

/** The bot's job (Professional mode) as the job bar shows it. `null` from `job`: the bot has none. */
export interface PotatoJob {
  /** What the person asked for, in their words. */
  goal: string;
  /** The step it is on now, in words. Empty when it has none yet. */
  step: string;
  /** When it started (epoch milliseconds). */
  startedMs: number;
  /** What it has spent so far, in nano-dollars. */
  spendNano: number;
  /** `running` and `waiting` show the bar; the others are over. */
  state: 'running' | 'waiting' | 'done' | 'stopped' | 'failed';
}

export interface PotatoMessage {
  id: string;
  at: number;
  from: { kind: FromKind; name?: string };
  kind: MessageKind;
  text: string;
  mood?: string;
  requestId?: string;
  /** A helper card: the folder name the bot asked for (a name only). */
  project?: string;
  /** A helper result row: the pane the helper worked in, for its Open button. */
  paneId?: string;
  /** A reply's estimated cost in nano-dollars (`potato/reply_cost.rs`). Absent when there is none. */
  costNano?: number;
  /** A file the bot sent (`send_file`): the card with Open and Save. Absent on every other message. */
  file?: PotatoFileCard;
  /** A chart the bot drew (`render_plot`). Absent on every other message. */
  chart?: PotatoChart;
  /** A `decision_card` or `decision_result` row: what the card asks and how it ended. Absent on every other message. */
  card?: PotatoDecisionCard;
  /** Yours, written while the bot was working, and not yet picked up. The label clears by itself when it is. */
  queued?: boolean;
  /** Your emoji reactions to this message, in the fixed order of `REACTIONS`. Absent when there are none. */
  reactions?: string[];
  /** A note that says the bot changed one of its own settings when you asked: the card with Undo. Absent on every other message. */
  change?: PotatoSettingChange;
}

/** The settings a bot may change for itself, when its owner asks in the private chat. Nothing else of the bot is in reach. */
export type SelfSetting = 'name' | 'instructions' | 'persona' | 'model';

export interface PotatoSettingChange {
  /** What Undo restores (`potato_setting_undo`). */
  id: string;
  field: SelfSetting;
  /** Undo was used; the card says so and has no button. */
  undone: boolean;
}

/** The reactions there are, in the order they are offered. Mirrors `EMOJI` in `potato/reactions.rs`; anything else is refused. */
export const REACTIONS = ['👍', '❤️', '😂', '🎉', '😮', '🙏'] as const;

/** A chart the bot drew (`render_plot`): checked numbers. The app draws them; they are never markup. */
export interface PotatoChart {
  kind: 'bar' | 'line' | 'pie';
  title: string;
  points: Array<{ label: string; value: number }>;
}

/** The file a `send_file` card points at (the artifact's id, and what the card shows). */
export interface PotatoFileCard {
  id: string;
  name: string;
  kind: ArtifactKind;
  /** Bytes. */
  size: number;
}

export interface PotatoBot {
  id: string;
  talkPersona: TalkPersona;
  createdMs: number;
}

export interface SendOutcome {
  id: string;
  state: BrainState;
  delivered: boolean;
}

/** What Start answers: the project the helper works in (a name). */
export interface HelperStarted {
  folder: string;
}

export interface PotatoTransport {
  state(): Promise<PotatoState>;
  messages(botId: string, limit?: number): Promise<PotatoMessage[]>;
  send(botId: string, text: string): Promise<SendOutcome>;
  addBot(botId: string): Promise<PotatoBot>;
  setPersona(botId: string, pack: TalkPersona): Promise<PotatoBot>;
  startBot(botId: string): Promise<void>;
  stopBot(botId: string): Promise<void>;
  markRead(botId: string): Promise<void>;
  /** React to a message of the bot's thread with one of `REACTIONS`, or take that reaction back. Returns the message's reactions now. Only the person can; the bot has no such tool. */
  toggleReaction(botId: string, messageId: string, emoji: string): Promise<string[]>;
  /** Undo on a card that says the bot changed one of its own settings: puts the old value back. Only the person can; the bot has no such tool. Once per change. */
  undoSetting?(botId: string, changeId: string): Promise<void>;
  /** Archive (true) or restore (false) a bot's chat. Only the list changes: no message is removed. */
  setArchived(botId: string, archived: boolean): Promise<void>;
  /** "Change folder": the names of the recent projects a helper may work in (names, never paths). */
  helperFolders(botId: string): Promise<string[]>;
  /** Start on a helper card. `folder` is a project name from `helperFolders`; without it the card's own folder is used. */
  helperStart(botId: string, requestId: string, folder?: string): Promise<HelperStarted>;
  /** Not now on a helper card. */
  helperDecline(botId: string, requestId: string): Promise<void>;
  /** A button on a decision card. `answer`: `allow_once`, `allow_job` or `deny` for an approval, a choice number from 0 (as text)
   *  for a question. One answer only: a second one, an answer to a cancelled card or to an unknown id is an error. */
  answerCard(botId: string, cardId: string, answer: string): Promise<PotatoCardAnswered>;
  /** The Always allow button: saves a rule for this bot and the card's one command class, and lets this command run now.
   *  A separate call on purpose: the plain `answerCard` refuses the same words. Refused for rm and force-push cards. */
  answerCardAlways(botId: string, cardId: string): Promise<PotatoCardAnswered>;
  /** Sign in on a sign-in card. The user name and the password go to the app, which types them into the page in the bot's
   *  browser; the card is answered only if that worked. Nothing returns them or the page. Rejects with a sentence on failure. */
  submitLogin(botId: string, cardId: string, username: string, password: string): Promise<PotatoCardAnswered>;
  /** It worked, continue: the person saw the page signed in themselves. Answers the card as signed in, once. */
  confirmLogin(botId: string, cardId: string): Promise<PotatoCardAnswered>;
  /** The origin the bot's browser really has in front, for a sign-in card to show. */
  loginOrigin(botId: string, cardId: string): Promise<PotatoLoginOrigin>;
  /** Sign out on a sign-in card: the site's cookies and stored data leave the bot's browser. */
  forgetSite(botId: string, cardId: string): Promise<{ cookiesCleared: number }>;
  /** The cards of a bot that still wait for an answer, oldest first. */
  openCards(botId: string): Promise<PotatoOpenCard[]>;
  /** The bot's job, for the job bar. Optional: a transport without it shows no bar. Rejects or answers null when there is none. */
  job?(botId: string): Promise<PotatoJob | null>;
  /** The files the bot's job changed with its file tools, each with Restore. Null when the bot has no job. Optional like `job`. */
  jobFiles?(botId: string): Promise<PotatoJobFiles | null>;
  /** Restore on one file: the old content goes back (or a file the job made is removed). Only the person can; the bot has no such tool. */
  restoreJobFile?(botId: string, jobId: string, fileId: string): Promise<PotatoJobFile>;
  /** "Install Node.js": the agent installer's `node` tool, in a window the person watches. */
  installNode(): Promise<void>;
  /** Settings: the saved settings (`<app data>/potato/settings.json`). The key is never in them. */
  settingsGet(): Promise<PotatoSettings>;
  settingsSet(settings: PotatoSettings): Promise<PotatoSettings>;
  /** Settings > Check key: one tiny turn. Answers "Works." or rejects with one plain sentence. */
  checkKey(): Promise<string>;
  /** The files the bots made or received: the Artifacts page and "Save as file" in a thread. */
  artifacts: ArtifactTransport;
  /** `potato://changed`: something about this bot changed (`null`: the Labs switch moved). */
  onChanged?(cb: (botId: string | null) => void): Promise<UnlistenFn>;
}

/** `CODE: A plain sentence.` from a Tauri command, shown as the sentence only. */
export function plainError(error: unknown, fallback: string): string {
  const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const text = raw.replace(/^[A-Z][A-Z0-9_]{2,}:\s*/, '').trim();
  return text || fallback;
}

export const tauriPotatoTransport: PotatoTransport = {
  state: () => invoke<PotatoState>('potato_state'),
  messages: (botId, limit) => invoke<PotatoMessage[]>('potato_messages', { botId, limit: limit ?? null }),
  send: (botId, text) => invoke<SendOutcome>('potato_send', { botId, text }),
  addBot: (botId) => invoke<PotatoBot>('potato_bot_add', { botId }),
  setPersona: (botId, pack) => invoke<PotatoBot>('potato_set_persona', { botId, pack }),
  startBot: (botId) => invoke<void>('potato_bot_start', { botId }),
  stopBot: (botId) => invoke<void>('potato_bot_stop', { botId }),
  markRead: (botId) => invoke<void>('potato_mark_read', { botId }),
  toggleReaction: (botId, messageId, emoji) => invoke<string[]>('potato_toggle_reaction', { botId, messageId, emoji }),
  undoSetting: (botId, changeId) => invoke<void>('potato_setting_undo', { botId, changeId }),
  setArchived: (botId, archived) => invoke<void>('potato_bot_archive', { botId, archived }),
  helperFolders: (botId) => invoke<string[]>('potato_helper_folders', { botId }),
  helperStart: (botId, requestId, folder) => invoke<HelperStarted>('potato_helper_start', { botId, requestId, folder: folder ?? null }),
  helperDecline: (botId, requestId) => invoke<void>('potato_helper_decline', { botId, requestId }),
  answerCard: (botId, cardId, answer) => invoke<PotatoCardAnswered>('potato_answer_card', { botId, cardId, answer }),
  answerCardAlways: (botId, cardId) => invoke<PotatoCardAnswered>('potato_answer_card_always', { botId, cardId }),
  submitLogin: (botId, cardId, username, password) => invoke<PotatoCardAnswered>('potato_submit_login', { botId, cardId, username, password }),
  confirmLogin: (botId, cardId) => invoke<PotatoCardAnswered>('potato_confirm_login', { botId, cardId }),
  loginOrigin: (botId, cardId) => invoke<PotatoLoginOrigin>('potato_login_origin', { botId, cardId }),
  forgetSite: (botId, cardId) => invoke<{ cookiesCleared: number }>('potato_forget_site', { botId, cardId }),
  openCards: (botId) => invoke<PotatoOpenCard[]>('potato_open_cards', { botId }),
  job: (botId) => invoke<PotatoJob | null>('potato_job_get', { botId }),
  jobFiles: (botId) => invoke<PotatoJobFiles | null>('potato_job_files', { botId }),
  restoreJobFile: (botId, jobId, fileId) => invoke<PotatoJobFile>('potato_job_file_restore', { botId, jobId, fileId }),
  installNode: async () => {
    await invoke('potato_install_node');
  },
  settingsGet: () => invoke<PotatoSettings>('potato_settings_get'),
  settingsSet: (settings) => invoke<PotatoSettings>('potato_settings_set', { settings }),
  checkKey: () => invoke<string>('potato_check_key'),
  artifacts: tauriArtifactTransport,
  onChanged: (cb) => {
    if (!isTauri()) return Promise.resolve((() => {}) as UnlistenFn);
    return listen<{ botId?: string } | null>('potato://changed', (e) => cb(e.payload?.botId ?? null));
  },
};

// ---- copy ---------------------------------------------------------------------------------------

export const PERSONA_LABEL: Record<TalkPersona, string> = { fish: 'Fish', potato: 'Potato', pro: 'Professional' };

const BRAIN_LABEL: Record<BrainKind, string> = { deepseek: 'DeepSeek', go: 'OpenCode Go', agents: 'My TUI agent', custom: 'API provider' };

export function brainLabel(kind: BrainKind): string {
  return BRAIN_LABEL[kind] ?? BRAIN_LABEL.deepseek;
}

const STATE_WORD: Record<BrainState, string> = {
  off: 'Off',
  idle: 'Idle',
  starting: 'Starting',
  ready: 'Running',
  restarting: 'Restarting',
  stopped: 'Stopped',
  needs_key: 'Needs a key',
  needs_node: 'Needs Node.js',
  missing_program: 'Not installed',
  daily_limit: 'Limit reached',
  agents: 'Through my agents',
};

export function brainStateWord(state: BrainState): string {
  return STATE_WORD[state] ?? '';
}

/** Cents as dollars: `$0.50`. */
export function dollars(cents: number): string {
  return `$${(Math.max(0, cents) / 100).toFixed(2)}`;
}

/** `Genki`: the mood word with a capital letter. */
export function moodWord(mood: string): string {
  return mood ? mood.charAt(0).toUpperCase() + mood.slice(1) : '';
}

/** `Genki · energy 72`: text only (spec section 4). */
export function moodLine(mood: MoodNow): string {
  return `${moodWord(mood.mood)} · energy ${Math.round(mood.energy)}`;
}

export const COPY = {
  emptyList: 'No bots yet. Make one to start chatting.',
  composer: (name: string) => `Message ${name}`,
  typing: (name: string) => `${name} is typing…`,
  limit: (name: string, cents: number) => `${name} has used today's limit (${dollars(cents)}). Raise it in Settings or wait until tomorrow.`,
  stopped: (name: string) => `${name}'s brain stopped.`,
  noKey: (name: string) => `Add this bot's model key in Settings to talk with ${name}.`,
  needsNode: 'Potato Bot needs Node.js 20 or newer.',
  agentsOnly: 'Mood and personas need the DeepSeek or OpenCode Go brain.',
  missingProgram: "Potato Bot's brain is missing from this install. Reinstall Puppet Master to get it back.",
};

/** The sentence for the last error code the brain reported, or null when it is not worth a card. */
export function errorSentence(name: string, code: string | null): string | null {
  switch (code) {
    case 'PROVIDER_AUTH':
      return `${name} could not use the model key. Check it in Settings.`;
    case 'PROVIDER_LIMIT':
      return 'The model is out of balance or too busy. Try again later.';
    case 'PROVIDER_DOWN':
      return 'The model is not answering right now. Try again in a minute.';
    case 'JOB_STOPPED':
      // The app's own code for a turn that died after doing work (the brain's cause is in the log): the work is kept.
      return `${name}'s job stopped on an error. Say continue and ${name} will pick it up.`;
    case 'NETWORK':
      return `${name} could not reach the internet.`;
    case 'PERSONA_TOO_BIG':
      return `${name}'s persona text is too long.`;
    case 'SURFACE_AUTH':
    case 'INTERNAL':
      return `Something went wrong inside ${name}'s brain.`;
    default:
      return null;
  }
}

/** Today: the clock. Another day: the short date. */
export function formatWhen(at: number, now: number): string {
  const when = new Date(at);
  const today = new Date(now);
  if (when.toDateString() === today.toDateString()) {
    return when.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  }
  return when.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

// ---- the fake -----------------------------------------------------------------------------------

export interface FakePotatoSeed {
  state?: Partial<PotatoState>;
  bots?: PotatoBotView[];
  messages?: Record<string, PotatoMessage[]>;
  /** What `send` answers. Default: delivered. */
  outcome?: Partial<SendOutcome>;
  /** The recent project names "Change folder" offers. */
  folders?: string[];
  /** What Check key answers: a sentence, or throws one. */
  keyCheck?: () => string;
  /** The job each bot has (`job`). Default: none. */
  jobs?: Record<string, PotatoJob | null>;
  /** The files each bot's job changed (`jobFiles`). Default: none. */
  jobFiles?: Record<string, PotatoJobFiles | null>;
  /** The files the bots made or received (see `fakeArtifactTransport`). */
  artifacts?: FakeArtifactSeed;
  /** The sign-in card: the origin the fake browser has in front (null: none), and a sentence Sign in fails with. */
  login?: { origin?: string | null; signInError?: string };
}

export interface FakePotatoTransport extends PotatoTransport {
  /** Every call so far, in order: the method name and its arguments. */
  readonly calls: { method: keyof PotatoTransport; args: unknown[] }[];
  /** Fires the `potato://changed` listeners, as the app does after the brain moved. */
  emit(botId?: string | null): void;
  /** Changes a bot or the state the way the app would, then fires the listeners. */
  patchBot(botId: string, patch: Partial<PotatoBotView>): void;
  setEnabled(on: boolean): void;
  addMessage(botId: string, message: PotatoMessage): void;
}

let nextId = 1;

export function fakePotatoTransport(seed: FakePotatoSeed = {}): FakePotatoTransport {
  const state: PotatoState = {
    enabled: true,
    settings: { v: 1, brain: 'deepseek', defaultPersona: 'fish', dailyLimitCents: 50 },
    key: { set: true, last4: null },
    node: { ready: true, found: null },
    bots: (seed.bots ?? []).map((b) => ({ ...b })),
    ...seed.state,
  };
  const files: Record<string, PotatoJobFiles | null> = structuredClone(seed.jobFiles ?? {});
  const threads = new Map<string, PotatoMessage[]>(Object.entries(seed.messages ?? {}).map(([id, list]) => [id, list.map((m) => ({ ...m }))]));
  const listeners = new Set<(botId: string | null) => void>();
  const calls: FakePotatoTransport['calls'] = [];
  const log = <T>(method: keyof PotatoTransport, args: unknown[], run: () => T): Promise<T> => {
    calls.push({ method, args });
    try {
      return Promise.resolve(run());
    } catch (error) {
      return Promise.reject(error);
    }
  };
  const find = (id: string): PotatoBotView => {
    const bot = state.bots.find((b) => b.id === id);
    if (!bot) throw new Error('No bot has that name.');
    return bot;
  };
  const emit = (botId: string | null = null) => listeners.forEach((cb) => cb(botId));
  const thread = (id: string): PotatoMessage[] => {
    let list = threads.get(id);
    if (!list) threads.set(id, (list = []));
    return list;
  };
  const view = (): PotatoState => ({ ...state, bots: state.bots.map((b) => ({ ...b })) });
  const message = (from: PotatoMessage['from'], kind: MessageKind, text: string): PotatoMessage => ({ id: `m-fake-${nextId++}`, at: Date.now(), from, kind, text });

  return {
    calls,
    artifacts: fakeArtifactTransport(seed.artifacts),
    emit,
    patchBot(botId, patch) {
      Object.assign(find(botId), patch);
      emit(botId);
    },
    setEnabled(on) {
      state.enabled = on;
      emit(null);
    },
    addMessage(botId, m) {
      thread(botId).push({ ...m });
      emit(botId);
    },
    state: () => log('state', [], view),
    messages: (botId, limit) => log('messages', [botId, limit], () => thread(botId).slice(-(limit ?? 200)).map((m) => ({ ...m }))),
    send: (botId, text) =>
      log('send', [botId, text], () => {
        const bot = find(botId);
        const saved = message({ kind: 'you' }, 'text', text.trim());
        thread(botId).push(saved);
        bot.lastLine = saved.text;
        bot.lastAt = saved.at;
        emit(botId);
        return { id: saved.id, state: bot.brainState, delivered: true, ...seed.outcome };
      }),
    addBot: (botId) =>
      log('addBot', [botId], () => {
        if (!state.bots.some((b) => b.id === botId)) {
          state.bots.push({
            id: botId, name: moodWord(botId), talkPersona: state.settings.defaultPersona, brainState: 'idle', lastError: null, typing: false, mood: null, provider: null,
            unread: 0, lastLine: null, lastAt: null, usedTodayNano: 0, limitCents: state.settings.dailyLimitCents, limitOff: state.settings.dailyLimitOff ?? false, openCards: 0,
          });
        }
        emit(botId);
        return { id: botId, talkPersona: find(botId).talkPersona, createdMs: Date.now() };
      }),
    setPersona: (botId, pack) =>
      log('setPersona', [botId, pack], () => {
        const bot = find(botId);
        if (bot.talkPersona !== pack) {
          bot.talkPersona = pack;
          thread(botId).push(message({ kind: 'note' }, 'note', `Now talking as ${PERSONA_LABEL[pack]}.`));
        }
        emit(botId);
        return { id: botId, talkPersona: pack, createdMs: 0 };
      }),
    startBot: (botId) =>
      log('startBot', [botId], () => {
        find(botId).brainState = 'ready';
        emit(botId);
      }),
    stopBot: (botId) =>
      log('stopBot', [botId], () => {
        find(botId).brainState = 'idle';
        emit(botId);
      }),
    markRead: (botId) =>
      log('markRead', [botId], () => {
        find(botId).unread = 0;
      }),
    toggleReaction: (botId, messageId, emoji) =>
      log('toggleReaction', [botId, messageId, emoji], () => {
        if (!(REACTIONS as readonly string[]).includes(emoji)) throw new Error('That reaction is not one of the choices.');
        const target = thread(botId).find((m) => m.id === messageId);
        if (!target) throw new Error('That message is not in this chat any more.');
        const has = (target.reactions ?? []).includes(emoji);
        const next = REACTIONS.filter((e) => ((target.reactions ?? []).includes(e) ? !(has && e === emoji) : !has && e === emoji));
        target.reactions = next.length > 0 ? [...next] : undefined;
        emit(botId);
        return [...next];
      }),
    undoSetting: (botId, changeId) =>
      log('undoSetting', [botId, changeId], () => {
        const target = thread(botId).find((m) => m.change?.id === changeId);
        if (!target?.change) throw new Error('POTATO_SETTING_CHANGE_GONE: That change is not in the record any more.');
        if (target.change.undone) throw new Error('POTATO_SETTING_UNDONE: That change was already undone.');
        target.change = { ...target.change, undone: true };
        emit(botId);
      }),
    setArchived: (botId, archived) =>
      log('setArchived', [botId, archived], () => {
        const bot = find(botId);
        bot.archived = archived;
        if (archived) bot.brainState = 'idle';
        emit(botId);
      }),
    helperFolders: (botId) => log('helperFolders', [botId], () => [...(seed.folders ?? ['demo-a', 'demo-b'])]),
    helperStart: (botId, requestId, folder) =>
      log('helperStart', [botId, requestId, folder], () => {
        const card = thread(botId).find((m) => m.kind === 'helper_card' && m.requestId === requestId);
        const chosen = folder ?? card?.project;
        if (!chosen) throw new Error('Choose a folder for the helper first.');
        thread(botId).push({ ...message({ kind: 'note' }, 'helper_result', `Started a helper in ${chosen}.`), requestId });
        emit(botId);
        return { folder: chosen };
      }),
    helperDecline: (botId, requestId) =>
      log('helperDecline', [botId, requestId], () => {
        thread(botId).push({ ...message({ kind: 'note' }, 'helper_result', 'Not now.'), requestId });
        emit(botId);
      }),
    answerCard: (botId, cardId, answer) =>
      log('answerCard', [botId, cardId, answer], () => {
        const card = thread(botId).find((m) => m.kind === 'decision_card' && m.requestId === cardId);
        if (!card) throw new Error('POTATO_CARD_NONE: There is no such card.');
        if (thread(botId).some((m) => m.kind === 'decision_result' && m.requestId === cardId)) throw new Error('POTATO_CARD_ANSWERED: That card was already answered.');
        if (card.card?.login && answer.trim() === '0') throw new Error('POTATO_CARD_BAD_ANSWER: Sign in works only from the Sign in button, which fills the page. Nothing was answered.');
        if (card.card?.kind !== 'question' && !['allow_once', 'allow_job', 'deny'].includes(answer.trim())) throw new Error("POTATO_CARD_BAD_ANSWER: That is not one of this card's answers.");
        const wire = card.card?.kind === 'question' ? `choice:${answer}` : answer;
        thread(botId).push({ ...message({ kind: 'note' }, 'decision_result', 'Answered.'), requestId: cardId, card: { kind: card.card?.kind ?? 'approval', why: '', choices: [], ...(card.card?.login ? { login: card.card.login } : {}), answer: wire } });
        emit(botId);
        return { cardId, answer: wire, jobResumed: false, turnStarted: true };
      }),
    answerCardAlways: (botId, cardId) =>
      log('answerCardAlways', [botId, cardId], () => {
        const card = thread(botId).find((m) => m.kind === 'decision_card' && m.requestId === cardId);
        if (!card) throw new Error('POTATO_CARD_NONE: There is no such card.');
        if (thread(botId).some((m) => m.kind === 'decision_result' && m.requestId === cardId)) throw new Error('POTATO_CARD_ANSWERED: That card was already answered.');
        if (card.card?.kind !== 'approval' || !card.card.canAlwaysAllow) throw new Error('POTATO_CARD_BAD_ANSWER: Always allow is not offered for this kind of command. Nothing was answered.');
        thread(botId).push({ ...message({ kind: 'note' }, 'decision_result', 'Always allowed.'), requestId: cardId, card: { kind: 'approval', why: '', choices: [], answer: 'always_allow' } });
        emit(botId);
        return { cardId, answer: 'always_allow', jobResumed: false, turnStarted: true };
      }),
    // The password is never written into the call log: only that there was one.
    submitLogin: (botId, cardId, username, _password) =>
      log('submitLogin', [botId, cardId, username, '[hidden]'], () => {
        const card = thread(botId).find((m) => m.kind === 'decision_card' && m.requestId === cardId);
        if (!card?.card?.login) throw new Error('POTATO_CARD_BAD_ANSWER: That is not a sign-in card.');
        if (seed.login?.signInError) throw new Error(seed.login.signInError);
        thread(botId).push({ ...message({ kind: 'note' }, 'decision_result', 'Signed in.'), requestId: cardId, card: { kind: 'question', why: '', choices: [], login: card.card.login, answer: 'choice:0' } });
        emit(botId);
        return { cardId, answer: 'choice:0', jobResumed: false, turnStarted: true };
      }),
    confirmLogin: (botId, cardId) =>
      log('confirmLogin', [botId, cardId], () => {
        const card = thread(botId).find((m) => m.kind === 'decision_card' && m.requestId === cardId);
        if (!card?.card?.login) throw new Error('POTATO_CARD_BAD_ANSWER: That is not a sign-in card.');
        if (thread(botId).some((m) => m.kind === 'decision_result' && m.requestId === cardId)) throw new Error('POTATO_CARD_ANSWERED: That card was already answered.');
        thread(botId).push({ ...message({ kind: 'note' }, 'decision_result', 'Signed in.'), requestId: cardId, card: { kind: 'question', why: '', choices: [], login: card.card.login, answer: 'choice:0' } });
        emit(botId);
        return { cardId, answer: 'choice:0', jobResumed: false, turnStarted: true };
      }),
    loginOrigin: (botId, cardId) =>
      log('loginOrigin', [botId, cardId], () => {
        const site = thread(botId).find((m) => m.kind === 'decision_card' && m.requestId === cardId)?.card?.login ?? '';
        const origin = seed.login?.origin ?? null;
        return { site, origin, matches: origin !== null && origin === site };
      }),
    forgetSite: (botId, cardId) => log('forgetSite', [botId, cardId], () => ({ cookiesCleared: 2 })),
    openCards: (botId) => log('openCards', [botId], () => []),
    job: (botId) => log('job', [botId], () => seed.jobs?.[botId] ?? null),
    jobFiles: (botId) => log('jobFiles', [botId], () => structuredClone(files[botId] ?? null)),
    restoreJobFile: (botId, jobId, fileId) =>
      log('restoreJobFile', [botId, jobId, fileId], () => {
        const list = files[botId];
        const file = list && list.jobId === jobId ? list.files.find((f) => f.id === fileId) : undefined;
        if (!file) throw new Error('POTATO_FILE_UNDO_NOT_FOUND: That file is not in the list any more.');
        if (file.state === 'too_big' || file.state === 'over_limit') throw new Error('POTATO_FILE_UNDO_NOT_SAVED: The old content of that file was not kept, so it cannot be restored.');
        file.restored = true;
        emit(botId);
        return { ...file };
      }),
    installNode: () => log('installNode', [], () => undefined),
    settingsGet: () => log('settingsGet', [], () => ({ ...state.settings })),
    settingsSet: (settings) =>
      log('settingsSet', [settings], () => {
        state.settings = { ...settings, dailyLimitCents: Math.min(2000, Math.max(5, Math.round(settings.dailyLimitCents))) };
        emit(null);
        return { ...state.settings };
      }),
    checkKey: () => log('checkKey', [], () => (seed.keyCheck ? seed.keyCheck() : 'Works.')),
    onChanged: (cb) => {
      listeners.add(cb);
      return Promise.resolve((() => listeners.delete(cb)) as UnlistenFn);
    },
  };
}
