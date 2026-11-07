// "Export bot" and "Duplicate bot" in the Advanced tab's Bot card. Export asks where to save (the engine writes the file and never
// puts a key, token, connector sign-in or cookie in it); Duplicate makes a second bot with the first one's instructions, skills,
// memory and routines (paused). Both say what they did in one line.
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import { useState } from 'react';
import type { CompanionTransport, CompanionView } from '../../../lib/companion';
import { plainError, type PotatoTransport } from '../../../lib/potato';
import { duplicateBot, PORTABLE_COPY, tauriPortableTransport, type PortableTransport } from '../../../lib/potato-portable';

export interface BotExportButtonsProps {
  bot: { id: string; name: string };
  portable?: PortableTransport;
  /** Duplicate needs these to make the second bot. Without them the button is not drawn. */
  companion?: CompanionView | null;
  companionTransport?: CompanionTransport;
  potatoTransport?: PotatoTransport;
  onChanged?: () => void;
  onNotify?: (text: string) => void;
}

export function BotExportButtons({ bot, portable = tauriPortableTransport, companion, companionTransport, potatoTransport, onChanged, onNotify }: BotExportButtonsProps) {
  const [memory, setMemory] = useState(false);
  const [busy, setBusy] = useState<'export' | 'duplicate' | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const exportBot = async () => {
    setBusy('export');
    setError(null);
    setStatus(null);
    try {
      const name = await portable.exportName(bot.id);
      const picked = await saveDialog({ defaultPath: name, filters: [{ name: 'Potato Bot file', extensions: ['json'] }] });
      if (typeof picked === 'string') setStatus(PORTABLE_COPY.exported(await portable.exportSave(bot.id, memory, picked)));
    } catch (e) {
      setError(plainError(e, 'Could not export the bot.'));
    } finally {
      setBusy(null);
    }
  };

  const duplicate = async () => {
    if (!companionTransport || !potatoTransport) return;
    setBusy('duplicate');
    setError(null);
    setStatus(null);
    try {
      await duplicateBot({ companions: companionTransport, potato: potatoTransport, portable }, { id: bot.id, name: bot.name, avatar: companion?.avatar, persona: companion?.persona });
      onChanged?.();
      const line = PORTABLE_COPY.duplicated(`${bot.name}${PORTABLE_COPY.copySuffix}`);
      setStatus(line);
      onNotify?.(line);
    } catch (e) {
      setError(plainError(e, 'Could not duplicate the bot.'));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <label className="ck-potato-adv-check">
        <input type="checkbox" checked={memory} onChange={(event) => setMemory(event.target.checked)} />
        <span>
          <span className="ck-potato-adv-check-title">{PORTABLE_COPY.exportMemory}</span>
          <span className="ck-potato-adv-line">{PORTABLE_COPY.exportHint}</span>
        </span>
      </label>
      <div className="ck-potato-adv-actions ck-potato-adv-stack">
        <button type="button" className="ck-btn ck-btn--secondary" disabled={busy !== null} onClick={() => void exportBot()}>
          {busy === 'export' ? PORTABLE_COPY.exporting : PORTABLE_COPY.exportBot}
        </button>
        {companionTransport && potatoTransport && (
          <button type="button" className="ck-btn ck-btn--secondary" disabled={busy !== null} onClick={() => void duplicate()}>
            {busy === 'duplicate' ? PORTABLE_COPY.duplicating : PORTABLE_COPY.duplicate}
          </button>
        )}
      </div>
      {status && (
        <p className="ck-potato-adv-status" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="ck-potato-adv-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
