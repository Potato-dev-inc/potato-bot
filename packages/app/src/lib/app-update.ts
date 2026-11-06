// One-click update. The app asks its own backend (Tauri updater: a signed MSI named in the GitHub release's latest.json);
// this file is the page's side: what the backend answers, how to read versions, and plain words for what went wrong.
import { invoke } from '@tauri-apps/api/core';

/** What the backend found when it checked. `null` means this version is the newest. */
export interface UpdateOffer {
  currentVersion: string;
  version: string;
  notes: string | null;
}

/** Kept for the Settings panel's props. */
export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  releaseUrl: string;
  releaseNotes: string | null;
  error: string | null;
}

export interface UpdateTransport {
  /** Reads the release's latest.json. Rejects with a plain sentence (no network, nothing found). */
  check(): Promise<UpdateOffer | null>;
  /** Ids of the bots whose job is running now. */
  busy(): Promise<string[]>;
  /** Downloads, checks the signature, stops the bots and the sandbox cleanly, installs. Rejects with a plain sentence. */
  install(confirmRunning: boolean): Promise<void>;
}

export const tauriUpdateTransport: UpdateTransport = {
  check: () => invoke<UpdateOffer | null>('app_update_check'),
  busy: async () => (await invoke<{ bots: string[] }>('app_update_busy')).bots,
  install: (confirmRunning) => invoke<void>('app_update_install', { confirmRunning }),
};

export function normalizeVersion(version: string): string {
  return version.trim().replace(/^v/i, '');
}

export function parseVersionParts(version: string): number[] {
  return normalizeVersion(version)
    .split(/[.-]/)
    .map((part) => {
      const match = /^(\d+)/.exec(part);
      return match ? Number.parseInt(match[1], 10) : 0;
    });
}

/** True when `latest` is strictly newer than `current`. */
export function isVersionNewer(latest: string, current: string): boolean {
  const a = parseVersionParts(latest);
  const b = parseVersionParts(current);
  const length = Math.max(a.length, b.length);
  for (let index = 0; index < length; index += 1) {
    const left = a[index] ?? 0;
    const right = b[index] ?? 0;
    if (left !== right) return left > right;
  }
  return false;
}

const FALLBACK = 'The update could not be installed. Your current version keeps working.';

/** A sentence for the page. Backend errors already are sentences; anything else (a code, a stack) becomes the fallback. */
export function plainUpdateError(error: unknown): string {
  const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const text = raw.replace(/^[A-Z_]{3,}:\s*/, '').trim();
  if (!text || /[\\/]|Error:|at \w/.test(text)) return FALLBACK;
  return text;
}

/** The sentence when a job is running: the person is told before anything starts. */
export function runningJobWarning(bots: string[]): string {
  const who = bots.length === 1 ? 'A bot is' : `${bots.length} bots are`;
  return `${who} working on a job. Updating stops it for a moment: it is saved and carries on after Potato Bot restarts.`;
}
