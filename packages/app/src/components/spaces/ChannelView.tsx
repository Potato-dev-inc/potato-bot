import { Fragment, useEffect, useRef, useState, type ReactNode } from 'react';
import {
  spaceFileGet, spaceFileOffer, spaceFileSave, spacesErrorText, useChannel, type SpaceChannel, type SpaceDetailBot, type SpaceMessage,
} from '../../lib/spaces';
import { MessageRow } from './MessageRow';
import { dayLabel, sameDay } from './messageParts';
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
/** Lines from one sender closer together than this read as one run, like a chat. */
export const GROUP_WITHIN_MS = 5 * 60_000;
/** Whether `m` continues the run that `prev` started: same sender, close in time, plain text on both sides. */
export function continuesRun(prev: SpaceMessage | undefined, m: SpaceMessage): boolean {
  if (!prev) return false;
  if (prev.deleted || m.deleted || prev.blocked || m.blocked || prev.file || m.file || m.replyPreview) return false;
  if (prev.member !== m.member || prev.isFish !== m.isFish || (prev.botId ?? null) !== (m.botId ?? null)) return false;
  const gap = m.createdAtMs - prev.createdAtMs;
  return gap >= 0 && gap < GROUP_WITHIN_MS && sameDay(prev.createdAtMs, m.createdAtMs);
}
export const READ_ONLY_TEXT = "You can read this channel, but you can't write in it.";
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
  const list = useRef<HTMLOListElement>(null);
  const [flash, setFlash] = useState<string | null>(null);
  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), 1600);
    return () => clearTimeout(t);
  }, [flash]);
  /** A reply line was clicked: scroll to the original when it is loaded, and mark it for a moment. */
  const jump = (id: string) => {
    const el = Array.from(list.current?.querySelectorAll<HTMLElement>('li[data-id]') ?? []).find((x) => x.dataset.id === id);
    if (!el) return;
    el.scrollIntoView?.({ block: 'center', behavior: 'smooth' });
    setFlash(id);
  };
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
  const myName = ch.messages.find((m) => m.isMine && !m.isFish)?.authorName;
  const names = [...new Set([...ch.messages.map((m) => m.authorName), ...bots.map((b) => b.name)])];
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
        <p className="ck-spaces-hint ck-spaces-botsline is-listed">{botsLine(visible)}</p>
      ) : null}
      <ol className="ck-spaces-msgs" ref={list}>
        {ch.atStart ? (
          <li className="ck-spaces-start">This is the start of #{channel.name}.</li>
        ) : (
          <li><button type="button" className="ck-spaces-quiet" onClick={() => void ch.loadOlder()}>Show older messages</button></li>
        )}
        {shown.map((m, i) => {
          const prev = shown[i - 1];
          return (
            <Fragment key={m.id}>
              {(!prev || !sameDay(prev.createdAtMs, m.createdAtMs)) && (
                <li className="ck-spaces-day" role="separator"><span>{dayLabel(m.createdAtMs)}</span></li>
              )}
              <MessageRow message={m} grouped={continuesRun(prev, m)} onReply={setReplyTo} menu={menu} onReact={onReact}
                names={names} myName={myName} onJump={jump} flash={flash === m.id}
                fileError={fileErrors[m.id] ?? null} fileSavedAs={savedAs[m.id] ?? null} onGetFile={getFile} onSaveFile={saveFile}
                revealBlocked={revealed.has(m.id)} onRevealBlocked={() => setRevealed(new Set(revealed).add(m.id))} />
            </Fragment>
          );
        })}
      </ol>
      <div ref={end} />
      {ch.error && <p className="ck-spaces-hint is-error" role="alert">{ch.error}</p>}
      {readOnly && <p className="ck-spaces-readonly" role="status">{READ_ONLY_TEXT}</p>}
      {!readOnly && (
        <Composer channelName={channel.name} hint={!botsOff && visible.length > 0 ? CALL_BOT_HINT : undefined}
          problem={(t) => callProblem(t, bots, botsOff)} notes={shownNotes}
          bots={botsOff ? [] : visible.map((b) => ({ id: b.id, name: b.name, ownerName: b.ownerName }))} replyTo={replyTo} onCancelReply={() => setReplyTo(null)}
          disabled={channel.archived} onAttach={attach} onSend={send} />
      )}
    </section>
  );
}
