// The note that says a bot changed one of its own settings because its owner asked (name, instructions, persona or model):
// what changed, and an Undo button that puts the old value back. Once used it says Undone. The change is the app's record, not
// the bot's: only this button undoes it.
import { useState } from 'react';
import { plainError, type PotatoMessage, type PotatoTransport } from '../../../lib/potato';

export const CHANGE_COPY = {
  undo: 'Undo',
  undone: 'Undone',
  failed: 'Could not undo this change.',
} as const;

export interface SettingChangeNoteProps {
  message: PotatoMessage;
  botId: string;
  transport: Pick<PotatoTransport, 'undoSetting'>;
  onChanged: () => void;
}

export function SettingChangeNote({ message, botId, transport, onChanged }: SettingChangeNoteProps) {
  const change = message.change!;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const undo = async () => {
    if (!transport.undoSetting) return;
    setBusy(true);
    setError(null);
    try {
      await transport.undoSetting(botId, change.id);
      onChanged();
    } catch (e) {
      setError(plainError(e, CHANGE_COPY.failed));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      {message.text}{' '}
      {change.undone ? (
        <span className="ck-potato-change-done">{CHANGE_COPY.undone}</span>
      ) : (
        transport.undoSetting && (
          <button type="button" className="ck-potato-note-open" disabled={busy} onClick={() => void undo()}>
            {CHANGE_COPY.undo}
          </button>
        )
      )}
      {error && (
        <span className="ck-potato-change-error" role="alert">
          {' '}
          {error}
        </span>
      )}
    </>
  );
}
