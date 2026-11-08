// Spaces: the actions on one person in the people column: Remove from space (owner), Block and Unblock (me only).
import { useCallback, useRef, useState } from 'react';
import type { SpaceMember } from '../../lib/spaces';
import { ConfirmSheet } from './ConfirmSheet';
import { useMenuDismiss } from './menuDismiss';

export interface MemberMenuProps {
  member: SpaceMember;
  spaceName: string;
  canRemove: boolean;
  onRemove: () => void;
  onBlock: (on: boolean) => void;
}

export function MemberMenu({ member, spaceName, canRemove, onRemove, onBlock }: MemberMenuProps) {
  const [open, setOpen] = useState(false);
  const [sheet, setSheet] = useState<'remove' | 'block' | null>(null);
  const box = useRef<HTMLSpanElement>(null);
  const more = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useMenuDismiss(open, box, more, close);
  if (member.isMe) return null;
  return (
    <span className="ck-spaces-menu ck-spaces-member-actions" ref={box}>
      <button type="button" ref={more} className="ck-spaces-more" aria-label={`Actions for ${member.name}`} aria-haspopup="menu"
        aria-expanded={open} onClick={() => setOpen(!open)}>More</button>
      {open && (
        <span role="menu" className="ck-spaces-menu-list">
          {canRemove && (
            <button type="button" role="menuitem" onClick={() => { setOpen(false); setSheet('remove'); }}>Remove from space</button>
          )}
          {member.blocked ? (
            <button type="button" role="menuitem" onClick={() => { setOpen(false); onBlock(false); }}>Unblock</button>
          ) : (
            <button type="button" role="menuitem" onClick={() => { setOpen(false); setSheet('block'); }}>Block</button>
          )}
        </span>
      )}
      {sheet === 'remove' && (
        <ConfirmSheet title={`Remove ${member.name} from ${spaceName}?`}
          body="They will stop getting new messages. Messages they already have stay on their computer."
          confirmLabel="Remove" onClose={() => setSheet(null)} onConfirm={() => { setSheet(null); onRemove(); }} />
      )}
      {sheet === 'block' && (
        <ConfirmSheet title={`Block ${member.name}?`}
          body={`You won't see their messages in ${spaceName}. They are not told. Bots won't answer them on your computer.`}
          confirmLabel="Block" onClose={() => setSheet(null)} onConfirm={() => { setSheet(null); onBlock(true); }} />
      )}
    </span>
  );
}
