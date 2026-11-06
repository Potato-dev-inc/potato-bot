// The group settings screen: rename the group, choose who answers (Listen or Mentions), add and remove members, switch a
// member off in this group only, write instructions every bot of the group reads, and name the shared folder. Nothing is
// saved until Save; one bad part saves nothing (the engine checks the same things, `rstore::apply_settings`). Escape cancels.
// The overlay and panel classes are the Starter picker's.
import { useEffect, useMemo, useState } from 'react';
import {
  ROOM_COPY,
  ROOM_FOLDER_ROOT,
  ROOM_MAX_BOTS,
  ROOM_MAX_FOLDER,
  ROOM_MAX_INSTRUCTIONS,
  ROOM_MAX_NAME,
  folderProblem,
  roomErrorText,
  roomProblem,
  type RoomMode,
  type RoomsTransport,
  type RoomView,
} from '../../../lib/potato-rooms';

export interface RoomSettingsProps {
  room: RoomView;
  /** All the person's bots, for the Add list and the member names. */
  bots: { id: string; name: string }[];
  transport: RoomsTransport;
  onDone: (room: RoomView) => void;
  onCancel: () => void;
}

export function RoomSettings({ room, bots, transport, onDone, onCancel }: RoomSettingsProps) {
  const [name, setName] = useState(room.name);
  const [mode, setMode] = useState<RoomMode>(room.mode ?? 'listen');
  const [members, setMembers] = useState<string[]>(() => room.bots.map((b) => b.id));
  const [off, setOff] = useState<string[]>(room.offBots ?? []);
  const [instructions, setInstructions] = useState(room.instructions ?? '');
  const [folder, setFolder] = useState(room.folder ?? '');
  const [adding, setAdding] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onCancel]);

  const nameOf = (id: string) => bots.find((b) => b.id === id)?.name ?? room.bots.find((b) => b.id === id)?.name ?? id;
  const addable = useMemo(() => bots.filter((b) => !members.includes(b.id)), [bots, members]);
  const onMembers = members.filter((id) => !off.includes(id));
  const problem = roomProblem(name, members) ?? folderProblem(folder) ?? (onMembers.length === 0 ? 'Keep at least one bot on in this group.' : null);
  const defaultPath = `${ROOM_FOLDER_ROOT}${folder.trim().toLowerCase() || room.id}`;

  const save = async () => {
    if (problem || busy) return;
    setBusy(true);
    setError(null);
    try {
      const sameMembers = members.length === room.bots.length && members.every((id, i) => id === room.bots[i]?.id);
      if (!sameMembers) await transport.setBots(room.id, members);
      if (name.trim() !== room.name) await transport.rename(room.id, name);
      const saved = await transport.setSettings(room.id, { instructions, folder, mode, offBots: off.filter((id) => members.includes(id)) });
      onDone(saved);
    } catch (e) {
      setError(roomErrorText(e, 'Could not save the group settings.'));
      setBusy(false);
    }
  };

  return (
    <div className="ck-potato-starters-overlay" role="presentation">
      <section className="ck-potato-starters ck-potato-roomsettings" role="dialog" aria-modal="true" aria-label={ROOM_COPY.settingsTitle}>
        <header className="ck-potato-starters-head">
          <h2 className="ck-potato-starters-title">{ROOM_COPY.settingsTitle}</h2>
        </header>
        <form
          className="ck-potato-newgroup"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <label className="ck-potato-routine-field">
            <span>{ROOM_COPY.settingsNameLabel}</span>
            <input className="ck-potato-adv-input" type="text" maxLength={ROOM_MAX_NAME} value={name} onChange={(e) => setName(e.target.value)} />
          </label>

          <fieldset className="ck-potato-newgroup-bots">
            <legend>{ROOM_COPY.settingsWhoLabel}</legend>
            <label className="ck-row ck-potato-newgroup-bot">
              <input type="radio" name="room-mode" checked={mode === 'listen'} onChange={() => setMode('listen')} />
              <span className="ck-row-title">{ROOM_COPY.settingsListen}</span>
            </label>
            <p className="ck-sub">{ROOM_COPY.settingsListenHint}</p>
            <label className="ck-row ck-potato-newgroup-bot">
              <input type="radio" name="room-mode" checked={mode === 'mentions'} onChange={() => setMode('mentions')} />
              <span className="ck-row-title">{ROOM_COPY.settingsMentions}</span>
            </label>
            <p className="ck-sub">{ROOM_COPY.settingsMentionsHint}</p>
          </fieldset>

          <fieldset className="ck-potato-newgroup-bots">
            <legend>{ROOM_COPY.settingsMembers}</legend>
            {members.map((id) => (
              <div key={id} className="ck-row ck-potato-newgroup-bot ck-potato-roomsettings-member">
                <label className="ck-potato-roomsettings-on">
                  <input
                    type="checkbox"
                    aria-label={ROOM_COPY.settingsOnLabel(nameOf(id))}
                    checked={!off.includes(id)}
                    onChange={() => setOff((was) => (was.includes(id) ? was.filter((b) => b !== id) : [...was, id]))}
                  />
                  <span className="ck-row-title">{nameOf(id)}</span>
                </label>
                <button
                  type="button"
                  className="ck-btn ck-btn--plain ck-btn--sm"
                  aria-label={ROOM_COPY.settingsRemove(nameOf(id))}
                  disabled={members.length <= 2}
                  onClick={() => setMembers((was) => was.filter((b) => b !== id))}
                >
                  Remove
                </button>
              </div>
            ))}
            {addable.length > 0 && (
              <div className="ck-potato-roomsettings-add">
                <select className="ck-potato-adv-input" aria-label={ROOM_COPY.settingsAdd} value={adding} onChange={(e) => setAdding(e.target.value)} disabled={members.length >= ROOM_MAX_BOTS}>
                  <option value="">{ROOM_COPY.settingsAddNone}</option>
                  {addable.map((b) => (
                    <option key={b.id} value={b.id}>
                      {b.name}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  className="ck-btn ck-btn--secondary ck-btn--sm"
                  disabled={adding === '' || members.length >= ROOM_MAX_BOTS}
                  onClick={() => {
                    setMembers((was) => [...was, adding]);
                    setAdding('');
                  }}
                >
                  {ROOM_COPY.settingsAddButton}
                </button>
              </div>
            )}
            {members.length >= ROOM_MAX_BOTS && <p className="ck-sub">{ROOM_COPY.settingsLimit(ROOM_MAX_BOTS)}</p>}
          </fieldset>

          <label className="ck-potato-routine-field">
            <span>{ROOM_COPY.settingsInstructions}</span>
            <textarea
              className="ck-potato-adv-input"
              rows={4}
              maxLength={ROOM_MAX_INSTRUCTIONS}
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
            />
            <span className="ck-sub">{ROOM_COPY.settingsInstructionsHint}</span>
          </label>

          <label className="ck-potato-routine-field">
            <span>{ROOM_COPY.settingsFolder}</span>
            <input className="ck-potato-adv-input" type="text" maxLength={ROOM_MAX_FOLDER} value={folder} placeholder={room.id} onChange={(e) => setFolder(e.target.value)} />
            <span className="ck-sub">{ROOM_COPY.settingsFolderHint(defaultPath)}</span>
          </label>

          {problem && <p className="ck-sub ck-potato-starters-error">{problem}</p>}
          {error && (
            <p role="alert" className="ck-potato-starters-error">
              {error}
            </p>
          )}
        </form>
        <footer className="ck-potato-starters-foot">
          <button type="button" className="ck-btn ck-btn--plain ck-btn--sm" onClick={onCancel}>
            {ROOM_COPY.cancel}
          </button>
          <button type="button" className="ck-btn ck-btn--secondary" disabled={problem !== null || busy} onClick={() => void save()}>
            {ROOM_COPY.save}
          </button>
        </footer>
      </section>
    </div>
  );
}
