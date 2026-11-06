// Routines of Potato Bot (phase P9a): a prompt a bot runs on a schedule. The names mirror
// src-tauri/src/potato/routines.rs and commands/potato_routines.rs in camelCase. Desktop only. Nothing here draws.
// `fakeRoutinesTransport` is the in-memory twin for tests (the pattern of `fakePotatoTransport`).
import { invoke } from '@tauri-apps/api/core';
import { isTauri } from './cockpit-api';
import type { ListStatus } from './potato-list';

/** When a routine runs (`RoutineSchedule` in routines.rs). Times are the computer's local clock. */
export type RoutineSchedule =
  | { kind: 'hourly' }
  | { kind: 'daily'; time: string }
  | { kind: 'weekdays'; time: string }
  | { kind: 'everyMinutes'; minutes: number }
  | { kind: 'once'; at: string }
  /** A standard 5-field cron (minute hour day month weekday), local clock, runs at least 15 minutes apart. */
  | { kind: 'cron'; expr: string };

/** "Only when something changes": the check a watching routine runs on its schedule (a read-only command or a web address). */
export interface WatchInput {
  check: string;
  /** Web address only: words the page must also hold. */
  contains?: string;
}

/** What the page shows of a watching routine. */
export interface WatchView {
  check: string;
  kind: 'command' | 'url';
  contains: string | null;
  lastCheckedMs: number | null;
}

export interface RoutineInput {
  title: string;
  prompt: string;
  schedule: RoutineSchedule;
  /** Run only when this check's result changes. Absent: the routine runs at every time on its schedule. */
  watch?: WatchInput;
}

export type RoutineStatus = 'active' | 'paused' | 'done';

/** One routine as the page shows it. `summary` is the schedule in words, made by the Rust side. */
export interface RoutineView {
  id: string;
  bot: string;
  title: string;
  prompt: string;
  schedule: RoutineSchedule;
  summary: string;
  status: RoutineStatus;
  paused: boolean;
  done: boolean;
  lastRunMs: number | null;
  nextRunMs: number | null;
  /** A watching routine's check. Absent or null: it runs at every time on its schedule. */
  watch?: WatchView | null;
}

/** One run in a routine's history (`RunView` in routines.rs). `text` is a plain sentence. */
export interface RoutineRunView {
  routineId: string;
  atMs: number;
  result: 'ok' | 'failed' | 'skipped';
  text: string;
  /** The bot's reply message in the thread, once there is one. */
  replyId: string | null;
}

/** What "Run now" answers: the brain got the prompt (or the note that it could not). */
export interface RoutineRunOutcome {
  id: string;
  delivered: boolean;
}

/** One open wait of a bot (`WaitView` in waits.rs): the bot ended its turn and the app looks on a timer, with no model. */
export interface WaitView {
  id: string;
  bot: string;
  /** What the bot is waiting for in its own words, or the check itself. */
  title: string;
  /** The read-only command or the web address that is looked at. */
  check: string;
  kind: 'command' | 'url';
  /** "5 minutes". */
  everyWords: string;
  /** Looks so far. */
  checks: number;
  nextCheckMs: number;
  deadlineMs: number;
  /** The start of what the last look found. */
  last: string;
}

export interface RoutinesTransport {
  /** The bot's open waits (`wait_until`), soonest look first. */
  waits(botId: string): Promise<WaitView[]>;
  /** Gives up a wait. The bot is not woken. */
  cancelWait(waitId: string): Promise<void>;
  list(botId: string): Promise<RoutineView[]>;
  create(botId: string, input: RoutineInput): Promise<RoutineView>;
  pause(routineId: string, paused: boolean): Promise<RoutineView>;
  /** Moves the routine to Recently deleted; resolves with the trash id (for Undo). */
  remove(routineId: string): Promise<string>;
  runNow(routineId: string): Promise<RoutineRunOutcome>;
  /** The next few runs (instants, ms) of a cron, worked out by the Rust side. Rejects with the plain reason when the cron would not be saved. */
  cronPreview(expr: string): Promise<number[]>;
  /** The last runs (10 per routine) of this bot's routines, newest first. */
  runs(botId: string): Promise<RoutineRunView[]>;
}

/** Outside the desktop app (a browser preview, the tests) there are no routines to list, and nothing can change. */
const NOT_ON_DESKTOP = 'Routines need the Puppet Master desktop app.';

export const tauriRoutinesTransport: RoutinesTransport = {
  waits: (botId) => (isTauri() ? invoke<WaitView[]>('potato_waits_list', { botId }) : Promise.resolve([])),
  cancelWait: (waitId) => (isTauri() ? invoke<void>('potato_wait_cancel', { waitId }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  list: (botId) => (isTauri() ? invoke<RoutineView[]>('potato_routines_list', { botId }) : Promise.resolve([])),
  create: (botId, input) => (isTauri() ? invoke<RoutineView>('potato_routine_create', { botId, input }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  pause: (routineId, paused) => (isTauri() ? invoke<RoutineView>('potato_routine_pause', { routineId, paused }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  remove: (routineId) => (isTauri() ? invoke<string>('potato_routine_delete', { routineId }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  runNow: (routineId) => (isTauri() ? invoke<RoutineRunOutcome>('potato_routine_run_now', { routineId }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  runs: (botId) => (isTauri() ? invoke<RoutineRunView[]>('potato_routine_runs', { botId }) : Promise.resolve([])),
  cronPreview: (expr) => (isTauri() ? invoke<number[]>('potato_routine_cron_preview', { expr, count: CRON_PREVIEW_COUNT }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
};

/** How many coming runs the cron box shows. */
export const CRON_PREVIEW_COUNT = 3;

// ---- the form -----------------------------------------------------------------------------------

/** The choices of the "New routine" form. `every` is "Every few minutes". */
export type RoutineFreq = 'hourly' | 'daily' | 'weekdays' | 'every' | 'once' | 'cron';

export interface RoutineFormValues {
  title: string;
  prompt: string;
  freq: RoutineFreq;
  /** "HH:MM" for daily and weekdays. */
  time: string;
  /** For every few minutes: 15 or more. */
  minutes: number;
  /** "YYYY-MM-DDTHH:MM" for once (the value of a datetime-local input). */
  at: string;
  /** For cron: the five parts, such as "0 9 * * mon-fri". */
  cron: string;
  /** "Run only when something changes" is ticked. */
  watchOn: boolean;
  /** The command or the web address to watch. */
  watchCheck: string;
  /** For a web address: words the page must also hold. */
  watchContains: string;
}

export const DEFAULT_ROUTINE_FORM: RoutineFormValues = {
  title: '',
  prompt: '',
  freq: 'weekdays',
  time: '09:00',
  minutes: 30,
  at: '',
  cron: '',
  watchOn: false,
  watchCheck: '',
  watchContains: '',
};

/** Does the watch box hold a web address (then the "page must hold" box is offered)? */
export function watchIsAddress(check: string): boolean {
  return /^https?:\/\//i.test(check.trim());
}

/** The request for the form's values, or one plain sentence that says what is missing. */
export function buildRoutineInput(values: RoutineFormValues): RoutineInput | string {
  const built = buildSchedule(values);
  if (typeof built === 'string' || !values.watchOn) return built;
  const check = values.watchCheck.trim();
  if (!check) return ROUTINE_COPY.watchMissing;
  const contains = watchIsAddress(check) ? values.watchContains.trim() : '';
  return { ...built, watch: { check, ...(contains ? { contains } : {}) } };
}

function buildSchedule(values: RoutineFormValues): RoutineInput | string {
  const title = values.title.trim();
  const prompt = values.prompt.trim();
  if (!title) return 'Give the routine a name.';
  if (!prompt) return 'Write what the bot should do.';
  switch (values.freq) {
    case 'hourly':
      return { title, prompt, schedule: { kind: 'hourly' } };
    case 'daily':
    case 'weekdays':
      if (!/^\d{2}:\d{2}$/.test(values.time)) return 'Pick a time.';
      return { title, prompt, schedule: { kind: values.freq, time: values.time } };
    case 'every':
      if (!Number.isInteger(values.minutes) || values.minutes < 15) return 'Pick every 15 minutes or more.';
      return { title, prompt, schedule: { kind: 'everyMinutes', minutes: values.minutes } };
    case 'once':
      if (!values.at) return 'Pick a date and time.';
      return { title, prompt, schedule: { kind: 'once', at: values.at } };
    case 'cron': {
      const expr = values.cron.trim().split(/\s+/).filter(Boolean).join(' ');
      if (!expr) return ROUTINE_COPY.cronMissing;
      return { title, prompt, schedule: { kind: 'cron', expr } };
    }
  }
}

// ---- copy ---------------------------------------------------------------------------------------

/** "today at 9:00 AM", "tomorrow at 6:00 AM", "Nov 3 at 9:00 AM". */
export function nextRunWords(at: number, now: number): string {
  const when = new Date(at);
  const today = new Date(now);
  const tomorrow = new Date(now + 24 * 60 * 60 * 1000);
  const time = when.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  if (when.toDateString() === today.toDateString()) return `today at ${time}`;
  if (when.toDateString() === tomorrow.toDateString()) return `tomorrow at ${time}`;
  return `${when.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })} at ${time}`;
}

/** "Mon, Nov 9 at 9:00 AM": one coming cron run, with the weekday because crons often name one. */
export function cronRunWords(at: number): string {
  const when = new Date(at);
  const day = when.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });
  const time = when.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  return `${day} at ${time}`;
}

/** "Nov 2, 6:00 AM": when a run happened. */
export function runWhenWords(at: number): string {
  const when = new Date(at);
  return `${when.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${when.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
}

/** The second line of a row: what happens next, or that it is paused or finished. */
export function routineStatusLine(routine: RoutineView, now: number): string {
  if (routine.status === 'done') return 'Ran once. Done.';
  if (routine.status === 'paused') return 'Paused';
  if (routine.nextRunMs == null) return 'No next run yet';
  return `Next run ${nextRunWords(routine.nextRunMs, now)}`;
}

/** "in 4 minutes", "in 2 hours", "any moment": when the next look is. */
export function nextLookWords(at: number, now: number): string {
  const seconds = Math.round((at - now) / 1000);
  if (seconds <= 45) return 'any moment';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `in ${minutes} minute${minutes === 1 ? '' : 's'}`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `in ${hours} hour${hours === 1 ? '' : 's'}`;
  const days = Math.round(hours / 24);
  return `in ${days} days`;
}

export const WAIT_COPY = {
  title: 'Waiting for',
  line: 'These look on a timer with no cost. The bot starts again when one is done or runs out of time. A wait keeps the computer awake.',
  cancel: 'Cancel',
  cancelFailed: 'That wait could not be cancelled.',
  looks: (count: number, every: string) => `${count === 0 ? 'Not looked at yet' : `Looked ${count} time${count === 1 ? '' : 's'}`} · every ${every}`,
  next: (when: string) => `next look ${when}`,
  until: (when: string) => `gives up ${when}`,
} as const;

export const ROUTINE_COPY = {
  title: 'Routines',
  empty: 'No routines yet. A routine is a job this bot runs on a schedule.',
  newRoutine: 'New routine',
  runNow: 'Run now',
  delete: 'Delete',
  confirmDelete: 'Delete this routine?',
  keep: 'Keep it',
  saving: 'Saving...',
  save: 'Save routine',
  cancel: 'Cancel',
  watchLabel: 'Run only when something changes',
  watchHint: 'Every time it is due, the app looks at the check below with no cost. The bot starts only when the result is different from the last look. The first look just remembers what it saw.',
  watchCheckLabel: 'What to watch',
  watchPlaceholder: 'ls /home/pm/inbox | wc -l   or   https://example.com/status',
  watchCheckHint: 'A command that only looks (ls, cat, grep, stat, git status ...) or a web address. Nothing that changes things.',
  watchContainsLabel: 'Page must also say',
  watchPromptLabel: 'What to do when it changes',
  watchMissing: 'Write what to watch: a command that only looks, or a web address.',
  watching: (check: string) => `Watching: ${check}`,
  lastLook: (when: string) => `last look ${when}`,
  notLooked: 'not looked at yet',
  cronMissing: 'Type a cron like 0 9 * * mon-fri (minute, hour, day, month, weekday).',
  cronLabel: 'Cron',
  cronHint: 'Five parts: minute, hour, day of the month, month, day of the week. Local time, at least 15 minutes apart.',
  cronNext: 'Next runs',
  history: 'History',
  showReply: 'Show reply',
  runResult: { ok: 'OK', failed: 'Failed', skipped: 'Skipped' } as const,
  sent: (name: string) => `Sent to ${name}.`,
  loadFailed: 'Could not load the routines.',
  saveFailed: 'Could not save the routine.',
  actionFailed: 'That did not work. Try again.',
} as const;

// ---- the bot list's second line ------------------------------------------------------------------

/** The grey line under a bot's name in the list: what the bot waits for, does, or does next. */
export interface ListNextLine {
  /** `needs` gets the one accent dot; the rest are grey. */
  tone: 'needs' | 'working' | 'quiet';
  text: string;
}

export const LIST_NEXT_COPY = {
  waiting: 'Waiting for you',
  thinking: 'Thinking…',
  paused: 'Paused',
  next: (name: string, when: string) => `Next: ${name} · ${when}`,
} as const;

/**
 * The time a row shows for the next run: `9:00` today, `Tomorrow 9:00`, else the weekday `Tue 9:00`. The clock and the
 * weekday follow the locale (default: the person's).
 */
export function nextRunShort(at: number, now: number, locale?: string): string {
  const when = new Date(at);
  const today = new Date(now);
  const tomorrow = new Date(now + 24 * 60 * 60 * 1000);
  const time = when.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' });
  if (when.toDateString() === today.toDateString()) return time;
  if (when.toDateString() === tomorrow.toDateString()) return `Tomorrow ${time}`;
  return `${when.toLocaleDateString(locale, { weekday: 'short' })} ${time}`;
}

/** The active routine that runs next, or null. Paused and finished routines do not count. */
export function nextRoutine(routines: readonly RoutineView[]): { title: string; at: number } | null {
  let best: { title: string; at: number } | null = null;
  for (const r of routines) {
    if (r.status !== 'active' || r.nextRunMs == null) continue;
    if (!best || r.nextRunMs < best.at) best = { title: r.title, at: r.nextRunMs };
  }
  return best;
}

/**
 * The second line of a row, by priority: waiting on the person, working, paused, then the next routine. Null means no
 * line: the row keeps its preview. Errors are not here (the row shows its own pill). "Using the computer" has no per-bot
 * signal on the client yet, so it is not chosen.
 */
export function listNextLine(status: ListStatus | null, routines: readonly RoutineView[], now: number, locale?: string): ListNextLine | null {
  if (status === 'needs') return { tone: 'needs', text: LIST_NEXT_COPY.waiting };
  if (status === 'working') return { tone: 'working', text: LIST_NEXT_COPY.thinking };
  if (status === 'paused') return { tone: 'quiet', text: LIST_NEXT_COPY.paused };
  if (status === 'error') return null;
  const next = nextRoutine(routines);
  if (!next) return null;
  return { tone: 'quiet', text: LIST_NEXT_COPY.next(next.title, nextRunShort(next.at, now, locale)) };
}

// ---- the fake -----------------------------------------------------------------------------------

export interface FakeRoutinesTransport extends RoutinesTransport {
  readonly calls: { method: keyof RoutinesTransport; args: unknown[] }[];
  /** The rows the fake holds, after the calls so far. */
  readonly routines: RoutineView[];
}

/** An in-memory `RoutinesTransport`. `fail` makes every call reject with that message. */
export function fakeRoutinesTransport(seed: RoutineView[] = [], fail?: string, opts: { cronPreview?: (expr: string) => number[]; runs?: RoutineRunView[]; waits?: WaitView[] } = {}): FakeRoutinesTransport {
  const routines = [...seed];
  const waitsOpen = [...(opts.waits ?? [])];
  const calls: FakeRoutinesTransport['calls'] = [];
  let nextId = 1;
  const run = <T,>(method: keyof RoutinesTransport, args: unknown[], work: () => T): Promise<T> => {
    calls.push({ method, args });
    if (fail) return Promise.reject(new Error(fail));
    return Promise.resolve().then(work);
  };
  const find = (id: string) => {
    const routine = routines.find((r) => r.id === id);
    if (!routine) throw new Error('No routine has that name.');
    return routine;
  };
  return {
    calls,
    routines,
    list: (botId) => run('list', [botId], () => routines.filter((r) => r.bot === botId).map((r) => ({ ...r }))),
    create: (botId, input) =>
      run('create', [botId, input], () => {
        const made: RoutineView = {
          id: `r-fake-${nextId++}`,
          bot: botId,
          title: input.title,
          prompt: input.prompt,
          schedule: input.schedule,
          summary: input.schedule.kind,
          status: 'active',
          paused: false,
          done: false,
          lastRunMs: null,
          nextRunMs: null,
          watch: input.watch ? { check: input.watch.check, kind: /^https?:/i.test(input.watch.check) ? 'url' : 'command', contains: input.watch.contains ?? null, lastCheckedMs: null } : null,
        };
        routines.push(made);
        return { ...made };
      }),
    pause: (routineId, paused) =>
      run('pause', [routineId, paused], () => {
        const routine = find(routineId);
        routine.paused = paused;
        routine.status = paused ? 'paused' : 'active';
        routine.nextRunMs = paused ? null : (routine.nextRunMs ?? Date.now() + 60 * 60 * 1000);
        return { ...routine };
      }),
    remove: (routineId) =>
      run('remove', [routineId], () => {
        const index = routines.findIndex((r) => r.id === routineId);
        if (index >= 0) routines.splice(index, 1);
        return `trash-${routineId}`;
      }),
    runNow: (routineId) =>
      run('runNow', [routineId], () => {
        find(routineId);
        return { id: routineId, delivered: true };
      }),
    runs: (botId) => run('runs', [botId], () => (opts.runs ?? []).map((r) => ({ ...r }))),
    waits: (botId) => run('waits', [botId], () => waitsOpen.filter((w) => w.bot === botId).map((w) => ({ ...w }))),
    cancelWait: (waitId) =>
      run('cancelWait', [waitId], () => {
        const at = waitsOpen.findIndex((w) => w.id === waitId);
        if (at < 0) throw new Error('There is no open wait with that id.');
        waitsOpen.splice(at, 1);
      }),
    cronPreview: (expr) => run('cronPreview', [expr], () => (opts.cronPreview ? opts.cronPreview(expr) : [])),
  };
}
