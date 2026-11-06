import { CodedError } from "./errors.ts";
import type { Msg } from "../types.ts";

/**
 * Waiting out a model outage inside one turn: the same round is asked again, the work so far is kept.
 * Pure helpers only; the loop in loop.ts owns the state.
 */

/** Waits before retry 1..4 of one round, in ms: about 15 s, 45 s, 2 min, 4 min. */
export const RETRY_WAITS_MS = [15_000, 45_000, 120_000, 240_000] as const;
/** Retried model calls in one turn, all rounds together. */
export const RETRY_TURN_CAP = 8;
/** From this retry on (1-based) the request is shrunk. */
export const SHRINK_FROM_RETRY = 2;
/** Tool results that keep their full text when the request is shrunk. */
export const KEEP_TOOL_RESULTS = 6;
/** A pure-chat turn says nothing about a retry until it has waited this long. */
export const QUIET_WAIT_MS = 30_000;
export const OLD_OUTPUT_LINE = "[old output removed]";

export type Sleep = (ms: number, signal?: AbortSignal) => Promise<void>;

/** The error a stopped turn ends with. Never retried. */
export function stoppedError(): CodedError {
  return new CodedError("NETWORK", "stopped", undefined, "stopped");
}

export function isStopped(err: unknown): boolean {
  return err instanceof CodedError && err.reason === "stopped";
}

/** Only an outage is waited out: never a bad key or a limit. */
export function isOutage(err: unknown): err is CodedError {
  return err instanceof CodedError && (err.code === "PROVIDER_DOWN" || err.code === "NETWORK") && !isStopped(err);
}

/** Wait, but end at once (rejecting) when the signal fires. */
export const realSleep: Sleep = (ms, signal) =>
  new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(stoppedError());
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(stoppedError());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });

/** The wait before retry `n` (1-based) with +-10% jitter; `random` is in [0, 1). */
export function waitFor(n: number, random: () => number): number {
  const base = RETRY_WAITS_MS[Math.min(n, RETRY_WAITS_MS.length) - 1] ?? RETRY_WAITS_MS[RETRY_WAITS_MS.length - 1]!;
  return Math.round(base * (0.9 + 0.2 * random()));
}

/**
 * Replace the text of older tool results by one short line, keeping the last `keep`. Every message stays, and
 * every tool result keeps its toolCallId, so each tool_call id stays paired. System, user and assistant text
 * are not touched. Returns the same array when there is nothing to shrink.
 */
export function shrinkOldToolResults(msgs: Msg[], keep: number = KEEP_TOOL_RESULTS): Msg[] {
  const toolAt: number[] = [];
  msgs.forEach((m, i) => {
    if (m.role === "tool") toolAt.push(i);
  });
  const old = new Set(toolAt.slice(0, Math.max(0, toolAt.length - keep)));
  let changed = false;
  const out = msgs.map((m, i) => {
    if (!old.has(i)) return m;
    if (m.content === OLD_OUTPUT_LINE && !m.images) return m;
    changed = true;
    const { images: _images, ...rest } = m;
    return { ...rest, content: OLD_OUTPUT_LINE };
  });
  return changed ? out : msgs;
}
