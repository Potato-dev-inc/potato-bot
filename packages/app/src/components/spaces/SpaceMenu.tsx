// Spaces: the header's small menu. It holds the actions that are rarely wanted (Leave space), one tab stop away.
import { useCallback, useRef, useState } from 'react';
import { useMenuDismiss } from './menuDismiss';

export interface SpaceMenuProps {
  canLeave: boolean;
  /** Why Leave is off (the owner), shown in the menu and as the item's tooltip. */
  leaveBlocked?: string;
  onLeave: () => void;
}

export function SpaceMenu({ canLeave, leaveBlocked, onLeave }: SpaceMenuProps) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  const more = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useMenuDismiss(open, box, more, close);
  return (
    <span className="ck-spaces-menu" ref={box}>
      <button type="button" ref={more} className="ck-spaces-btn" aria-label="Space actions" aria-haspopup="menu" aria-expanded={open}
        onClick={() => setOpen(!open)}>More</button>
      {open && (
        <span role="menu" className="ck-spaces-menu-list ck-spaces-menu-head">
          <button type="button" role="menuitem" disabled={!canLeave} title={leaveBlocked}
            onClick={() => { setOpen(false); onLeave(); }}>Leave space</button>
          {leaveBlocked && <span className="ck-spaces-menu-note">{leaveBlocked}</span>}
        </span>
      )}
    </span>
  );
}
