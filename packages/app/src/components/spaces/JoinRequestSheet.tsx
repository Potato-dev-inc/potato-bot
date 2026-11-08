import { useId } from 'react';
import { Sheet } from '../cockpit/Sheet';
import type { SpaceJoinRequest } from '../../lib/spaces';
import '../../styles/cockpit-spaces.css';

export interface JoinRequestSheetProps {
  request: SpaceJoinRequest | null;
  onAnswer: (space: string, request: number, allow: boolean) => void;
}

/** Shows only when a request is waiting. Closing it (Escape or the scrim) is the same as No. */
export function JoinRequestSheet({ request, onAnswer }: JoinRequestSheetProps) {
  const titleId = useId();
  if (!request) return null;
  const no = () => onAnswer(request.space, request.id, false);
  return (
    <Sheet labelledBy={titleId} onClose={no} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">{request.name} wants to join {request.spaceName}</h2>
      <div className="ck-spaces-sheet-scroll">
        <p className="ck-spaces-sheet-body">They used your invite and typed the right words.</p>
        <div className="ck-spaces-group">
          <div className="ck-spaces-card">
            <p className="ck-spaces-note ck-spaces-row">Their computer: {request.short}</p>
          </div>
          <p className="ck-spaces-group-foot">
            Only let in people you know. They will be able to read this space's recent messages.
          </p>
        </div>
      </div>
      <div className="ck-sheet-actions">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={no}>No</button>
        <button type="button" className="ck-btn ck-btn--primary" onClick={() => onAnswer(request.space, request.id, true)}>
          Let in
        </button>
      </div>
    </Sheet>
  );
}
