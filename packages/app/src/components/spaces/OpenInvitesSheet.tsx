import { useId } from 'react';
import { Sheet } from '../cockpit/Sheet';
import { timeLeftText } from '../../lib/questions';
import type { SpaceInvite } from '../../lib/spaces';
import '../../styles/cockpit-spaces.css';

export const MAX_OPEN_INVITES = 3;
export const INVITES_FULL = 'Three invites are already open. Cancel one first.';
export const INVITE_NOT_SHOWN_AGAIN = "An invite made earlier can't show its link and 8 words again. Cancel it and make a new one if you need them.";

export interface OpenInvitesSheetProps {
  spaceName: string;
  /** This space's invites that still work. */
  invites: SpaceInvite[];
  /** Invites made since this window opened: their link and words are still known and can be shown. */
  known: Record<string, SpaceInvite>;
  nowMs: number;
  error: string | null;
  onShow: (invite: SpaceInvite) => void;
  onCancelInvite: (invite: SpaceInvite) => void;
  onNew: () => void;
  onClose: () => void;
}

/** Opens when the space already has an invite that works: lists them with a way to cancel each, instead of making another. */
export function OpenInvitesSheet({ spaceName, invites, known, nowMs, error, onShow, onCancelInvite, onNew, onClose }: OpenInvitesSheetProps) {
  const titleId = useId();
  const full = invites.length >= MAX_OPEN_INVITES;
  const allKnown = invites.every((i) => known[i.id]);
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">Open invites for {spaceName}</h2>
      <div className="ck-spaces-sheet-scroll">
        {invites.length === 0 ? (
          <p className="ck-spaces-sheet-body">No invite is open.</p>
        ) : (
          <>
            {!allKnown && <p className="ck-spaces-sheet-body">{INVITE_NOT_SHOWN_AGAIN}</p>}
            <div className="ck-spaces-group">
              <ul className="ck-spaces-card" aria-label="Open invites">
                {invites.map((i, n) => (
                  <li key={i.id} className="ck-spaces-row ck-spaces-invite-row">
                    <span className="ck-spaces-row-text">
                      Invite {n + 1}. Expires in {timeLeftText(Math.max(0, i.expiresAtMs - nowMs))}.
                    </span>
                    {known[i.id] && (
                      <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => onShow(known[i.id])}>
                        Show link and words
                      </button>
                    )}
                    <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm is-danger" onClick={() => onCancelInvite(i)}>
                      Cancel invite
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          </>
        )}
        {full && <p className="ck-spaces-sheet-status is-error" role="status">{INVITES_FULL}</p>}
        {error && !full && <p className="ck-spaces-sheet-status is-error" role="alert">{error}</p>}
      </div>
      <div className="ck-sheet-actions">
        <button type="button" className="ck-btn ck-btn--secondary ck-spaces-actions-start" disabled={full} onClick={onNew}>New invite</button>
        <button type="button" className="ck-btn ck-btn--primary" onClick={onClose}>Close</button>
      </div>
    </Sheet>
  );
}
