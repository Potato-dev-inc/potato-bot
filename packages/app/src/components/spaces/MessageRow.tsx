import type { ReactNode } from 'react';
import type { SpaceMessage } from '../../lib/spaces';
import { FileCard } from './FileCard';
import { Avatar, avatarKey, mentionsMe, renderMentions } from './messageParts';

export const REACTION_PICTURE: Record<string, string> = {
  thumbs_up: '\u{1F44D}', heart: '\u{2764}\u{FE0F}', laugh: '\u{1F602}', wow: '\u{1F62E}',
  sad: '\u{1F622}', party: '\u{1F389}', eyes: '\u{1F440}', check: '\u{2705}',
};

export function timeOf(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** A bot line is signed by its owner's key, but it is the bot talking: never "mine", whoever owns the bot. */
const isMineRow = (m: SpaceMessage) => m.isMine && !m.isFish;

export interface MessageRowProps {
  message: SpaceMessage;
  onReply: (m: SpaceMessage) => void;
  /** The message menu (it owns Reply too). Without it the row shows its own Reply link. */
  menu?: (m: SpaceMessage, reply: () => void) => ReactNode;
  onReact?: (m: SpaceMessage, code: string, on: boolean) => void;
  /** Same sender as the line above, a moment later: no avatar or name (kept for screen readers), the time shows in the gutter on hover or focus. */
  grouped?: boolean;
  revealBlocked?: boolean;
  onRevealBlocked?: () => void;
  /** Files: what to show under a file card and what its two buttons do. */
  fileError?: string | null;
  fileSavedAs?: string | null;
  onGetFile?: (m: SpaceMessage) => void;
  onSaveFile?: (m: SpaceMessage) => void;
  /** Every name that can be @mentioned in this channel, and my own name (to mark mentions of me). */
  names?: string[];
  myName?: string;
  /** The reply line was clicked: scroll to the original. */
  onJump?: (id: string) => void;
  /** Just jumped to: shown with a brief highlight. */
  flash?: boolean;
}

export function MessageRow({ message: m, onReply, menu, onReact, grouped, revealBlocked, onRevealBlocked, fileError, fileSavedAs, onGetFile, onSaveFile, names = [], myName, onJump, flash }: MessageRowProps) {
  const mention = !m.deleted && !m.file && mentionsMe(m.text, myName);
  const cls = ['ck-spaces-msg', m.isFish && 'is-fish is-bot', grouped && 'is-grouped', m.deleted && 'is-deleted', mention && 'is-mention', flash && 'is-flash',
    m.blocked && !revealBlocked && 'is-blocked'].filter(Boolean).join(' ');
  if (m.blocked && !revealBlocked) {
    return (
      <li className={cls}>
        <span className="ck-spaces-msg-text">Message from someone you blocked.</span>
        <button type="button" className="ck-spaces-quiet" onClick={onRevealBlocked}>Show</button>
      </li>
    );
  }
  const iso = new Date(m.createdAtMs).toISOString();
  const actions = !m.deleted && (menu
    ? <span className="ck-spaces-msg-actions">{menu(m, () => onReply(m))}</span>
    : <span className="ck-spaces-msg-actions"><button type="button" className="ck-spaces-more" onClick={() => onReply(m)}>Reply</button></span>);
  return (
    <li className={cls} data-id={m.id} data-mine={isMineRow(m) ? 'true' : undefined}>
      {m.replyPreview && (
        <button type="button" className="ck-spaces-msg-reply" disabled={!m.replyTo || !onJump} onClick={() => m.replyTo && onJump?.(m.replyTo)}
          aria-label={`In reply to ${m.replyPreview.authorName}: ${m.replyPreview.text}`}>
          <svg className="ck-spaces-reply-curve" viewBox="0 0 20 12" width="20" height="12" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" aria-hidden="true">
            <path d="M2 12V6.5Q2 2 6.5 2H19" />
          </svg>
          <Avatar name={m.replyPreview.authorName} id={m.replyPreview.authorName} small />
          <span className="ck-spaces-reply-author">{m.replyPreview.authorName}</span>
          <span className="ck-spaces-reply-text">{m.replyPreview.text}</span>
        </button>
      )}
      <div className="ck-spaces-msg-gutter">
        {grouped
          ? <time className="ck-spaces-msg-time" dateTime={iso}>{timeOf(m.createdAtMs)}</time>
          : <Avatar name={m.authorName} id={avatarKey(m)} bot={m.isFish} />}
      </div>
      <div className="ck-spaces-msg-body">
        <div className="ck-spaces-msg-line">
          <span className="ck-spaces-msg-author">{m.authorName}</span>
          {m.isFish && <span className="ck-spaces-bot-tag">Bot</span>}
          {m.isFish && m.ownerName && <span className="ck-spaces-msg-owner" title="Who brought this bot">{m.ownedByMe ? 'yours' : `${m.ownerName}'s`}</span>}
          {!grouped && <time className="ck-spaces-msg-time" dateTime={iso}>{timeOf(m.createdAtMs)}</time>}
        </div>
        {m.deleted ? (
          <p className="ck-spaces-msg-text">
            {isMineRow(m) && !m.deleted.byOwner ? 'You deleted this message.'
              : m.deleted.byOwner ? 'Deleted by the owner.' : `${m.deleted.byName} deleted this message.`}
          </p>
        ) : m.file ? (
          <FileCard file={m.file} error={fileError ?? null} savedAs={fileSavedAs ?? null}
            onGet={() => onGetFile?.(m)} onSave={() => onSaveFile?.(m)} />
        ) : (
          <p className="ck-spaces-msg-text">{renderMentions(m.text, names, myName)}</p>
        )}
        {m.reactions.length > 0 && !m.deleted && (
          <ul className="ck-spaces-reactions">
            {m.reactions.map((r) => (
              <li key={r.code}>
                <button type="button" className={`ck-spaces-reaction${r.mine ? ' is-mine' : ''}`}
                  aria-pressed={r.mine} aria-label={`${r.code.replace('_', ' ')}, ${r.count}`}
                  onClick={() => onReact?.(m, r.code, !r.mine)}>
                  {REACTION_PICTURE[r.code]} {r.count}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      {actions}
    </li>
  );
}
