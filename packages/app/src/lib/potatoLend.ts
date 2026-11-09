// "Apps from your PC" (Labs `potato_lend_app_enabled`, Windows only): the person lends an app they have open to their
// bot. The names mirror the Rust serde shapes: WinInfo is snake_case (no rename), LentView is camelCase (rename_all).
// Every Tauri call for the lend feature goes through this file, so a field rename is a one-file change.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isTauri } from './cockpit-api';

/** `visible` stays where it is with a border; `hidden` is see-through and click-through, and still draws. */
export type LendMode = 'visible' | 'hidden';

/** Who has the app: nobody, the bot, the person, or a bot that asked the person to step in. */
export type LendPhase = 'free' | 'bot' | 'you' | 'needs_you';

export interface LendRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A window the person can lend (from `potato_lend_candidates`). */
export interface WinInfo {
  hwnd: number;
  pid: number;
  title: string;
  /** The file name, e.g. `LINE.exe`. */
  exe: string;
  /** The exe name without `.exe`, or the file description. */
  app_name: string;
  rect: LendRect;
  /** Runs as administrator: the app cannot be lent. */
  elevated: boolean;
  minimized: boolean;
}

/** A lent app (from `potato_lend_list` or `potato_lend_start`), camelCase as Rust serializes it. */
export interface LentView {
  id: string;
  hwnd: number;
  pid: number;
  appName: string;
  title: string;
  mode: LendMode;
  phase: LendPhase;
  /** The bot's reason on a "needs you" card (plain text). */
  reason?: string | null;
}

export interface LendTransport {
  candidates(): Promise<WinInfo[]>;
  start(hwnd: number, mode: LendMode): Promise<LentView>;
  list(): Promise<LentView[]>;
  bringBack(id: string): Promise<void>;
  setMode(id: string, mode: LendMode): Promise<void>;
  takeOver(id: string): Promise<void>;
  handBack(id: string): Promise<void>;
  /** A PNG as base64 (no `data:` prefix), at most 320 px wide. */
  thumbnail(hwnd: number): Promise<string>;
  /** `potato://lend`: read the list again. */
  onChanged?(cb: () => void): Promise<UnlistenFn>;
}

export const LEND_FLAG = 'potato_lend_app_enabled';

export const tauriLendTransport: LendTransport = {
  candidates: () => invoke<WinInfo[]>('potato_lend_candidates'),
  start: (hwnd, mode) => invoke<LentView>('potato_lend_start', { hwnd, mode }),
  list: () => invoke<LentView[]>('potato_lend_list'),
  bringBack: async (id) => {
    await invoke('potato_lend_bring_back', { id });
  },
  setMode: async (id, mode) => {
    await invoke('potato_lend_set_mode', { id, mode });
  },
  takeOver: async (id) => {
    await invoke('potato_lend_take_over', { id });
  },
  handBack: async (id) => {
    await invoke('potato_lend_hand_back', { id });
  },
  thumbnail: (hwnd) => invoke<string>('potato_lend_thumbnail', { hwnd }),
  onChanged: async (cb) => {
    if (!isTauri()) return (() => {}) as UnlistenFn;
    return listen('potato://lend', () => cb());
  },
};

/** The lend feature exists on Windows only. */
export function isWindowsRuntime(): boolean {
  return typeof navigator !== 'undefined' && navigator.userAgent.includes('Windows');
}

/** Plain words for a failed lend call. The Rust error text is matched loosely; anything else gets a generic line. */
export function lendErrorText(error: unknown): string {
  const text = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const lower = text.toLowerCase();
  if (lower.includes('person has control') || lower.includes('lease_held')) return 'You have control of this app right now.';
  if (lower.includes('window is gone')) return 'That window is gone. Lend another app.';
  if (lower.includes('no such lent app')) return 'That app is no longer lent.';
  if (lower.includes('not supported')) return 'Lending apps works on Windows only.';
  if (lower.includes('elevated') || lower.includes('administrator')) return "Runs as administrator, can't be lent";
  return 'That did not work. Try again.';
}
