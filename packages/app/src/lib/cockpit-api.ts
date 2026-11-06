// Typed wrappers for exactly the commands and events in docs/design/cockpit-contract.md.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { UserGesture } from './user-gesture';

export type UiMode = 'orchestration' | 'individual';

export interface PromptChoice {
  id: string;
  label: string;
  /** "Don't ask again" / "allow all" style answer (set by the backend; older builds omit it). */
  broad?: boolean;
}

export type RiskLevel = 'low' | 'medium' | 'high';

/** Plain-language risk of what saying yes would do (backend `prompt_risk`). */
export interface PromptRisk {
  level: RiskLevel;
  headline: string;
  reasons: string[];
}

export interface PanePrompt {
  promptId: string;
  kind: string;
  text: string;
  summary?: string;
  choices: PromptChoice[];
  /** Absent on older backends: the UI then shows the plain question. */
  risk?: PromptRisk;
}

export interface PaneRunInfo {
  handle: string;
  status: string;
  receipt: unknown | null;
  /** Why the run failed: the typed code (and the backend's message). Absent while it did not fail. */
  error?: { code: string; message?: string } | null;
}

export interface PaneTranscript {
  chunks: { stream: string; text: string }[];
}

export type SpawnStage = 'claiming' | 'spawning' | 'attached' | 'ready' | 'failed';

export interface SpawnStageEvent {
  agent_id: string;
  pane_id: string | null;
  stage: SpawnStage;
  warm: boolean;
  message?: string;
}

import type { Question } from './questions';

export interface Checkpoint {
  id: string;
  commit: string;
  paneId: string;
  turn: number;
  kind: 'before' | 'after' | 'answer' | 'undo' | string;
  label: string;
  note: string;
  atMs: number;
  files: number;
  added: number;
  deleted: number;
}

export interface CheckpointList {
  state: 'ok' | 'off' | 'no_git' | 'too_big';
  message: string;
  /** Newest first. */
  items: Checkpoint[];
}

export interface FileChange {
  path: string;
  status: 'A' | 'M' | 'D' | string;
  added: number;
  deleted: number;
  binary: boolean;
}

export interface RestoreReport {
  safetyId: string | null;
  written: number;
  removed: number;
  failed: string[];
}

export interface CheckpointsApi {
  checkpointsList: (paneId: string) => Promise<CheckpointList>;
  checkpointDiff: (paneId: string, id: string) => Promise<FileChange[]>;
  checkpointRestore: (paneId: string, id: string) => Promise<RestoreReport>;
  checkpointUndoLast: (paneId: string) => Promise<RestoreReport>;
}

export interface TurnFileStats {
  count: number;
  added: number;
  deleted: number;
  paths: string[];
}

export interface TurnApproval {
  atMs: number;
  promptId: string;
  prompt: string;
  kind: string;
  decision: "allow" | "deny" | "answer" | string;
}

/** What one agent turn did, built without an LLM (Rust: turn_summary). */
export interface TurnSummary {
  paneId: string;
  agent: string;
  turn: number;
  startedAtMs: number;
  endedAtMs: number;
  durationMs: number;
  /** null = unknown (no checkpoints for this pane). */
  files: TurnFileStats | null;
  approvals: TurnApproval[];
  end: { kind: "idle" | "waiting_input" | "error" | string; prompt?: string };
  receipt?: { checks: string; outcome: string };
  /** One plain sentence, ready to show. */
  sentence: string;
}

export interface DigestLine {
  paneId: string;
  agent: string;
  turns: number;
  sentence: string;
  endedAtMs: number;
  end: "idle" | "waiting_input" | "error" | string;
}

export interface NeedsYou {
  /** The attention item this line is (a stable key); absent from older builds. */
  id?: string;
  /** Empty for something that has no pane (a question with no pane, a consent card). */
  paneId: string;
  agent: string;
  status: "waiting_input" | "error" | string;
  prompt?: string;
}

/** "While you were away": built from turn summaries since the away start. */
/**
 * The end-of-Focus summary, when the digest is the one Focus sends (SPEC n33 3.4 D, 3.6 (5)).
 * `line` is the sentence the person reads. `counts` has numbers only; `held` is the total of pings Focus held.
 */
export interface DigestFocus {
  line: string;
  minutes: number;
  endedBy: 'timer' | 'early';
  counts: {
    turnsDone: number;
    agents: number;
    failed: number;
    asked: number;
    urgent: number;
    stillWaiting: number;
    held: number;
  };
}

export interface Digest {
  sinceMs: number;
  nowMs: number;
  agentsWorked: number;
  turns: number;
  lines: DigestLine[];
  needsYou: NeedsYou[];
  /** Set only for the end of a Focus. Absent on the normal "While you were away" digest. */
  focus?: DigestFocus;
}

export interface TurnSummaryApi {
  turnSummaries: (paneId: string) => Promise<TurnSummary[]>;
}

const noop: UnlistenFn = () => {};

export const isTauri = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error(`Desktop shell required: ${command}`);
  return invoke<T>(command, args);
}

async function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  if (!isTauri()) return noop;
  return listen<T>(event, (e) => cb(e.payload));
}

export const cockpitApi = {
  getUiMode: () => call<UiMode>('get_ui_mode'),
  setUiMode: (mode: UiMode) => call<void>('set_ui_mode', { mode }),
  openCockpit: (paneId?: string) => call<void>('open_cockpit', { paneId }),
  panePrompt: (paneId: string) => call<PanePrompt | null>('pane_prompt', { paneId }),
  /** A person's answer: `gesture` is their click (`gestureOf(event)`); without one the backend sends
   *  nothing but a refusal of an ordinary prompt (never a yes, never folder trust). */
  answerPanePrompt: (paneId: string, promptId: string, choice: string, gesture: UserGesture) =>
    call<void>('answer_pane_prompt', { paneId, promptId, choice, gesture }),
  /** Puppet Master's own No (Simple mode, a high-risk command): a refusal needs no click. */
  declinePanePrompt: (paneId: string, promptId: string, choice: string) =>
    call<void>('answer_pane_prompt', { paneId, promptId, choice, gesture: null }),
  interruptPane: (paneId: string) => call<void>('interrupt_pane', { paneId }),
  /** Rename a pane; a blank title clears it and automatic titling resumes. */
  paneSetTitle: (paneId: string, title: string) => call<string | null>('pane_set_title', { paneId, title }),

  onModeChanged: (cb: (e: { mode: UiMode }) => void) => on('ui://mode-changed', cb),
  onSelect: (cb: (e: { pane_id: string }) => void) => on('cockpit://select', cb),
  /** Rust (tray "Open Inbox" / "Talk to Fish") asks the cockpit to show a main view. */
  onView: (cb: (e: { view: string; bot?: string | null }) => void) => on('cockpit://view', cb),
  /** Rust asks the cockpit to show Settings inline (tray "Settings" in Orchestration mode). */
  onSettingsOpen: (cb: (e: { tab?: string | null }) => void) => on('cockpit://settings', cb),
  /** Rust asks a visible Settings window to move into the cockpit (the mode switched to Orchestration). */
  onSettingsToInline: (cb: () => void) => on('settings://to-inline', cb),
  // The event is built where the pane's folder is unknown, so its risk cannot say "outside this
  // project". Re-read the prompt through the command (which knows the folder) before showing it.
  onPrompt: (cb: (e: { pane_id: string; prompt: PanePrompt | null }) => void) =>
    on<{ pane_id: string; prompt: PanePrompt | null }>('pty://prompt', (e) => {
      if (!e.prompt) return cb(e);
      const id = e.prompt.promptId;
      call<PanePrompt | null>('pane_prompt', { paneId: e.pane_id })
        .then((fresh) => cb(fresh && fresh.promptId === id ? { pane_id: e.pane_id, prompt: fresh } : e))
        .catch(() => cb(e));
    }),
  onSpawnStage: (cb: (e: SpawnStageEvent) => void) => on('pane://spawn-stage', cb),
  onTurnSummary: (cb: (e: TurnSummary) => void) => on('turn://summary', cb),
  onDigestReady: (cb: (e: Digest) => void) => on('digest://ready', cb),
  onStandbyChanged: (cb: (e: unknown) => void) => on('standby://ready-changed', cb),

  questionsList: () => call<Question[]>('questions_list'),
  answerQuestion: (id: string, answer: { choice_id?: string; text?: string }) =>
    call<{ accepted: boolean; question: Question }>('question_answer', { id, choiceId: answer.choice_id, text: answer.text }),
  dismissQuestion: (id: string) => call<{ accepted: boolean; question: Question }>('question_dismiss', { id }),
  onQuestionsChanged: (cb: () => void) => on('questions://changed', cb),

  openSettings: (tab?: string) => call<void>('open_settings', { tab }),
};
