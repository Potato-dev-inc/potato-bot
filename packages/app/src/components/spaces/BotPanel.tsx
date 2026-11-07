// Spaces: the bots of one space. Three kinds of control, kept apart:
// everyone (hide a bot on this PC), the bot's owner (their own bot's settings), the space owner (the rules).
import { useEffect, useId, useRef, useState } from 'react';
import { Sheet } from '../cockpit/Sheet';
import {
  spaceBotLocal, spaceBotRemove, spaceBotRules, spacesErrorText, useSpaceBots,
  type BotMode, type SpaceBotList, type SpaceBotPatch, type SpaceBotRow, type SpaceChannel,
} from '../../lib/spaces';
import { AddBotSheet } from './AddBotSheet';
import { botLimit, limitLine } from './botLimit';

export const NON_OWNER_CHIP = "Bots here can chat and read their memory. Only a bot's owner can use its tools.";
export const waitingText = (name: string) => `Waiting for the space owner's computer: ${name}`;
export const answeredText = (n: number, cap: number) => `Answered ${n} of ${cap} today.`;
export const heldBackText = (n: number) => `Held back ${n} ${n === 1 ? 'reply' : 'replies'} today because they looked private.`;
export const MAX_BOTS_PER_PERSON = 10;

type Run = (call: Promise<SpaceBotList>) => Promise<void>;

/** One bot. Its owner sees the settings; everyone sees the hide switch. */
function BotItem({ bot, canRemove, space, run }: { bot: SpaceBotRow; canRemove: boolean; space: string; run: Run }) {
  const [cap, setCap] = useState(String(bot.dayCap ?? ''));
  const [removing, setRemoving] = useState(false);
  useEffect(() => setCap(String(bot.dayCap ?? '')), [bot.dayCap]);
  const patch = (p: SpaceBotPatch) => void run(spaceBotLocal(space, bot.id, p));
  const commitCap = () => {
    const n = Number(cap);
    if (cap.trim() !== '' && Number.isFinite(n) && n >= 0 && n !== bot.dayCap) patch({ dayCap: Math.floor(n) });
    else setCap(String(bot.dayCap ?? ''));
  };
  return (
    <li className={`ck-spaces-bot${bot.muted ? ' is-muted' : ''}`} aria-label={bot.name}>
      <div className="ck-spaces-bot-head">
        <span className="ck-spaces-bot-name">{bot.name}</span>
        <span className="ck-spaces-role">{bot.mine ? 'Your bot' : `Owner: ${bot.ownerName}`}</span>
        {bot.silenced && <span className="ck-spaces-role">Silenced</span>}
        {bot.muted && <span className="ck-spaces-role">Hidden here</span>}
      </div>
      {bot.mine && (
        <fieldset className="ck-spaces-bot-own">
          <legend>Your bot's settings</legend>
          <div role="radiogroup" aria-label={`When ${bot.name} answers`} className="ck-spaces-choice">
            {([['called', 'Only when called'], ['helps', 'When it helps']] as [BotMode, string][]).map(([m, label]) => (
              <label key={m}>
                <input type="radio" name={`mode-${bot.id}`} checked={bot.mode === m} onChange={() => patch({ mode: m })} /> {label}
              </label>
            ))}
          </div>
          <label className="ck-spaces-check">
            <input type="checkbox" role="switch" checked={bot.memory === true} onChange={(e) => patch({ memory: e.target.checked })} /> Memory
          </label>
          <label className="ck-spaces-field">
            <span>Daily limit</span>
            <input type="number" min={0} inputMode="numeric" value={cap} onChange={(e) => setCap(e.target.value)}
              onBlur={commitCap} onKeyDown={(e) => { if (e.key === 'Enter') commitCap(); }} />
          </label>
          <p className="ck-spaces-hint">{answeredText(bot.usedToday, bot.dayCap ?? 0)}</p>
          <p className="ck-spaces-hint">{heldBackText(bot.heldBackToday)}</p>
          <label className="ck-spaces-check">
            <input type="checkbox" role="switch" checked={bot.silenced === true} onChange={(e) => patch({ silenced: e.target.checked })} /> Silence this bot
          </label>
        </fieldset>
      )}
      <div className="ck-spaces-actions ck-spaces-bot-actions">
        <button type="button" className="ck-spaces-btn" aria-pressed={bot.muted} onClick={() => patch({ muted: !bot.muted })}>
          {bot.muted ? 'Show on this computer' : 'Hide on this computer'}
        </button>
        {canRemove && !removing && (
          <button type="button" className="ck-spaces-btn" onClick={() => setRemoving(true)}>Remove bot</button>
        )}
      </div>
      {removing && (
        <div className="ck-spaces-bot-confirm" role="group" aria-label={`Remove ${bot.name}?`}>
          <span>Remove {bot.name} from this space?</span>
          <button type="button" className="ck-spaces-btn" onClick={() => setRemoving(false)}>Keep</button>
          <button type="button" className="ck-spaces-btn is-danger"
            onClick={() => { setRemoving(false); void run(spaceBotRemove(space, bot.id)); }}>Yes, remove</button>
        </div>
      )}
    </li>
  );
}

/** The space owner's rules: how many bots each person may have and where bots may speak. */
function OwnerRules({ list, channels, space, run, focusSignal }: { list: SpaceBotList; channels: SpaceChannel[]; space: string; run: Run; focusSignal: number }) {
  const [per, setPer] = useState(list.rules.perPerson);
  const [only, setOnly] = useState<string[] | null>(list.rules.channels);
  const key = JSON.stringify(list.rules);
  useEffect(() => {
    setPer(list.rules.perPerson);
    setOnly(list.rules.channels);
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  const live = channels.filter((c) => !c.archived);
  const perRef = useRef<HTMLSelectElement>(null);
  // The add sheet's "Change Bots per person" lands here: bring the setting into view and focus it.
  useEffect(() => {
    if (focusSignal === 0) return;
    perRef.current?.scrollIntoView?.({ block: 'center' });
    perRef.current?.focus();
  }, [focusSignal]);
  const toggle = (id: string) => setOnly((cur) => {
    const set = new Set(cur ?? []);
    if (set.has(id)) set.delete(id);
    else set.add(id);
    return [...set];
  });
  return (
    <section className="ck-spaces-bot-rules" aria-label="Space owner settings">
      <h3 className="ck-spaces-feed-title">Space owner settings</h3>
      <label className="ck-spaces-field">
        <span>Bots per person</span>
        <select ref={perRef} value={per} onChange={(e) => setPer(Number(e.target.value))}>
          {Array.from({ length: MAX_BOTS_PER_PERSON + 1 }, (_, n) => <option key={n} value={n}>{n}</option>)}
        </select>
      </label>
      <fieldset className="ck-spaces-bot-own">
        <legend>Allowed channels</legend>
        <div role="radiogroup" aria-label="Allowed channels" className="ck-spaces-choice">
          <label><input type="radio" name="bot-channels" checked={only === null} onChange={() => setOnly(null)} /> All channels</label>
          <label><input type="radio" name="bot-channels" checked={only !== null} onChange={() => setOnly(only ?? live.map((c) => c.id))} /> Only these</label>
        </div>
        {only !== null && live.map((c) => (
          <label key={c.id} className="ck-spaces-check">
            <input type="checkbox" checked={only.includes(c.id)} onChange={() => toggle(c.id)} /> # {c.name}
          </label>
        ))}
      </fieldset>
      <div className="ck-spaces-actions">
        <button type="button" className="ck-spaces-btn is-primary" onClick={() => void run(spaceBotRules(space, per, only))}>Save bot rules</button>
      </div>
    </section>
  );
}

export interface BotPanelProps {
  space: string;
  spaceName: string;
  channels: SpaceChannel[];
  onClose: () => void;
}

export function BotPanel({ space, spaceName, channels, onClose }: BotPanelProps) {
  const titleId = useId();
  const api = useSpaceBots(space);
  const [adding, setAdding] = useState(false);
  const [focusRules, setFocusRules] = useState(0);
  const [actionError, setActionError] = useState<string | null>(null);
  const list = api.list;
  const run: Run = async (call) => {
    try {
      api.set(await call);
      setActionError(null);
    } catch (e) {
      setActionError(spacesErrorText(e));
    }
  };
  // The add sheet takes the screen while it is open (one sheet at a time keeps Escape simple).
  if (adding) {
    return (
      <AddBotSheet space={space} list={list} onAdded={(l) => { api.set(l); setActionError(null); }} onClose={() => setAdding(false)}
        onChangeLimit={() => { setAdding(false); setFocusRules((n) => n + 1); }} />
    );
  }
  const limit = botLimit(list);
  const error = actionError ?? api.error;
  const othersBots = list?.bots.some((b) => !b.mine) ?? false;
  return (
    <Sheet labelledBy={titleId} role="dialog" onClose={onClose} className="ck-spaces-sheet ck-spaces-botpanel">
      <h2 id={titleId} className="ck-spaces-sheet-title">Bots in {spaceName}</h2>
      {!list && !error && <p className="ck-spaces-hint" role="status">Loading bots…</p>}
      {list && (
        <>
          {othersBots && <p className="ck-spaces-chip">{NON_OWNER_CHIP}</p>}
          {list.bots.length === 0 && <p className="ck-spaces-sheet-body">No bots in this space yet.</p>}
          <ul className="ck-spaces-bots">
            {list.bots.map((b) => (
              <BotItem key={b.id} bot={b} space={space} run={run} canRemove={b.mine || list.isOwner} />
            ))}
          </ul>
          {list.pendingAdd.map((n) => <p key={n} className="ck-spaces-hint" role="status">{waitingText(n)}</p>)}
          {list.addBlocked && <p className="ck-spaces-hint">{list.addBlocked}</p>}
          {limit && <p className="ck-spaces-hint">{limitLine(limit)}</p>}
          <div className="ck-spaces-actions">
            {/* Stays pressable at the limit: the sheet it opens says what is already here and what to do. */}
            <button type="button" className="ck-spaces-btn is-primary" disabled={list.addBlocked !== null} onClick={() => setAdding(true)}>
              Add my bot
            </button>
          </div>
          {list.isOwner && <OwnerRules list={list} channels={channels} space={space} run={run} focusSignal={focusRules} />}
        </>
      )}
      {error && <p className="ck-spaces-hint is-error" role="alert">{error}</p>}
      <div className="ck-spaces-actions">
        <button type="button" className="ck-spaces-btn" onClick={onClose}>Close</button>
      </div>
    </Sheet>
  );
}
