import { useEffect, useId, useState } from 'react';
import QRCode from 'qrcode';
import { Sheet } from '../cockpit/Sheet';
import { timeLeftText } from '../../lib/questions';
import type { SpaceInvite } from '../../lib/spaces';
import '../../styles/cockpit-spaces.css';

export interface InviteSheetProps {
  spaceName: string;
  invite: SpaceInvite;
  nowMs: number;
  onCancelInvite: () => void;
  onClose: () => void;
}

export function InviteSheet({ spaceName, invite, nowMs, onCancelInvite, onClose }: InviteSheetProps) {
  const titleId = useId();
  const [copied, setCopied] = useState(false);
  const [showQr, setShowQr] = useState(false);
  const [qrSrc, setQrSrc] = useState<string | null>(null);
  const left = Math.max(0, invite.expiresAtMs - nowMs);
  const open = invite.state === 'open' && left > 0;

  useEffect(() => {
    if (!showQr || !open) return;
    let alive = true;
    QRCode.toDataURL(invite.qr, { width: 192, margin: 1 }).then(
      (src: string) => {
        if (alive) setQrSrc(src);
      },
      () => {
        if (alive) setQrSrc(null);
      },
    );
    return () => {
      alive = false;
    };
  }, [showQr, open, invite.qr]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(invite.link);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">Invite someone to {spaceName}</h2>
      {open ? (
        <>
          <div className="ck-spaces-sheet-scroll">
            <p className="ck-spaces-sheet-body">
              Send them this link. Then tell them the 8 words another way: say them out loud, or send them in a
              different app.
            </p>
            <div className="ck-spaces-group">
              <div className="ck-spaces-card">
                <label className="ck-spaces-field">
                  <span>Invite link</span>
                  <input readOnly value={invite.link} onFocus={(e) => e.currentTarget.select()} />
                </label>
                <div className="ck-spaces-row is-actions">
                  <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => void copy()}>
                    {copied ? 'Copied' : 'Copy link'}
                  </button>
                  <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => setShowQr((v) => !v)}>
                    {showQr ? 'Hide QR code' : 'Show QR code'}
                  </button>
                </div>
              </div>
            </div>
            <div className="ck-spaces-group">
              <p className="ck-spaces-group-title">Invite words</p>
              <ol className="ck-spaces-words" aria-label="Invite words">
                {invite.words.map((w, i) => (
                  <li key={i} className="ck-spaces-word">{w}</li>
                ))}
              </ol>
            </div>
            {showQr && (
              <div className="ck-spaces-qr">
                {qrSrc && <img src={qrSrc} alt="Invite QR code" />}
                <p className="ck-spaces-hint">
                  Scan this with the other computer's camera. The code holds the link and the words.
                </p>
              </div>
            )}
            <p className="ck-spaces-group-foot">
              Works once. Expires in {timeLeftText(left)}. After 5 wrong tries it stops working.
            </p>
          </div>
          <div className="ck-sheet-actions">
            <button type="button" className="ck-btn ck-btn--secondary ck-spaces-actions-start is-danger" onClick={onCancelInvite}>Cancel invite</button>
            <button type="button" className="ck-btn ck-btn--primary" onClick={onClose}>Done</button>
          </div>
        </>
      ) : (
        <>
          <p className="ck-spaces-sheet-body">{inviteEndedText(invite, left)}</p>
          <div className="ck-sheet-actions">
            <button type="button" className="ck-btn ck-btn--primary" onClick={onClose}>Close</button>
          </div>
        </>
      )}
    </Sheet>
  );
}

export function inviteEndedText(invite: SpaceInvite, leftMs: number): string {
  switch (invite.state) {
    case 'pending': return 'Someone used this invite. Answer their request to finish.';
    case 'used': return 'This invite was used.';
    case 'dead': return leftMs <= 0 ? 'This invite expired.' : 'This invite stopped working after 5 wrong tries.';
    case 'cancelled': return 'Invite cancelled.';
    case 'open': return 'This invite expired.';
  }
}
