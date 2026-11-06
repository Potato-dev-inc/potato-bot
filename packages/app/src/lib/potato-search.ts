// Search chats: one box over every bot chat and group thread. The names mirror `potato/search.rs` and
// `commands/potato_search.rs` in camelCase. Desktop only; the search is local and runs off the UI thread.
// `fakeSearchTransport` is the in-memory twin for tests. Nothing here draws.
import { invoke } from '@tauri-apps/api/core';

export interface SearchHit {
  /** A bot's own chat, or a group's thread. */
  kind: 'bot' | 'group';
  /** The bot's id or the group's id. */
  ownerId: string;
  ownerName: string;
  messageId: string;
  at: number;
  speaker: string;
  snippet: string;
  /** Character ranges `[start, end)` of the snippet to highlight. */
  marks: [number, number][];
  /** How many messages come after this one in its thread. */
  fromEnd: number;
}

export interface SearchResults {
  hits: SearchHit[];
  /** Matches in all; more than `hits` holds when the list was cut. */
  matches: number;
  searched: number;
}

export interface SearchTransport {
  search(query: string): Promise<SearchResults>;
}

export const tauriSearchTransport: SearchTransport = {
  search: (query) => invoke<SearchResults>('potato_search_chats', { query }),
};

export const SEARCH_COPY = {
  title: 'Search chats',
  placeholder: 'Words from any bot or group chat',
  hint: 'Type a word or two. Every word has to be in the message.',
  searching: 'Searching',
  none: 'No message has those words',
  error: 'Could not search the chats.',
  back: 'Back to chat',
  group: 'Group',
  more: (shown: number, total: number) => `Showing the newest ${shown} of ${total} matches. Add a word to narrow it.`,
  open: (bot: string) => `Open the message in ${bot}`,
  tooOld: 'That message is further back than the group chat shows.',
} as const;

/** The most a thread is read back for a jump (the store keeps at most this many private messages). */
export const JUMP_MAX = 5000;
/** The group chat reads at most this many lines. */
export const GROUP_READ_MAX = 500;

/** How many messages a private thread must load for a hit to be in it: the ones after it, and some before for context. */
export function loadFor(hit: SearchHit, normal = 200): number {
  return Math.min(JUMP_MAX, Math.max(normal, hit.fromEnd + 30));
}

/** Whether the group chat can show this hit at all. */
export function groupCanShow(hit: SearchHit): boolean {
  return hit.fromEnd < GROUP_READ_MAX;
}

export interface SnippetPart {
  text: string;
  hit: boolean;
}

/** The snippet cut at its marks, in order, for drawing the matches. Out-of-range or overlapping marks are ignored. */
export function snippetParts(hit: Pick<SearchHit, 'snippet' | 'marks'>): SnippetPart[] {
  const chars = [...hit.snippet];
  const parts: SnippetPart[] = [];
  let at = 0;
  for (const [start, end] of [...hit.marks].sort((a, b) => a[0] - b[0])) {
    if (start < at || end <= start || end > chars.length) continue;
    if (start > at) parts.push({ text: chars.slice(at, start).join(''), hit: false });
    parts.push({ text: chars.slice(start, end).join(''), hit: true });
    at = end;
  }
  if (at < chars.length) parts.push({ text: chars.slice(at).join(''), hit: false });
  return parts;
}

/** The words the person typed, ready for the engine; empty means "do not search yet". */
export function searchWords(raw: string): string {
  return raw.replace(/\s+/g, ' ').trim().slice(0, 100);
}

// ---- the fake -----------------------------------------------------------------------------------

export interface FakeSearchCall {
  query: string;
}

export interface FakeSearchTransport extends SearchTransport {
  calls: FakeSearchCall[];
}

/** A transport over fixed rows: a row matches when every word is in its snippet (any case). Rejects when `fail` is set. */
export function fakeSearchTransport(rows: SearchHit[] = [], opts: { fail?: string; delayMs?: number } = {}): FakeSearchTransport {
  const calls: FakeSearchCall[] = [];
  return {
    calls,
    async search(query) {
      calls.push({ query });
      if (opts.delayMs) await new Promise((r) => setTimeout(r, opts.delayMs));
      if (opts.fail) throw new Error(opts.fail);
      const words = query.toLowerCase().split(/\s+/).filter(Boolean);
      const hits = rows.filter((r) => words.every((w) => r.snippet.toLowerCase().includes(w)));
      return { hits, matches: hits.length, searched: rows.length };
    },
  };
}
