// Spaces: the actions on one message. Only Reply, React and Delete exist; nothing here runs, opens or approves
// anything that a message says (cowork M2).
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { REACTION_LABEL, type ReactionCode, type SpaceMessage } from '../../lib/spaces';
import { ConfirmSheet } from './ConfirmSheet';
import { REACTION_PICTURE } from './MessageRow';

export interface MessageMenuProps {
  message: SpaceMessage;
  canDelete: boolean;
  onReply: () => void;
  onReact: (code: ReactionCode) => void;
  onDelete: () => void;
}

export function MessageMenu({ message, canDelete, onReply, onReact, onDelete }: MessageMenuProps) {
  const [open, setOpen] = useState(false);
  const [picking, setPicking] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  const more = useRef<HTMLButtonElement>(null);
  // Opens upward when the list would run past the bottom of the feed (the feed is the scroll container and clips it).
  const [up, setUp] = useState(false);
  const close = () => { setOpen(false); setPicking(false); };
  // Measured before paint, while the list is still in its downward position (it is unmounted when closed, and up is reset then).
  useLayoutEffect(() => {
    if (!open) { setUp(false); return; }
    const list = box.current?.querySelector<HTMLElement>('.ck-spaces-menu-list');
    const feed = box.current?.closest<HTMLElement>('.ck-spaces-msgs');
    if (!list || !feed) return;
    setUp(list.getBoundingClientRect().bottom > feed.getBoundingClientRect().bottom);
  }, [open]);
  // Escape closes the menu and gives focus back to More; a click elsewhere closes it too.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      setPicking(false);
      more.current?.focus();
    };
    const onDown = (e: MouseEvent) => {
      if (e.target instanceof Node && !box.current?.contains(e.target)) { setOpen(false); setPicking(false); }
    };
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('mousedown', onDown);
    return () => { document.removeEventListener('keydown', onKey, true); document.removeEventListener('mousedown', onDown); };
  }, [open]);
  if (message.deleted) return null;
  return (
    <span className="ck-spaces-menu" ref={box}>
      <button type="button" ref={more} className="ck-spaces-more" aria-label="Message actions" aria-haspopup="menu" aria-expanded={open}
        onClick={() => { setOpen(!open); setPicking(false); }}>More</button>
      {open && (
        <span role="menu" className={up ? 'ck-spaces-menu-list is-up' : 'ck-spaces-menu-list'}>
          <button type="button" role="menuitem" onClick={() => { close(); onReply(); }}>Reply</button>
          <button type="button" role="menuitem" aria-expanded={picking} onClick={() => setPicking(!picking)}>React</button>
          {picking && (
            <span role="group" aria-label="Pick a reaction" className="ck-spaces-picker">
              {(Object.keys(REACTION_LABEL) as ReactionCode[]).map((c) => (
                <button key={c} type="button" aria-label={REACTION_LABEL[c]} title={REACTION_LABEL[c]}
                  onClick={() => { close(); onReact(c); }}>{REACTION_PICTURE[c]}</button>
              ))}
            </span>
          )}
          {canDelete && (
            <button type="button" role="menuitem" onClick={() => { close(); setConfirm(true); }}>Delete</button>
          )}
        </span>
      )}
      {confirm && (
        <ConfirmSheet title="Delete this message?"
          body="It disappears for everyone in this space. People who already saw it may still have a copy."
          confirmLabel="Delete" onClose={() => setConfirm(false)} onConfirm={() => { setConfirm(false); onDelete(); }} />
      )}
    </span>
  );
}
