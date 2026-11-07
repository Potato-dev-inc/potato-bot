// "Move bots": bring a bot in from a bot file, from ChatGPT or Claude, or from a backup folder, and back every bot up. Every way in
// shows what is inside first, and nothing is made until the person presses the button. The engine reads and cleans the files;
// a file adds only text (no tools, no accounts, no ask level) and routines come in paused.
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { useId, useState } from 'react';
import type { CompanionTransport } from '../../../lib/companion';
import { t } from '../../../lib/i18n';
import { plainError, type PotatoTransport } from '../../../lib/potato';
import {
  IMPORT_PROMPT,
  makeBotFromDrafts,
  makeBotFromFile,
  parseChatExport,
  PORTABLE_COPY,
  PROMPT_GUIDE,
  restoreAccount,
  tauriPortableTransport,
  type AccountPreview,
  type ChatDrafts,
  type PortableChoices,
  type PortablePreview,
  type PortableTransport,
} from '../../../lib/potato-portable';
import { Sheet } from '../Sheet';

export const MOVE_BOT_LIMIT = 10;
type Tab = 'file' | 'chat' | 'backup';

export interface MoveBotsSheetProps {
  companions: Pick<CompanionTransport, 'agents' | 'create'>;
  potato: Pick<PotatoTransport, 'addBot'>;
  portable?: PortableTransport;
  /** How many bots there are now; the sheet stops at ten. */
  botCount: number;
  /** A bot was made (the page opens it). */
  onMade: (botId: string) => void;
  onNotify: (text: string) => void;
  onClose: () => void;
}

export function MoveBotsSheet({ companions, potato, portable = tauriPortableTransport, botCount, onMade, onNotify, onClose }: MoveBotsSheetProps) {
  const titleId = useId();
  const [tab, setTab] = useState<Tab>('file');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const makers = { companions, potato, portable };
  const room = Math.max(0, MOVE_BOT_LIMIT - botCount);

  const guard = async (work: () => Promise<void>, fallback: string) => {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await work();
    } catch (e) {
      setError(plainError(e, fallback));
    } finally {
      setBusy(false);
    }
  };

  // ---- a bot file
  const [file, setFile] = useState<{ path: string; preview: PortablePreview } | null>(null);
  const [choices, setChoices] = useState<PortableChoices>({ instructions: true, skills: true, routines: true, memory: true });
  const [fileName, setFileName] = useState('');
  const pickFile = () =>
    guard(async () => {
      const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: 'Potato Bot file', extensions: ['json'] }] });
      if (typeof picked !== 'string') return;
      const preview = await portable.fileInspect(picked);
      setFile({ path: picked, preview });
      setFileName(preview.bot.name);
    }, 'Could not open that file.');
  const makeFromFile = () =>
    file &&
    guard(async () => {
      const id = await makeBotFromFile(makers, file.path, file.preview, choices, fileName);
      onNotify(PORTABLE_COPY.made(fileName));
      onMade(id);
    }, 'Could not make the bot.');

  // ---- ChatGPT or Claude
  const [pasted, setPasted] = useState('');
  const [drafts, setDrafts] = useState<ChatDrafts | null>(null);
  const [keep, setKeep] = useState<boolean[]>([]);
  const [chatName, setChatName] = useState('From my assistant');
  const readDrafts = (text: string) => {
    const found = parseChatExport(text);
    setDrafts(found);
    setKeep(found.memories.map(() => true));
  };
  const pickChatFile = () =>
    guard(async () => {
      const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: 'Export', extensions: ['json', 'md', 'txt'] }] });
      if (typeof picked !== 'string') return;
      readDrafts(await portable.readText(picked));
    }, 'Could not read that file.');
  const copyPrompt = async () => {
    try {
      await navigator.clipboard.writeText(IMPORT_PROMPT);
      setStatus(PORTABLE_COPY.promptCopied);
    } catch {
      setError('Could not copy. Select the text and copy it by hand.');
    }
  };
  const makeFromDrafts = () =>
    drafts &&
    guard(async () => {
      const memories = drafts.memories.filter((_, i) => keep[i]);
      const id = await makeBotFromDrafts(makers, chatName, drafts.instructions, memories);
      onNotify(PORTABLE_COPY.made(chatName));
      onMade(id);
    }, 'Could not make the bot.');

  // ---- backups
  const [withChats, setWithChats] = useState(true);
  const [backup, setBackup] = useState<{ dir: string; preview: AccountPreview } | null>(null);
  const makeBackup = () =>
    guard(async () => {
      const parent = await openDialog({ multiple: false, directory: true });
      if (typeof parent !== 'string') return;
      setStatus(PORTABLE_COPY.backedUp(await portable.accountExport(parent, withChats)));
    }, 'Could not make the backup.');
  const pickBackup = () =>
    guard(async () => {
      const dir = await openDialog({ multiple: false, directory: true });
      if (typeof dir !== 'string') return;
      setBackup({ dir, preview: await portable.accountInspect(dir) });
    }, 'Could not open that folder.');
  const restore = () =>
    backup &&
    guard(async () => {
      const { made, failed } = await restoreAccount(makers, backup.dir, backup.preview, room);
      if (failed.length > 0) setError(failed.join(' '));
      if (made.length > 0) {
        onNotify(PORTABLE_COPY.restored(made.length));
        onMade(made[0]);
      }
      if (made.length === 0 && failed.length === 0) setStatus(PORTABLE_COPY.needsRoom);
    }, 'Could not restore.');

  const tickRow = (key: keyof PortableChoices, label: string, count: number | null) => (
    <label className="ck-potato-adv-check" key={key}>
      <input type="checkbox" checked={choices[key]} onChange={(e) => setChoices((was) => ({ ...was, [key]: e.target.checked }))} />
      <span>
        <span className="ck-potato-adv-check-title">{label}</span>
        {count !== null && <span className="ck-potato-adv-line">{count}</span>}
      </span>
    </label>
  );

  return (
    <Sheet role="dialog" labelledBy={titleId} onClose={busy ? undefined : onClose} className="ck-potato-move">
      <h2 id={titleId} className="ck-potato-adv-sheet-title">
        {PORTABLE_COPY.moveTitle}
      </h2>
      <p className="ck-sub">{PORTABLE_COPY.moveIntro}</p>
      <div role="tablist" className="ck-potato-move-tabs">
        {(
          [
            ['file', PORTABLE_COPY.tabFile],
            ['chat', PORTABLE_COPY.tabChat],
            ['backup', PORTABLE_COPY.tabBackup],
          ] as const
        ).map(([id, label]) => (
          <button key={id} type="button" role="tab" aria-selected={tab === id} className={`ck-btn ck-btn--sm ${tab === id ? 'ck-btn--secondary' : 'ck-btn--plain'}`} disabled={busy} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {botCount >= MOVE_BOT_LIMIT && tab !== 'backup' && <p className="ck-sub">{PORTABLE_COPY.needsRoom}</p>}

      {tab === 'file' && (
        <section aria-label={PORTABLE_COPY.tabFile}>
          <p className="ck-sub">{PORTABLE_COPY.fileHint}</p>
          <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void pickFile()}>
            {PORTABLE_COPY.importFile}
          </button>
          {file && (
            <div className="ck-potato-move-preview" data-testid="file-preview">
              <h3>{PORTABLE_COPY.contains}</h3>
              <label className="ck-potato-adv-field">
                <span className="ck-potato-adv-line">{PORTABLE_COPY.botName}</span>
                <input className="ck-potato-adv-input" type="text" value={fileName} maxLength={24} onChange={(e) => setFileName(e.target.value)} />
              </label>
              {tickRow('instructions', PORTABLE_COPY.tickInstructions, file.preview.instructions ? file.preview.instructions.length : 0)}
              {file.preview.instructions && <pre className="ck-potato-move-text">{file.preview.instructions}</pre>}
              {tickRow('skills', PORTABLE_COPY.tickSkills, file.preview.skills.length)}
              <ul className="ck-potato-move-list">
                {file.preview.skills.map((s) => (
                  <li key={s.name}>{s.name}</li>
                ))}
              </ul>
              {tickRow('routines', PORTABLE_COPY.tickRoutines, file.preview.routines.length)}
              <ul className="ck-potato-move-list">
                {file.preview.routines.map((r) => (
                  <li key={r.title}>
                    {r.title} <span className="ck-sub">{r.summary}</span>
                  </li>
                ))}
              </ul>
              {tickRow('memory', PORTABLE_COPY.tickMemory, file.preview.memory.length)}
              {file.preview.notes.map((n) => (
                <p key={n} className="ck-sub">
                  {t(n)}
                </p>
              ))}
              <p className="ck-sub">{file.preview.promise}</p>
              <button type="button" className="ck-btn ck-btn--primary" disabled={busy || room === 0 || fileName.trim() === ''} onClick={() => void makeFromFile()}>
                {busy ? PORTABLE_COPY.making : PORTABLE_COPY.makeBot}
              </button>
            </div>
          )}
        </section>
      )}

      {tab === 'chat' && (
        <section aria-label={PORTABLE_COPY.tabChat}>
          <p className="ck-sub">{PORTABLE_COPY.chatIntro}</p>
          <details>
            <summary>{PORTABLE_COPY.guideTitle}</summary>
            <h3>ChatGPT</h3>
            <ol>
              {PROMPT_GUIDE.chatgpt.map((line) => (
                <li key={line}>{t(line)}</li>
              ))}
            </ol>
            <h3>Claude</h3>
            <ol>
              {PROMPT_GUIDE.claude.map((line) => (
                <li key={line}>{t(line)}</li>
              ))}
            </ol>
            <p className="ck-sub">{t(PROMPT_GUIDE.note)}</p>
            <button type="button" className="ck-btn ck-btn--plain ck-btn--sm" onClick={() => void copyPrompt()}>
              {PORTABLE_COPY.copyPrompt}
            </button>
          </details>
          <label className="ck-potato-adv-field">
            <span className="ck-potato-adv-line">{PORTABLE_COPY.pasteLabel}</span>
            <textarea className="ck-potato-adv-input" rows={6} value={pasted} placeholder={PORTABLE_COPY.pastePlaceholder} onChange={(e) => setPasted(e.target.value)} />
          </label>
          <div className="ck-potato-adv-actions">
            <button type="button" className="ck-btn ck-btn--secondary" disabled={busy || pasted.trim() === ''} onClick={() => readDrafts(pasted)}>
              {PORTABLE_COPY.readIt}
            </button>
            <button type="button" className="ck-btn ck-btn--plain" disabled={busy} onClick={() => void pickChatFile()}>
              {PORTABLE_COPY.openChatFile}
            </button>
          </div>
          {drafts && drafts.instructions === '' && drafts.memories.length === 0 && <p className="ck-sub">{PORTABLE_COPY.noDrafts}</p>}
          {drafts && (drafts.instructions !== '' || drafts.memories.length > 0) && (
            <div className="ck-potato-move-preview" data-testid="draft-preview">
              <h3>{PORTABLE_COPY.draftsTitle}</h3>
              <label className="ck-potato-adv-field">
                <span className="ck-potato-adv-line">{PORTABLE_COPY.botName}</span>
                <input className="ck-potato-adv-input" type="text" value={chatName} maxLength={24} onChange={(e) => setChatName(e.target.value)} />
              </label>
              <label className="ck-potato-adv-field">
                <span className="ck-potato-adv-line">{PORTABLE_COPY.draftInstructions}</span>
                <textarea className="ck-potato-adv-input" rows={6} value={drafts.instructions} onChange={(e) => setDrafts({ ...drafts, instructions: e.target.value })} />
              </label>
              <h4>{PORTABLE_COPY.draftMemories}</h4>
              {drafts.memories.map((m, i) => (
                <label className="ck-potato-adv-check" key={`${i}-${m}`}>
                  <input type="checkbox" checked={keep[i] ?? false} onChange={(e) => setKeep((was) => was.map((v, j) => (j === i ? e.target.checked : v)))} />
                  <span className="ck-potato-adv-line">{m}</span>
                </label>
              ))}
              <button type="button" className="ck-btn ck-btn--primary" disabled={busy || room === 0 || chatName.trim() === ''} onClick={() => void makeFromDrafts()}>
                {busy ? PORTABLE_COPY.making : PORTABLE_COPY.makeBot}
              </button>
            </div>
          )}
        </section>
      )}

      {tab === 'backup' && (
        <section aria-label={PORTABLE_COPY.tabBackup}>
          <h3>{PORTABLE_COPY.backupTitle}</h3>
          <p className="ck-sub">{PORTABLE_COPY.backupHint}</p>
          <label className="ck-potato-adv-check">
            <input type="checkbox" checked={withChats} onChange={(e) => setWithChats(e.target.checked)} />
            <span className="ck-potato-adv-check-title">{PORTABLE_COPY.backupMakeWithChats}</span>
          </label>
          <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void makeBackup()}>
            {PORTABLE_COPY.backupMake}
          </button>
          <h3>{PORTABLE_COPY.restoreTitle}</h3>
          <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void pickBackup()}>
            {PORTABLE_COPY.restoreOpen}
          </button>
          {backup && (
            <div className="ck-potato-move-preview" data-testid="backup-preview">
              <ul className="ck-potato-move-list">
                {backup.preview.bots.map((b) => (
                  <li key={b.id}>
                    {b.bot.name} <span className="ck-sub">{t('{skills} skills, {routines} routines, {notes} notes, {messages} messages', { skills: b.skills, routines: b.routines, notes: b.notes, messages: b.messages })}</span>
                  </li>
                ))}
              </ul>
              {backup.preview.notes.map((n) => (
                <p key={n} className="ck-sub">
                  {t(n)}
                </p>
              ))}
              <p className="ck-sub">{backup.preview.promise}</p>
              <button type="button" className="ck-btn ck-btn--primary" disabled={busy || room === 0 || backup.preview.bots.length === 0} onClick={() => void restore()}>
                {busy ? PORTABLE_COPY.restoring : PORTABLE_COPY.restoreAll}
              </button>
            </div>
          )}
        </section>
      )}

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
      <div className="ck-potato-adv-actions">
        <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={onClose}>
          {PORTABLE_COPY.close}
        </button>
      </div>
    </Sheet>
  );
}
