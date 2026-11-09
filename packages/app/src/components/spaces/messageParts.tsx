// Spaces feed: small pieces of the Discord-like message row (avatar, mentions, day labels). Grayscale only; no colours.
import type { ReactNode } from 'react';
import type { SpaceMessage } from '../../lib/spaces';

/** A stable number from a member or bot id (FNV-1a), so one sender keeps one shade. */
export function hashOf(key: string): number {
  let h = 2166136261;
  for (let i = 0; i < key.length; i += 1) { h ^= key.charCodeAt(i); h = Math.imul(h, 16777619); }
  return h >>> 0;
}
export const SHADES = 6;
export const shadeOf = (key: string) => hashOf(key) % SHADES;
export const initialOf = (name: string) => (Array.from(name.trim())[0] ?? '?').toUpperCase();
/** What the avatar is keyed on: the bot id for a bot, the member id for a person. */
export const avatarKey = (m: SpaceMessage) => (m.isFish ? m.botId ?? m.member : m.member) || m.authorName;

export function Avatar({ name, id, bot, small }: { name: string; id: string; bot?: boolean; small?: boolean }) {
  const cls = ['ck-spaces-avatar', bot && 'is-bot', small && 'is-small'].filter(Boolean).join(' ');
  return (
    <span className={cls} data-shade={shadeOf(id)} aria-hidden="true">
      {bot && !small ? (
        <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round">
          <rect x="2.5" y="5" width="11" height="8" rx="2.5" /><path d="M8 5V2.8" /><circle cx="5.9" cy="9" r=".6" fill="currentColor" /><circle cx="10.1" cy="9" r=".6" fill="currentColor" />
        </svg>
      ) : initialOf(name)}
    </span>
  );
}

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const WORD = String.raw`[\p{L}\p{N}_][\p{L}\p{N}_'’-]*`;

/** Text with each @mention in its own element. Known names (they may hold spaces) win over the plain @word rule. */
export function renderMentions(text: string, known: string[], myName: string | undefined): ReactNode[] {
  const names = [...new Set(known.filter((n) => n.trim()))].sort((a, b) => b.length - a.length).map(escapeRe);
  const alt = names.length ? `(?:${names.join('|')})(?![\\p{L}\\p{N}_])|` : '';
  const re = new RegExp(`(?<![\\p{L}\\p{N}_@])@(${alt}${WORD})`, 'giu');
  const out: ReactNode[] = [];
  let last = 0;
  for (const hit of text.matchAll(re)) {
    const at = hit.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const me = !!myName && hit[1].toLowerCase() === myName.toLowerCase();
    out.push(<span key={at} className={me ? 'ck-spaces-mention is-me' : 'ck-spaces-mention'}>{hit[0]}</span>);
    last = at + hit[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export const mentionsMe = (text: string, myName: string | undefined) =>
  !!myName && new RegExp(`(?<![\\p{L}\\p{N}_@])@${escapeRe(myName)}(?![\\p{L}\\p{N}_])`, 'iu').test(text);

const dayStart = (ms: number) => { const d = new Date(ms); d.setHours(0, 0, 0, 0); return d.getTime(); };
export const sameDay = (a: number, b: number) => dayStart(a) === dayStart(b);
export function dayLabel(ms: number, now = Date.now()): string {
  const diff = Math.round((dayStart(now) - dayStart(ms)) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  const d = new Date(ms);
  return d.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric', ...(d.getFullYear() !== new Date(now).getFullYear() && { year: 'numeric' }) });
}
