import { useId, useMemo, useState } from 'react';
import { Sheet } from '../cockpit/Sheet';
import { joinStateText, type SpaceJoining } from '../../lib/spaces';
import '../../styles/cockpit-spaces.css';

export interface JoinSheetProps {
  joining: SpaceJoining | null;
  error: string | null;
  onJoin: (text: string, words: string) => void;
  onCancel: (space: string | null) => void;
  onOpenSpace: (space: string) => void;
}

/** Splits like the backend: spaces, dashes, commas, dots, new lines. */
export function countWords(text: string): number {
  return text
    .toLowerCase()
    .split(/[\s\-,.]+/)
    .filter((w) => w !== '').length;
}

export function JoinSheet({ joining, error, onJoin, onCancel, onOpenSpace }: JoinSheetProps) {
  const titleId = useId();
  const [link, setLink] = useState('');
  const [words, setWords] = useState('');
  // A QR code carries the link and the words joined by '#'. Its words fill the field when the field is empty.
  const qrWords = useMemo(() => (link.includes('#') ? (link.split('#')[1] ?? '') : ''), [link]);
  const effectiveWords = words.trim() === '' ? qrWords : words;
  const n = countWords(effectiveWords);
  const linkOk = link.trim().startsWith('pms1:');
  const busy = joining?.state === 'connecting' || joining?.state === 'waiting';
  const canAsk = linkOk && n === 8 && !busy;

  if (joining?.state === 'joined') {
    return (
      <Sheet labelledBy={titleId} role="dialog" onClose={() => onCancel(null)} className="ck-spaces-sheet">
        <h2 id={titleId} className="ck-spaces-sheet-title">{joinStateText(joining)}</h2>
        <div className="ck-sheet-actions">
          <button type="button" className="ck-btn ck-btn--primary" onClick={() => onOpenSpace(joining.space)}>
            Open space
          </button>
        </div>
      </Sheet>
    );
  }

  const statusIsError = !(joining?.state === 'waiting' || joining?.state === 'connecting');
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={() => onCancel(joining?.space ?? null)} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">Join a space</h2>
      <div className="ck-spaces-sheet-scroll">
        <label className="ck-spaces-field">
          <span>Invite link</span>
          <input value={link} placeholder="pms1:…" disabled={busy} onChange={(e) => setLink(e.target.value)} />
          <small className="ck-spaces-hint">Paste the link you got.</small>
        </label>
        {link.trim() !== '' && !linkOk && (
          <p className="ck-spaces-hint is-error">That doesn't look like a Spaces invite link.</p>
        )}
        <label className="ck-spaces-field">
          <span>Invite words</span>
          <textarea
            rows={2}
            value={words}
            placeholder={qrWords !== '' ? 'Filled in from the QR code' : '8 words, in order'}
            disabled={busy}
            onChange={(e) => setWords(e.target.value)}
          />
          <small className="ck-spaces-hint">Type the 8 words you were told. Spaces or dashes between them are fine.</small>
        </label>
        {effectiveWords.trim() !== '' && n !== 8 && (
          <p className="ck-spaces-hint">Type all 8 words. You typed {n}.</p>
        )}
      </div>
      {joining && (
        <p className={`ck-spaces-sheet-status${statusIsError ? ' is-error' : ''}`} role="status">
          {joinStateText(joining)}
        </p>
      )}
      {error && <p className="ck-spaces-sheet-status is-error" role="alert">{error}</p>}
      <div className="ck-sheet-actions">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={() => onCancel(joining?.space ?? null)}>
          Cancel
        </button>
        <button
          type="button"
          className="ck-btn ck-btn--primary"
          disabled={!canAsk}
          onClick={() => onJoin(link.trim(), effectiveWords)}
        >
          Ask to join
        </button>
      </div>
    </Sheet>
  );
}
