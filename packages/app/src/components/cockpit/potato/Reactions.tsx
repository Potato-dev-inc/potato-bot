// Emoji reactions on a message of a bot's thread: a React button in the hover toolbar that opens a small picker of the
// fixed set, and the reactions under the message (press one to take it back). Grayscale; the emoji are the only colour.
import { useEffect, useRef, useState } from 'react';
import { REACTIONS } from '../../../lib/potato';
import { THREAD_COPY } from '../../../lib/potato-thread';

export interface ReactButtonProps {
  /** The reactions already on the message: a picked one shows as pressed. */
  current: readonly string[];
  onPick: (emoji: string) => void;
}

export function ReactButton({ current, onPick }: ReactButtonProps) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: Event) => {
      if (box.current && e.target instanceof Node && !box.current.contains(e.target)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', key);
    return () => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', key);
    };
  }, [open]);
  return (
    <span className="ck-potato-react" ref={box}>
      <button type="button" className="ck-potato-action" aria-label={`${THREAD_COPY.react} to message`} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        {THREAD_COPY.react}
      </button>
      {open && (
        <div className="ck-potato-react-menu" role="menu" aria-label={THREAD_COPY.reactPick}>
          {REACTIONS.map((emoji) => (
            <button
              key={emoji}
              type="button"
              role="menuitem"
              className="ck-potato-react-pick"
              aria-pressed={current.includes(emoji)}
              onClick={() => {
                setOpen(false);
                onPick(emoji);
              }}
            >
              {emoji}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}

export function ReactionChips({ reactions, onToggle }: { reactions: readonly string[]; onToggle: (emoji: string) => void }) {
  if (reactions.length === 0) return null;
  return (
    <div className="ck-potato-reactions" role="group" aria-label={THREAD_COPY.reactions}>
      {reactions.map((emoji) => (
        <button key={emoji} type="button" className="ck-potato-reaction" aria-pressed="true" aria-label={`Take back ${emoji}`} onClick={() => onToggle(emoji)}>
          {emoji}
        </button>
      ))}
    </div>
  );
}
