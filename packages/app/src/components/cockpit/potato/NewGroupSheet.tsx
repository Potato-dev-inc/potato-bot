// The New group sheet (plan 12, slice G9): a name, a checklist of the person's bots, and Make group. The sheet does
// not let an invalid group through: the button stays off and the sentence says what is wrong (the same checks as the
// engine, `roomProblem`). Escape cancels. The overlay and panel classes are the Starter picker's.
import { useEffect, useRef, useState } from 'react';
import { roomErrorText, roomProblem, ROOM_COPY, type RoomsTransport, type RoomView } from '../../../lib/potato-rooms';

export interface NewGroupSheetProps {
  bots: { id: string; name: string }[];
  transport: RoomsTransport;
  /** Called with the new group once it is saved. */
  onDone: (room: RoomView) => void;
  onCancel: () => void;
  /** Change the bots of this group instead of making one: the sheet is titled Change bots, has no name field, and Save keeps the picked bots. */
  edit?: RoomView;
}

export function NewGroupSheet({ bots, transport, onDone, onCancel, edit }: NewGroupSheetProps) {
  const [name, setName] = useState('');
  const [picked, setPicked] = useState<string[]>(() => (edit ? edit.bots.map((b) => b.id).filter((id) => bots.some((b) => b.id === id)) : []));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onCancel]);

  const tooFew = bots.length < 2;
  const problem = roomProblem(edit ? edit.name : name, picked);

  const toggle = (id: string) => setPicked((was) => (was.includes(id) ? was.filter((b) => b !== id) : [...was, id]));

  const make = async () => {
    if (problem || busy) return;
    setBusy(true);
    setError(null);
    try {
      const room = edit ? await transport.setBots(edit.id, picked) : await transport.create(name, picked);
      onDone(room);
    } catch (e) {
      setError(roomErrorText(e, edit ? 'Could not change the bots.' : 'Could not make the group.'));
      setBusy(false);
    }
  };

  return (
    <div className="ck-potato-starters-overlay" role="presentation">
      <section className="ck-potato-starters" role="dialog" aria-modal="true" aria-label={edit ? ROOM_COPY.changeBotsTitle : ROOM_COPY.sheetTitle}>
        <header className="ck-potato-starters-head">
          <h2 className="ck-potato-starters-title">{edit ? ROOM_COPY.changeBotsTitle : ROOM_COPY.sheetTitle}</h2>
        </header>
        {tooFew ? (
          <p className="ck-sub ck-potato-starters-intro">{ROOM_COPY.emptyNoBots}</p>
        ) : (
          <form
            className="ck-potato-newgroup"
            onSubmit={(e) => {
              e.preventDefault();
              void make();
            }}
          >
            {!edit && (
            <label className="ck-potato-routine-field">
              <span>{ROOM_COPY.nameLabel}</span>
              <input
                ref={nameRef}
                className="ck-potato-adv-input"
                type="text"
                maxLength={60}
                value={name}
                placeholder={ROOM_COPY.namePlaceholder}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            )}
            <fieldset className="ck-potato-newgroup-bots">
              <legend>{ROOM_COPY.botsLabel}</legend>
              <p className="ck-sub">{ROOM_COPY.botsHint}</p>
              {bots.map((bot) => (
                <label key={bot.id} className="ck-row ck-potato-newgroup-bot">
                  <input type="checkbox" checked={picked.includes(bot.id)} onChange={() => toggle(bot.id)} />
                  <span className="ck-row-title">{bot.name}</span>
                </label>
              ))}
            </fieldset>
            {problem && <p className="ck-sub ck-potato-starters-error">{problem}</p>}
            {error && (
              <p role="alert" className="ck-potato-starters-error">
                {error}
              </p>
            )}
          </form>
        )}
        <footer className="ck-potato-starters-foot">
          <button type="button" className="ck-btn ck-btn--plain ck-btn--sm" onClick={onCancel}>
            {ROOM_COPY.cancel}
          </button>
          {!tooFew && (
            <button type="button" className="ck-btn ck-btn--secondary ck-potato-starter-make" disabled={problem !== null || busy} onClick={() => void make()}>
              {edit ? ROOM_COPY.save : ROOM_COPY.make}
            </button>
          )}
        </footer>
      </section>
    </div>
  );
}
