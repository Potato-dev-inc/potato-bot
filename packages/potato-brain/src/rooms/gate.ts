import type { Member, RoomFacts, Verdict } from "./types.ts";
import { MAX_BOT_HOPS } from "./types.ts";
import { addressedTo } from "./addressing.ts";
import type { SilenceBudget } from "./silence.ts";

/** fishd also has "jev", "llm" and "chain"; version 1 builds only the free heuristic. */
export type GateMode = "heuristic";
export type StageResult = Verdict | "next";
export interface GateState { silence: SilenceBudget; now: number }
export type Stage = (f: RoomFacts, s: GateState) => StageResult;

/** Base threshold for speaking when nobody was called. Higher = quieter. */
export const THRESHOLD = 0.45;
export const LEFT_ALONE_BIAS = 1.6; // fishd
export const AIMED_ELSEWHERE_BIAS = 2.5; // fishd; kept for a later ambient mode, unused by the heuristic chain
const LEFT_ALONE_WORDS = ["be quiet", "shh", "stop talking", "leave me alone", "quiet please", "not now"];
const LEFT_ALONE_WINDOW_MS = 10 * 60_000;

export function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
  return h >>> 0;
}

/** Every brain computes the same answer from the same roster, so only one bot answers an open question. */
export function firstResponder(lineId: string, roster: Member[]): string | null {
  const bots = roster.filter((m) => m.kind === "bot");
  if (bots.length === 0) return null;
  return bots.reduce((best, b) => (fnv1a(lineId + ":" + b.id) < fnv1a(lineId + ":" + best.id) ? b : best)).id;
}

/** The person asked the bots to be quiet in the last 10 minutes. */
export function leftAlone(f: RoomFacts): boolean {
  return f.recent.concat(f.line).some((l) => l.from.kind === "person" && f.line.atMs - l.atMs < LEFT_ALONE_WINDOW_MS &&
    LEFT_ALONE_WORDS.some((w) => l.text.toLowerCase().includes(w)));
}

/** 0..1. A question to nobody scores 0.6; a plain statement 0.2; very short lines (< 4 chars) 0. */
export function heuristicScore(f: RoomFacts): number {
  const t = f.line.text.trim();
  if (t.length < 4) return 0;
  return /[?？]$/.test(t) ? 0.6 : 0.2;
}

// ---- stages, in order ----
export const ownLine: Stage = (f) => (f.line.from.id === f.me.id ? { kind: "pass", why: "own_line" } : "next");
export const hopLimit: Stage = (f) => (f.line.hop >= MAX_BOT_HOPS ? { kind: "pass", why: "hop_limit" } : "next");
export const addressed: Stage = (f) => {
  const aim = addressedTo(f.line, f.me, f.roster, f.recent);
  if (aim === "me") return { kind: "speak", why: "addressed" };
  if (f.line.from.kind === "bot") return { kind: "pass", why: "bot_not_addressed" }; // bots only answer bots when called
  if (aim === "everyone") return { kind: "speak", why: "everyone" };
  if (aim === "other") return { kind: "pass", why: "aimed_elsewhere" };
  return "next";
};
export const silence: Stage = (f, s) => {
  if (f.line.from.kind === "person") return "next"; // the person is never met with silence because a bot spoke lately
  const b = s.silence.blocked(s.now);
  return b === "budget" ? { kind: "pass", why: "silence_budget" } : b === "cooldown" ? { kind: "pass", why: "cooldown" } : "next";
};
export const responder: Stage = (f) =>
  firstResponder(f.line.id, f.roster) === f.me.id ? "next" : { kind: "pass", why: "not_first_responder" };
export const heuristic: Stage = (f) => {
  const quiet = leftAlone(f);
  // The person talked to the group and named nobody: the first responder answers, even to "hi". Silence here looked
  // like a broken group (the app saves nothing for a pass). Only an explicit "be quiet" holds it back.
  if (f.line.from.kind === "person" && !quiet) return { kind: "speak", why: heuristicScore(f) >= 0.6 ? "question" : "chatty" };
  const score = heuristicScore(f);
  if (score >= THRESHOLD * (quiet ? LEFT_ALONE_BIAS : 1)) return { kind: "speak", why: score >= 0.6 ? "question" : "chatty" };
  return { kind: "pass", why: quiet ? "left_alone" : "low_score" };
};

export const HEURISTIC_CHAIN: Stage[] = [ownLine, hopLimit, addressed, silence, responder, heuristic];

export function makeGate(_mode: GateMode = "heuristic"): (f: RoomFacts, s: GateState) => Verdict {
  const chain = HEURISTIC_CHAIN;
  return (f, s) => {
    for (const stage of chain) {
      const r = stage(f, s);
      if (r !== "next") return r;
    }
    return { kind: "pass", why: "low_score" };
  };
}
