// Spaces (Labs `spaces_enabled`): the New space sheet. The name is the only field.
import { useId, useState } from 'react';
import { Sheet } from '../cockpit/Sheet';

export interface NewSpaceSheetProps {
  /** Returns the new space id, or null when it failed (the page shows `error`). */
  onCreate: (name: string) => Promise<string | null>;
  error: string | null;
  onClose: () => void;
}

export function NewSpaceSheet({ onCreate, error, onClose }: NewSpaceSheetProps) {
  const titleId = useId();
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const ready = !busy && name.trim() !== '';
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={busy ? undefined : onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">
        New space
      </h2>
      <label className="ck-spaces-field">
        <span>Name</span>
        <input value={name} maxLength={40} placeholder="Book club" onChange={(e) => setName(e.target.value)} />
        <small className="ck-spaces-hint">You can change it later.</small>
      </label>
      {error && (
        <p className="ck-spaces-hint is-error" role="alert">
          {error}
        </p>
      )}
      <div className="ck-spaces-actions">
        <button type="button" className="ck-spaces-btn" onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button
          type="button"
          className="ck-spaces-btn is-primary"
          disabled={!ready}
          onClick={async () => {
            setBusy(true);
            const id = await onCreate(name.trim());
            setBusy(false);
            if (id) onClose();
          }}
        >
          Create
        </button>
      </div>
    </Sheet>
  );
}
