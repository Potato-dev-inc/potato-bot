// Spaces: calling a bot with `@name`. The match is the same as the Rust side (`bot_bridge::is_called`), so the screen
// can say why a call will not be answered before and after the line is sent.
import type { BotRefusal, SpaceDetailBot } from '../../lib/spaces';

const isWord = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}_]/u.test(c);
const same = (a: string, b: string) => a === b || a.toLowerCase() === b.toLowerCase();

/** `@name` as a word: nothing alphanumeric or `_` right before the `@` or right after the name; any case. */
export function isCalled(text: string, name: string): boolean {
  const t = [...text];
  const n = [...name];
  if (n.length === 0 || t.length < n.length + 1) return false;
  for (let i = 0; i <= t.length - n.length - 1; i++) {
    if (t[i] !== '@' || (i > 0 && isWord(t[i - 1]))) continue;
    if (!n.every((c, k) => same(t[i + 1 + k], c))) continue;
    if (isWord(t[i + 1 + n.length])) continue;
    return true;
  }
  return false;
}

export const refusalText = (name: string, why: BotRefusal): string => {
  switch (why) {
    case 'limit': return `${name} did not answer: it reached its daily money limit.`;
    case 'busy': return `${name} did not answer: it was busy with another turn. Call it again in a moment.`;
    case 'not_allowed': return `${name} did not answer: this kind of bot can't be used in a space.`;
    default: return `${name} did not answer: it is switched off, paused or has no key. Check it in Potato Bot.`;
  }
};
export const silencedText = (name: string) => `${name} is silenced, so it will not answer. Turn off "Silence this bot" in Bots.`;
export const hiddenText = (name: string) => `${name} is hidden on this computer. Show it in Bots to get its answers.`;
export const dailyLimitText = (name: string, cap: number) => `${name} reached its daily limit of ${cap}, so it will not answer today.`;
export const wrongNameText = (typed: string, suggest: string) => `No bot here is called @${typed}. Did you mean @${suggest}?`;
export const BOTS_OFF_CALL = 'Bots are off in this channel, so nobody will answer.';

/** The `@word`s of a line (a trailing mark such as `,` or `?` is not part of the word). */
function mentions(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/(?<![\p{L}\p{N}_])@([^\s@]+)/gu)) {
    const w = m[1].replace(/[\p{P}\p{S}]+$/u, '');
    if (w) out.push(w);
  }
  return out;
}

/** A bot the word is probably short for: part of its name, or its name without the `<owner>的` front. */
function nearBot(word: string, bots: SpaceDetailBot[]): SpaceDetailBot | undefined {
  const w = word.toLowerCase();
  return bots.find((b) => {
    const n = b.name.toLowerCase();
    return n !== w && (n.includes(w) || w.includes(n));
  });
}

/**
 * Why a line the person is typing will not get an answer from a bot, or null. Only a line that calls a bot, or tries to,
 * gets a message: plain talk and `@` for something else are left alone.
 */
export function callProblem(text: string, bots: SpaceDetailBot[], botsOff: boolean): string | null {
  if (!text.includes('@') || bots.length === 0) return null;
  const called = bots.filter((b) => isCalled(text, b.name));
  if (called.length === 0) {
    for (const w of mentions(text)) {
      const near = nearBot(w, bots);
      if (near) return wrongNameText(w, near.name);
    }
    return null;
  }
  if (botsOff) return BOTS_OFF_CALL;
  for (const b of called) {
    if (b.mine && b.silenced) return silencedText(b.name);
    if (b.muted) return hiddenText(b.name);
    if (b.mine && b.dayCap != null && (b.usedToday ?? 0) >= b.dayCap) return dailyLimitText(b.name, b.dayCap);
    if (b.mine && b.refused) return refusalText(b.name, b.refused);
  }
  return null;
}

/** After a send: my bots whose last call was refused, as sentences. */
export function refusedNotes(bots: SpaceDetailBot[]): string[] {
  return bots.filter((b) => b.mine && b.refused).map((b) => refusalText(b.name, b.refused as BotRefusal));
}
