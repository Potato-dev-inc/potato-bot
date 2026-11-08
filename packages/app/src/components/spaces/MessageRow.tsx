import type { ReactNode } from 'react';
import type { SpaceMessage } from '../../lib/spaces';
import { FileCard } from './FileCard';
import '../../styles/cockpit-chat.css';
import '../../styles/cockpit-potato-thread.css';

export const REACTION_PICTURE: Record<string, string> = {
  thumbs_up: '\u{1F44D}', heart: '\u{2764}\u{FE0F}', laugh: '\u{1F602}', wow: '\u{1F62E}',
  sad: '\u{1F622}', party: '\u{1F389}', eyes: '\u{1F440}', check: '\u{2705}',
};

export function timeOf(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** What a message looks like, from the Potato chat: yours is a filled bubble, a bot's an outlined one, a person's plain text. */
const bubbleClass = (m: SpaceMessage) =>
  m.isFish ? 'ck-potato-bubble ck-potato-bubble--bot' : m.isMine ? 'ck-potato-bubble ck-potato-bubble--you' : 'ck-chat-text';

export interface MessageRowProps {
  message: SpaceMessage;
  onReply: (m: SpaceMessage) => void;
  /** The message menu (it owns Reply too). Without it the row shows its own Reply link. */
  menu?: (m: SpaceMessage, reply: () => void) => ReactNode;
  onReact?: (m: SpaceMessage, code: string, on: boolean) => void;
  /** Same sender as the line above, a moment later: the name and bot tag are said once (kept for screen readers), the time shows on hover or focus. */
  grouped?: boolean;
  revealBlocked?: boolean;
  onRevealBlocked?: () => void;
  /** Files: what to show under a file card and what its two buttons do. */
  fileError?: string | null;
  fileSavedAs?: string | null;
  onGetFile?: (m: SpaceMessage) => void;
  onSaveFile?: (m: SpaceMessage) => void;
}

export function MessageRow({ message: m, onReply, menu, onReact, grouped, revealBlocked, onRevealBlocked, fileError, fileSavedAs, onGetFile, onSaveFile }: MessageRowProps) {
  const cls = ['ck-spaces-msg', m.isMine && 'is-mine', m.isFish && 'is-fish is-bot', grouped && 'is-grouped', m.deleted && 'is-deleted',
    m.blocked && !revealBlocked && 'is-blocked'].filter(Boolean).join(' ');
  if (m.blocked && !revealBlocked) {
    return (
      <li className={cls}>
        <span className="ck-spaces-msg-text">Message from someone you blocked.</span>
        <button type="button" className="ck-spaces-quiet" onClick={onRevealBlocked}>Show</button>
      </li>
    );
  }
  return (
    <li className={cls} data-id={m.id}>
      <div className="ck-spaces-msg-line">
        <span className="ck-spaces-msg-author">{m.authorName}</span>
        {m.isFish && <span className="ck-spaces-bot-tag">Bot</span>}
        <time className="ck-spaces-msg-time" dateTime={new Date(m.createdAtMs).toISOString()}>{timeOf(m.createdAtMs)}</time>
        {!m.deleted && !menu && (
          <span className="ck-spaces-msg-actions">
            <button type="button" className="ck-spaces-more" onClick={() => onReply(m)}>Reply</button>
          </span>
        )}
        {menu && <span className="ck-spaces-msg-actions">{menu(m, () => onReply(m))}</span>}
      </div>
      {m.replyPreview && (
        <p className="ck-spaces-msg-reply">{m.replyPreview.authorName}: {m.replyPreview.text}</p>
      )}
      {m.deleted ? (
        <p className="ck-spaces-msg-text">
          {m.isMine && !m.deleted.byOwner ? 'You deleted this message.'
            : m.deleted.byOwner ? 'Deleted by the owner.' : `${m.deleted.byName} deleted this message.`}
        </p>
      ) : m.file ? (
        <FileCard file={m.file} error={fileError ?? null} savedAs={fileSavedAs ?? null}
          onGet={() => onGetFile?.(m)} onSave={() => onSaveFile?.(m)} />
      ) : (
        <p className={`ck-spaces-msg-text ${bubbleClass(m)}`}>{m.text}</p>
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
    </li>
  );
}
