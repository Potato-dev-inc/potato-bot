import { useEffect, useRef, useState, type ReactNode } from 'react';
import {
  spaceFileGet, spaceFileOffer, spaceFileSave, spacesErrorText, useChannel, type SpaceChannel, type SpaceDetailBot, type SpaceMessage,
} from '../../lib/spaces';
import { MessageRow } from './MessageRow';
import { Composer } from './Composer';
import { callProblem, isCalled, refusedNotes } from './botCall';

export interface ChannelViewProps {
  space: string;
  channel: SpaceChannel;
  menu?: (m: SpaceMessage, reply: () => void) => ReactNode;
  onReact?: (m: SpaceMessage, code: string, on: boolean) => void;
  /** A space I left or was removed from: the feed stays, the composer goes. */
  readOnly?: boolean;
  /** The bots of this space (header line, composer hint, which rows to hide). */
  bots?: SpaceDetailBot[];
  /** The space owner's rules keep bots out of this channel. */
  botsOff?: boolean;
}

export const BOTS_OFF_TEXT = 'Bots are off in this channel.';
export const CALL_BOT_HINT = "Type @ and a bot's name to call it.";
export const botsLine = (bots: SpaceDetailBot[]) => `Bots here: ${bots.map((b) => b.name).join(' · ')}`;

export function ChannelView({ space, channel, menu, onReact, readOnly, bots = [], botsOff = false }: ChannelViewProps) {
  const ch = useChannel(space, channel.id);
  const [replyTo, setReplyTo] = useState<SpaceMessage | null>(null);
  const [revealed, setRevealed] = useState<Set<string>>(new Set());
  const [fileErrors, setFileErrors] = useState<Record<string, string | null>>({});
  const [savedAs, setSavedAs] = useState<Record<string, string>>({});
  /** The refusal notes the person has moved past by sending a line that called nobody (see `notes` below). */
  const [movedPast, setMovedPast] = useState('');
  const end = useRef<HTMLDivElement>(null);
  const getFile = (m: SpaceMessage) => {
    if (!m.file) return;
    setFileErrors((e) => ({ ...e, [m.id]: null }));
    spaceFileGet(space, m.file.sha).catch((e: unknown) => setFileErrors((x) => ({ ...x, [m.id]: spacesErrorText(e) })));
  };
  const saveFile = (m: SpaceMessage) => {
    if (!m.file) return;
    setFileErrors((e) => ({ ...e, [m.id]: null }));
    spaceFileSave(space, m.file.sha).then(
      (r) => setSavedAs((s) => ({ ...s, [m.id]: r.name })),
      (e: unknown) => setFileErrors((x) => ({ ...x, [m.id]: spacesErrorText(e) })),
    );
  };
  const attach = async (path: string): Promise<string | null> => {
    try {
      await spaceFileOffer(space, channel.id, path);
      return null;
    } catch (e) {
      return spacesErrorText(e);
    }
  };
  const hidden = new Set(bots.filter((b) => b.muted).map((b) => b.id));
  const shown = ch.messages.filter((m) => !(m.botId && hidden.has(m.botId)));
  const visible = bots.filter((b) => !b.muted);
  const lastId = ch.messages[ch.messages.length - 1]?.id;
  useEffect(() => { end.current?.scrollIntoView?.({ block: 'end' }); }, [lastId]);
  // A "did not answer" note belongs to a call just made. Once a later line that calls nobody is sent, it goes; it comes
  // back only when the same bot refuses a new call (the backend clears `refused` first, so the key empties in between).
  const notes = refusedNotes(bots);
  const notesKey = notes.join('|');
  useEffect(() => { if (notesKey === '') setMovedPast(''); }, [notesKey]);
  const shownNotes = notesKey === movedPast ? [] : notes;
  const send = async (text: string) => {
    const err = await ch.send(text, replyTo?.id ?? null);
    if (!err && !bots.some((b) => isCalled(text, b.name))) setMovedPast(notesKey);
    return err;
  };
  return (
    <section className="ck-spaces-feed" aria-label={`#${channel.name}`}>
      <h2 className="ck-spaces-feed-title"># {channel.name}</h2>
      {botsOff ? (
        <p className="ck-spaces-hint ck-spaces-botsline">{BOTS_OFF_TEXT}</p>
      ) : visible.length > 0 ? (
        <p className="ck-spaces-hint ck-spaces-botsline">{botsLine(visible)}</p>
      ) : null}
      <ol className="ck-spaces-msgs">
        {ch.atStart ? (
          <li className="ck-spaces-start">This is the start of #{channel.name}.</li>
        ) : (
          <li><button type="button" className="ck-spaces-quiet" onClick={() => void ch.loadOlder()}>Show older messages</button></li>
        )}
        {shown.map((m) => (
          <MessageRow key={m.id} message={m} onReply={setReplyTo} menu={menu} onReact={onReact}
            fileError={fileErrors[m.id] ?? null} fileSavedAs={savedAs[m.id] ?? null} onGetFile={getFile} onSaveFile={saveFile}
            revealBlocked={revealed.has(m.id)} onRevealBlocked={() => setRevealed(new Set(revealed).add(m.id))} />
        ))}
      </ol>
      <div ref={end} />
      {ch.error && <p className="ck-spaces-hint is-error" role="alert">{ch.error}</p>}
      {!readOnly && (
        <Composer channelName={channel.name} hint={!botsOff && visible.length > 0 ? CALL_BOT_HINT : undefined}
          problem={(t) => callProblem(t, bots, botsOff)} notes={shownNotes}
          bots={botsOff ? [] : visible.map((b) => ({ id: b.id, name: b.name, ownerName: b.ownerName }))} replyTo={replyTo} onCancelReply={() => setReplyTo(null)}
          disabled={channel.archived} onAttach={attach} onSend={send} />
      )}
    </section>
  );
}
