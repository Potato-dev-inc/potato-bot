// Spaces: asks for the name people will see, before the first space is made or joined. One field, same sheet as the others.
import { useId, useState } from 'react';
import { Sheet } from '../cockpit/Sheet';

export interface DisplayNameSheetProps {
  /** Returns true when the name was saved (the page shows `error` otherwise). */
  onSave: (name: string) => Promise<boolean>;
  error: string | null;
  onClose: () => void;
}

export function DisplayNameSheet({ onSave, error, onClose }: DisplayNameSheetProps) {
  const titleId = useId();
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const ready = !busy && name.trim() !== '';
  const save = async () => {
    if (!ready) return;
    setBusy(true);
    const ok = await onSave(name.trim());
    setBusy(false);
    if (ok) onClose();
  };
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={busy ? undefined : onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">
        Your name in Spaces
      </h2>
      <div className="ck-spaces-sheet-scroll">
        <label className="ck-spaces-field">
          <span>Name</span>
          <input
            value={name}
            maxLength={40}
            placeholder="Sam"
            autoFocus
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') void save(); }}
          />
          <small className="ck-spaces-hint">This is what other people in your spaces see. You can change it in Settings.</small>
        </label>
      </div>
      {error && (
        <p className="ck-spaces-sheet-status is-error" role="alert">
          {error}
        </p>
      )}
      <div className="ck-sheet-actions">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button type="button" className="ck-btn ck-btn--primary" disabled={!ready} onClick={() => void save()}>
          Continue
        </button>
      </div>
    </Sheet>
  );
}
