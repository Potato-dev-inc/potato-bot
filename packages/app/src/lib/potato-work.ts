// The live step list of a turn ("Looked at the screen", "Model stopped answering, retrying"): the shapes the app sends
// while a bot works, and the words and times the thread shows for them. The words come from `toolLabel`, so a step reads
// the same live and once it is saved as a tool line ("Used the mouse and keyboard"). No arguments or results ever
// reach these shapes: the brain sends the tool's name and one short, redacted line.
import { toolLabel } from './potato-thread';

export type WorkStatus = 'pending' | 'running' | 'done' | 'error' | 'retry';

export interface WorkStepView {
  id: string;
  tool: string;
  status: WorkStatus;
  startedMs: number;
  endedMs: number | null;
  /** At most three short lines, oldest first. */
  log: string[];
  /** A plain one-line description of what the step did ("Ran: git status"). Absent: the tool's own words are shown. */
  detail?: string;
  /** The full command or path, shown when the row is opened. */
  full?: string;
  /** The model's own short reason for the call, when it gave one. */
  why?: string;
  /** The last lines of the output, once the step ended. */
  tail?: string;
  /** The error line of a step that failed. */
  error?: string;
  exit?: number;
}

export interface WorkRunView {
  startedMs: number;
  steps: WorkStepView[];
  /** The turn failed: the X sits on the step it stopped at. */
  failed: boolean;
  /** When the turn ended well. Null while it runs. */
  endedMs?: number | null;
  /** The bot has a running job: the next turn starts by itself, so this turn did not end the work. */
  continues?: boolean;
}

/** A bot's list in a group. */
export interface RoomWorkView {
  bot: string;
  run: WorkRunView;
}

export const WORK_COPY = {
  retry: 'Model stopped answering, retrying',
  thinking: 'Thinking',
  steps: 'Steps',
  show: 'Show steps',
  hide: 'Hide steps',
  stoppedAt: (label: string) => `Stopped at: ${label}`,
  errorGoesOn: 'Hit an error. The job goes on by itself',
  jobContinues: 'the job goes on',
  earlier: (n: number) => (n === 1 ? 'Show 1 earlier step' : `Show ${n} earlier steps`),
  earlierHide: 'Hide earlier steps',
  exit: (code: number) => `Exit ${code}`,
  worked: (time: string, n: number) => `Worked for ${time} · ${n} ${n === 1 ? 'step' : 'steps'}`,
};

/** The words for one step. */
export function workLabel(step: Pick<WorkStepView, 'tool' | 'status'>): string {
  return step.tool === 'retry' || step.status === 'retry' ? WORK_COPY.retry : toolLabel(step.tool);
}

/** What a row says: the plain description of what the step did, else the tool's own words. */
export function stepText(step: Pick<WorkStepView, 'tool' | 'status' | 'detail'>): string {
  return step.status === 'retry' || step.tool === 'retry' ? WORK_COPY.retry : (step.detail ?? toolLabel(step.tool));
}

/** True when opening the row shows more than its line. */
export function hasMore(step: WorkStepView): boolean {
  return !!(step.full || step.tail || step.error || step.exit !== undefined);
}

/** Rows kept in view while the list is open; older finished ones fold behind a button. */
export const KEEP_ROWS = 5;

/** Splits a long list: the older rows (folded by default) and the rows that stay in view. Running steps always stay. */
export function foldLong(steps: readonly WorkStepView[]): { hidden: WorkStepView[]; shown: WorkStepView[] } {
  if (steps.length <= KEEP_ROWS + 1) return { hidden: [], shown: [...steps] };
  const cut = steps.length - KEEP_ROWS;
  const hidden = steps.slice(0, cut).filter((s) => s.status === 'done' || s.status === 'error');
  const hiddenIds = new Set(hidden.map((s) => s.id + ':' + s.startedMs));
  return { hidden, shown: steps.filter((s) => !hiddenIds.has(s.id + ':' + s.startedMs)) };
}

/** "8s", "1m 20s", "2h 5m". Under a second is "<1s". */
export function elapsedLabel(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  if (total < 1) return '<1s';
  if (total < 60) return `${total}s`;
  const m = Math.floor(total / 60);
  if (m < 60) return `${m}m ${total % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** The step to name in the header: the one running, else the one that stopped, else the last. */
export function headStep(run: WorkRunView): WorkStepView | null {
  const running = [...run.steps].reverse().find((s) => s.status === 'running' || s.status === 'retry');
  if (running) return running;
  const failed = [...run.steps].reverse().find((s) => s.status === 'error');
  return failed ?? run.steps[run.steps.length - 1] ?? null;
}

/** A run is settled when the turn ended, well or not. */
export const isSettled = (run: WorkRunView): boolean => run.failed || (run.endedMs ?? null) !== null;

/** How long a finished step took, or null while it runs. */
export function stepTime(step: WorkStepView): string | null {
  return step.endedMs === null ? null : elapsedLabel(step.endedMs - step.startedMs);
}

/**
 * The header of the list. "Stopped at" only when the turn really failed and nothing goes on after it; a failed step in the
 * middle of a turn that is still running is just a row with an X, and the header keeps saying what is happening now.
 */
export function workTitle(run: WorkRunView, nowMs: number): string {
  const head = headStep(run);
  if (!head) return WORK_COPY.thinking;
  const live = [...run.steps].reverse().find((s) => s.status === 'running' || s.status === 'retry');
  if (run.failed) {
    const at = [...run.steps].reverse().find((s) => s.status === 'error') ?? head;
    return run.continues ? WORK_COPY.errorGoesOn : WORK_COPY.stoppedAt(stepText(at));
  }
  if (isSettled(run)) {
    const worked = WORK_COPY.worked(elapsedLabel((run.endedMs ?? head.endedMs ?? nowMs) - run.startedMs), run.steps.length);
    return run.continues ? `${worked} \u00b7 ${WORK_COPY.jobContinues}` : worked;
  }
  return live ? stepText(live) : WORK_COPY.thinking;
}

/** The line a screen reader hears when the step changes. */
export function announce(run: WorkRunView, owner?: string): string {
  if (!headStep(run)) return '';
  return `${owner ? `${owner}: ` : ''}${workTitle(run, run.endedMs ?? run.startedMs)}`;
}
