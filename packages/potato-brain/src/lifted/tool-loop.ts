// Ported from Rakazo (https://github.com/elie222/rakazo), commit f48fa5c7, files packages/adapters/src/tool-loop.ts and packages/core/src/events.ts (trackToolCallStreak, ToolCallStreak only).
// Copyright 2026 Rakazo contributors. Licensed under the Apache License, Version 2.0;
// see packages/potato-sandbox/LICENSE-APACHE-2.0. Changed by Puppet Master: trackToolCallStreak moved here from events.ts;
// no @rakazo imports; the stop text names the brain's tool, and the thresholds are unchanged (6 identical, 25 similar).

import { sha } from "../core/hash.ts";

/** `shape` is the call with the digits in its text arguments blanked, so `echo n-1`, `echo n-2` match. */
export type ToolCallStreak = {
  key: string | undefined;
  count: number;
  shape?: string;
  shapeCount?: number;
  /** hash of the last result text, and how many calls in a row (the same call) gave it */
  resultKey?: string;
  sameResultRun?: number;
};

export function trackToolCallStreak(streak: ToolCallStreak, name: string, args: unknown): ToolCallStreak {
  const key = `${name}:${JSON.stringify(args)}`;
  const shape = `${name}:${JSON.stringify(args, (_, value: unknown) => (typeof value === "string" ? value.replace(/\d+/g, "#") : value))}`;
  return {
    // the result record of the call before rides along: noteToolResult compares against it
    ...(streak.resultKey === undefined ? {} : { resultKey: streak.resultKey }),
    ...(streak.sameResultRun === undefined ? {} : { sameResultRun: streak.sameResultRun }),
    key,
    count: key === streak.key ? streak.count + 1 : 1,
    shape,
    shapeCount: shape === streak.shape ? (streak.shapeCount ?? 0) + 1 : 1,
  };
}

const LOOP_GUARD_STOP_PREFIX = "I got stuck calling ";
const LOOP_GUARD_STOP_COUNT_PATTERN = " with (?:nearly )?the same input ";
const LOOP_GUARD_STOP_SUFFIX =
  " times in a row without making progress, so I stopped early. Try rephrasing this, or ask me to try a different approach.";
const LOOP_GUARD_STOP_PATTERN = new RegExp(
  `^${LOOP_GUARD_STOP_PREFIX}.+${LOOP_GUARD_STOP_COUNT_PATTERN}\\d+${LOOP_GUARD_STOP_SUFFIX.replaceAll(".", "\\.")}$`,
);

// Same tool, same arguments, this many times in a row means the agent is stuck, not paginating.
export const MAX_CONSECUTIVE_IDENTICAL_TOOL_CALLS = 6;
// Arguments that differ only by digits in their text (`echo noop-1`, `echo noop-2`) can be a real
// sequence such as reading a file in chunks or paging through a site, so they get more room.
export const MAX_CONSECUTIVE_SIMILAR_TOOL_CALLS = 25;

export function loopGuardStopText(toolName: string, streak: ToolCallStreak): string {
  const identical = streak.count >= MAX_CONSECUTIVE_IDENTICAL_TOOL_CALLS;
  const count = identical ? streak.count : toolCallStreakLength(streak);
  return `${LOOP_GUARD_STOP_PREFIX}${toolName} with ${identical ? "" : "nearly "}the same input ${count}${LOOP_GUARD_STOP_SUFFIX}`;
}

export function isLoopGuardStopText(text: string): boolean {
  return LOOP_GUARD_STOP_PATTERN.test(text) || isSameResultStopText(text);
}

/** The longer of the two runs, for the stop message. */
export function toolCallStreakLength(streak: ToolCallStreak): number {
  return Math.max(streak.count, streak.shapeCount ?? 0);
}

/**
 * Checking a background task (`task` with action status or tail) is rate limited by the host (one fresh answer per task
 * per 30 s, the last answer in between), so a long run of them is a patient wait, not a loop. They do not count toward
 * the "similar" limit. They still count toward the identical limit, and the same-call-same-result guard still ends a
 * loop that is really stuck.
 */
export function isTaskCheck(name: string, args: unknown): boolean {
  if (name !== "task" || typeof args !== "object" || args === null) return false;
  const action = (args as { action?: unknown }).action;
  return action === "status" || action === "tail";
}

/** Tracks one more call. `stuck` is true at 6 identical calls in a row or 25 similar ones (task status/tail excepted). */
export function advanceToolCallLoopGuard(
  streak: ToolCallStreak,
  name: string,
  args: unknown,
): { streak: ToolCallStreak; stuck: boolean } {
  const next = trackToolCallStreak(streak, name, args);
  return {
    streak: next,
    stuck:
      next.count >= MAX_CONSECUTIVE_IDENTICAL_TOOL_CALLS ||
      (!isTaskCheck(name, args) && (next.shapeCount ?? 0) >= MAX_CONSECUTIVE_SIMILAR_TOOL_CALLS),
  };
}

// ---- the same call AND the same result (added by Puppet Master) ----------------------------------------------
// Polling a job is a fair use of the same call, but the same call returning the same text again and again is a
// loop. Third time in a row: a note to the model. Fifth: the turn ends with a plain message.

export const SAME_RESULT_NUDGE_AT = 3;
export const SAME_RESULT_STOP_AT = 5;

function textKey(text: string): string {
  return `${text.length}:${sha(text)}`;
}

/**
 * Records the result of the call `advanceToolCallLoopGuard` just counted. The run grows while the call is the same as
 * the one before (count > 1) and the result text is the same; anything else restarts it at 1.
 */
export function noteToolResult(
  streak: ToolCallStreak,
  resultText: string,
): { streak: ToolCallStreak; nudge: boolean; stuck: boolean } {
  const key = textKey(resultText);
  const same = streak.count > 1 && streak.resultKey === key;
  const run = same ? (streak.sameResultRun ?? 1) + 1 : 1;
  return {
    streak: { ...streak, resultKey: key, sameResultRun: run },
    nudge: run === SAME_RESULT_NUDGE_AT,
    stuck: run >= SAME_RESULT_STOP_AT,
  };
}

export function sameResultNudge(streak: ToolCallStreak): string {
  return `[Note: this exact call has now returned exactly the same result ${streak.sameResultRun ?? SAME_RESULT_NUDGE_AT} times in a row. Repeating it will not change anything. Try something different, or tell the person what is blocking you.]`;
}

const SAME_RESULT_STOP_PATTERN = /^I ran .+ \d+ times in a row and got exactly the same result each time, so I stopped instead of repeating it\./;

export function sameResultStopText(toolName: string, streak: ToolCallStreak): string {
  return `I ran ${toolName} ${streak.sameResultRun ?? SAME_RESULT_STOP_AT} times in a row and got exactly the same result each time, so I stopped instead of repeating it. Tell me what to try differently, or what you see on your side.`;
}

export function isSameResultStopText(text: string): boolean {
  return SAME_RESULT_STOP_PATTERN.test(text);
}
