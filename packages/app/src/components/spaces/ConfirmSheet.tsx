// One confirm sheet for every Spaces moderation action: title, plain words, Cancel and one bold button.
import { useId } from 'react';
import { Sheet } from '../cockpit/Sheet';

export interface ConfirmSheetProps {
  title: string;
  body: string;
  confirmLabel: string;
  onConfirm: () => void;
  onClose: () => void;
}

export function ConfirmSheet({ title, body, confirmLabel, onConfirm, onClose }: ConfirmSheetProps) {
  const titleId = useId();
  return (
    <Sheet labelledBy={titleId} onClose={onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">{title}</h2>
      <p className="ck-spaces-sheet-body">{body}</p>
      <div className="ck-sheet-actions">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={onClose}>Cancel</button>
        <button type="button" className="ck-btn ck-btn--primary is-danger" onClick={onConfirm}>{confirmLabel}</button>
      </div>
    </Sheet>
  );
}
