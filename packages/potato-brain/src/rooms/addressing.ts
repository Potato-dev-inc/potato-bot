import type { Member, RoomLine } from "./types.ts";

export const EVERYONE_WORDS = ["everyone", "everybody", "all of you", "you all", "@all", "@everyone"];

export type Aim = "me" | "everyone" | "other" | "none";

function norm(s: string): string { return s.toLowerCase().replace(/\s+/g, " ").trim(); }

const WORD = /[\p{L}\p{N}_]/u;
const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;

/** True when `@name` sits at index `at` of `t` as a whole name, not as the start of a longer word. */
function mentionAt(t: string, at: number, name: string, roster: Member[]): boolean {
  const before = at === 0 ? "" : (Array.from(t.slice(0, at)).pop() ?? "");
  if (before !== "" && WORD.test(before) && !CJK.test(before)) return false; // "a@fish" is an address, not a call
  const end = at + 1 + name.length;
  const next = Array.from(t.slice(end))[0] ?? "";
  if (next === "" || !WORD.test(next)) return true;
  // Chinese, Japanese and Korean have no spaces: "@小魚你好" calls 小魚. Unless a longer member name fits here ("@小魚乾").
  const last = Array.from(name).pop() ?? "";
  if (!CJK.test(last) || !CJK.test(next)) return false;
  return !roster.some((o) => norm(o.name).length > name.length && t.startsWith("@" + norm(o.name), at));
}

/** Names called with "@Name" (case-insensitive; a whole name, so "@fishing" does not call Fish). Names may have spaces. */
export function mentionNames(text: string, roster: Member[]): Member[] {
  const t = norm(text);
  return roster.filter((m) => {
    const name = norm(m.name);
    if (name === "") return false;
    for (let at = t.indexOf("@" + name); at !== -1; at = t.indexOf("@" + name, at + 1)) {
      if (mentionAt(t, at, name, roster)) return true;
    }
    return false;
  });
}

/** "Fish, ..." or "Fish: ..." at the start also calls Fish. */
function calledAtStart(text: string, m: Member): boolean {
  const t = norm(text);
  const n = norm(m.name);
  return t.startsWith(n + ",") || t.startsWith(n + ":") || t === n;
}

function hasEveryone(text: string): boolean {
  const t = " " + norm(text).replace(/[^\p{L}\p{N}@ ]/gu, " ") + " ";
  return EVERYONE_WORDS.some((w) => t.includes(" " + w + " "));
}

/**
 * Order: me (by @ or name at start, or a reply to my line) > everyone > other > none.
 * A line that calls me AND others still counts as "me".
 */
export function addressedTo(line: RoomLine, me: Member, roster: Member[], recent: RoomLine[]): Aim {
  const others = roster.filter((m) => m.id !== me.id && m.id !== line.from.id);
  const mentioned = mentionNames(line.text, roster);
  if (mentioned.some((m) => m.id === me.id) || calledAtStart(line.text, me)) return "me";
  if (line.inReplyTo) {
    const parent = recent.find((l) => l.id === line.inReplyTo);
    if (parent && parent.from.id === me.id) return "me";
  }
  if (hasEveryone(line.text)) return "everyone";
  if (mentioned.some((m) => m.id !== me.id) || others.some((m) => calledAtStart(line.text, m))) return "other";
  return "none";
}

/** fishd `aimedElsewhere`: true when the line clearly talks to someone who is not me. */
export function aimedElsewhere(line: RoomLine, me: Member, roster: Member[], recent: RoomLine[]): boolean {
  return addressedTo(line, me, roster, recent) === "other";
}
