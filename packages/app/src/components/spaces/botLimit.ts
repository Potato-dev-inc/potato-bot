// Spaces: how many bots a person may have here, in the words the bot screens use.
import type { SpaceBotList } from '../../lib/spaces';

export const ASK_OWNER = 'Ask the space owner to allow more.';
export const CHANGE_LIMIT = 'Change Bots per person';

export interface BotLimit {
  allowed: number;
  used: number;
  /** My bots on the roster and my bots waiting for the space owner's computer. */
  names: string[];
  reached: boolean;
}

export function botLimit(list: SpaceBotList | null): BotLimit | null {
  if (!list) return null;
  const names = [...list.bots.filter((b) => b.mine).map((b) => b.name), ...list.pendingAdd];
  const used = list.mineCount + list.pendingAdd.length;
  return { allowed: list.rules.perPerson, used: Math.max(used, names.length), names, reached: used >= list.rules.perPerson };
}

const bots = (n: number) => (n === 1 ? '1 bot' : `${n} bots`);
const joined = (names: string[]) => names.join(' and ');

/** Before anything is filled in: what is allowed and what is used. */
export const limitLine = (l: BotLimit) => `This space allows ${bots(l.allowed)} per person. You have ${l.used}.`;

/** The limit is reached: what is already here (or that none are allowed) and the limit. */
export function limitReachedText(l: BotLimit): string {
  if (l.allowed === 0) return 'This space allows 0 bots per person.';
  if (l.names.length === 0) return `You already have as many bots here as this space allows. It allows ${bots(l.allowed)} per person.`;
  return `${joined(l.names)} ${l.names.length === 1 ? 'is' : 'are'} already here. This space allows ${bots(l.allowed)} per person.`;
}
