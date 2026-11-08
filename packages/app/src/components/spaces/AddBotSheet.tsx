// Spaces: add one of my Potato bots to a space. The name starts as `<owner>的<bot>` and stays editable.
import { useEffect, useId, useState } from 'react';
import { Sheet } from '../cockpit/Sheet';
import {
  spaceBotAdd, spaceBotCandidates, spacesErrorText, type SpaceBotCandidate, type SpaceBotList,
} from '../../lib/spaces';
import { ASK_OWNER, CHANGE_LIMIT, botLimit, limitLine, limitReachedText } from './botLimit';

export const MEMORY_NOTICE = 'People here may get your bot to repeat what it remembers.';
export const NO_CANDIDATES = 'You have no Potato bot that can join a space. Bots made with custom agents can\'t join.';
/** The bots that could join are all already here (or waiting), so the backend list is empty for that reason. */
export const ALL_HERE = 'All your Potato bots are already here.';

export interface AddBotSheetProps {
  space: string;
  /** Gets the bot list the add answered with. The sheet closes after. */
  onAdded: (list: SpaceBotList) => void;
  onClose: () => void;
  /** The bot list the panel has (the limit and what is already here). Without it the sheet shows no limit. */
  list?: SpaceBotList | null;
  /** The space owner's way straight to Bots per person. The panel closes this sheet and opens the setting. */
  onChangeLimit?: () => void;
}

export function AddBotSheet({ space, onAdded, onClose, list = null, onChangeLimit }: AddBotSheetProps) {
  const titleId = useId();
  const [candidates, setCandidates] = useState<SpaceBotCandidate[] | null>(null);
  const [picked, setPicked] = useState('');
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The space refused although the list said there was room (a bot of mine added elsewhere): treat it as reached.
  const [refusedFull, setRefusedFull] = useState(false);
  useEffect(() => {
    let alive = true;
    spaceBotCandidates(space).then(
      (r) => {
        if (!alive) return;
        const list = r?.candidates ?? [];
        setCandidates(list);
        if (list[0]) {
          setPicked(list[0].id);
          setName(list[0].suggestedName);
        }
      },
      (e: unknown) => {
        if (!alive) return;
        setCandidates([]);
        setError(spacesErrorText(e));
      },
    );
    return () => {
      alive = false;
    };
  }, [space]);
  const choose = (id: string) => {
    setPicked(id);
    const c = candidates?.find((x) => x.id === id);
    if (c) setName(c.suggestedName);
  };
  const add = async () => {
    setBusy(true);
    try {
      const list = await spaceBotAdd(space, picked, name.trim());
      onAdded(list);
      onClose();
    } catch (e) {
      const text = spacesErrorText(e);
      if (text.includes('as many bots')) setRefusedFull(true);
      else setError(text);
      setBusy(false);
    }
  };
  const limit = botLimit(list);
  const full = refusedFull || limit?.reached === true;
  const none = candidates !== null && candidates.length === 0;
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={busy ? undefined : onClose} className="ck-spaces-sheet">
      <h2 id={titleId} className="ck-spaces-sheet-title">Add a bot</h2>
      <div className="ck-spaces-sheet-scroll">
        {limit && !full && <p className="ck-spaces-sheet-body">{limitLine(limit)}</p>}
        {full && (
          <div className="ck-spaces-card ck-spaces-limit" role="status">
            <p className="ck-spaces-sheet-body">
              {limit ? limitReachedText(limit) : 'You already have as many bots here as this space allows.'}
            </p>
            {list?.isOwner ? (
              onChangeLimit && (
                <div className="ck-spaces-row is-actions">
                  <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={onChangeLimit}>{CHANGE_LIMIT}</button>
                </div>
              )
            ) : (
              <p className="ck-spaces-hint">{ASK_OWNER}</p>
            )}
          </div>
        )}
        {!full && candidates === null && <p className="ck-spaces-hint" role="status">Looking for your bots…</p>}
        {!full && none && <p className="ck-spaces-sheet-body">{limit && limit.names.length > 0 ? ALL_HERE : NO_CANDIDATES}</p>}
        {!full && candidates !== null && candidates.length > 1 && (
          <label className="ck-spaces-field">
            <span>Your bot</span>
            <select value={picked} onChange={(e) => choose(e.target.value)}>
              {candidates.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
            </select>
          </label>
        )}
        {!full && candidates !== null && candidates.length === 1 && (
          <div className="ck-spaces-card"><p className="ck-spaces-sheet-body">Your bot: {candidates[0].name}</p></div>
        )}
        {!full && !none && candidates !== null && (
          <>
            <label className="ck-spaces-field">
              <span>Name in this space</span>
              <input value={name} onChange={(e) => setName(e.target.value)} />
            </label>
            <p className="ck-spaces-note">{MEMORY_NOTICE}</p>
          </>
        )}
      </div>
      {error && <p className="ck-spaces-sheet-status is-error" role="alert">{error}</p>}
      <div className="ck-sheet-actions">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={onClose} disabled={busy}>{full ? 'Close' : 'Cancel'}</button>
        <button type="button" className="ck-btn ck-btn--primary" disabled={busy || full || none || candidates === null || name.trim() === ''}
          onClick={() => void add()}>Add bot</button>
      </div>
    </Sheet>
  );
}
