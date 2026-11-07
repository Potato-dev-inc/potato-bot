import { useId, useState } from 'react';
import { Sheet } from '../cockpit/Sheet';

export interface NewChannelSheetProps {
  /** Returns an error text to show, or null when the channel was made. */
  onCreate: (name: string) => Promise<string | null>;
  onClose: () => void;
}

/** Same rule as the Rust `clean_channel_name`, for the live hint only. The Rust rule decides. */
export function previewChannelName(raw: string): string {
  const s = Array.from(raw.trim().toLowerCase())
    .map((c) => (/\s/.test(c) ? '-' : c))
    .filter((c) => /^[a-z0-9-]$/.test(c))
    .join('')
    .replace(/^-+|-+$/g, '');
  return Array.from(s).slice(0, 32).join('');
}

export function NewChannelSheet({ onCreate, onClose }: NewChannelSheetProps) {
  const titleId = useId();
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const preview = previewChannelName(name);
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">
        New channel
      </h2>
      <label className="ck-spaces-field">
        <span>Name</span>
        <input value={name} placeholder="books" onChange={(e) => setName(e.target.value)} />
        <small className="ck-spaces-hint">Lowercase letters, numbers and dashes.</small>
      </label>
      {preview !== '' && <p className="ck-spaces-note"># {preview}</p>}
      {error && (
        <p className="ck-spaces-hint is-error" role="alert">
          {error}
        </p>
      )}
      <div className="ck-spaces-actions">
        <button type="button" className="ck-spaces-btn" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="ck-spaces-btn is-primary"
          disabled={preview === ''}
          onClick={async () => {
            const e = await onCreate(name);
            if (e) setError(e);
            else onClose();
          }}
        >
          Create
        </button>
      </div>
    </Sheet>
  );
}
