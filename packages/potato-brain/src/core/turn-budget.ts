import type { Msg } from "../types.ts";

/**
 * Keeps the size of old tool output bounded inside one turn. A long job (a render log, a build, a big file) can return
 * tens of thousands of characters per call; sixty such rounds would not fit any model's window.
 *
 * The rule: when all tool results together pass `budget` characters, the oldest big ones (all but the newest `keepFull`)
 * become a head + tail stub, oldest first, until the total is under `lowWater`. If stubs alone are not enough, the oldest
 * stubs shrink to one line. Trimming only starts past the budget and then goes down to the low-water mark, so it
 * happens in steps, not every round: the start of the prompt (what the provider caches) stays unchanged in between.
 * Every message stays and keeps its toolCallId, so each call remains paired with its result.
 */
export interface TrimOptions {
  /** trimming starts when the tool results hold more than this many characters */
  budget: number;
  /** ...and goes on until they hold at most this many */
  lowWater: number;
  /** the newest this-many results are never touched */
  keepFull: number;
  /** a result shorter than this is not worth a stub */
  stubMin: number;
  head: number;
  tail: number;
}

export const DEFAULT_TRIM: TrimOptions = { budget: 160_000, lowWater: 100_000, keepFull: 4, stubMin: 3_000, head: 500, tail: 700 };
/** After the provider said the request was too long. */
export const HARD_TRIM: TrimOptions = { budget: 30_000, lowWater: 15_000, keepFull: 2, stubMin: 1_000, head: 250, tail: 350 };

const STUB_MARK = "[trimmed:";
const LINE_MARK = "[older output trimmed:";

function isTrimmed(content: string): boolean {
  return content.includes(STUB_MARK) || content.startsWith(LINE_MARK);
}

function stubOf(content: string, o: TrimOptions): string {
  const omitted = content.length - o.head - o.tail;
  return `${content.slice(0, o.head)}\n${STUB_MARK} ${omitted} characters of older output cut out here to save room; the start and the end are kept. Run the command again if you need the middle.]\n${content.slice(content.length - o.tail)}`;
}

/** Returns the same array when nothing needed trimming. */
export function trimTurnToolResults(msgs: Msg[], opts: TrimOptions = DEFAULT_TRIM): Msg[] {
  const toolAt: number[] = [];
  let total = 0;
  msgs.forEach((m, i) => {
    if (m.role !== "tool") return;
    toolAt.push(i);
    total += m.content.length;
  });
  if (total <= opts.budget) return msgs;

  const older = toolAt.slice(0, Math.max(0, toolAt.length - opts.keepFull));
  const out = msgs.slice();
  let changed = false;
  // Pass 1: big results become head + tail stubs, oldest first.
  for (const i of older) {
    if (total <= opts.lowWater) break;
    const m = msgs[i] as Msg;
    if (m.content.length < opts.stubMin || m.content.length <= opts.head + opts.tail + 200 || isTrimmed(m.content)) continue;
    const next = stubOf(m.content, opts);
    total -= m.content.length - next.length;
    out[i] = { ...m, content: next };
    changed = true;
  }
  // Pass 2: still too much, so the oldest stubs (and any other old result) shrink to a single line.
  for (const i of older) {
    if (total <= opts.lowWater) break;
    const m = out[i] as Msg;
    if (m.content.startsWith(LINE_MARK) || m.content.length <= 120) continue;
    const next = `${LINE_MARK} ${m.content.length} characters]`;
    total -= m.content.length - next.length;
    out[i] = { ...m, content: next };
    changed = true;
  }
  return changed ? out : msgs;
}
