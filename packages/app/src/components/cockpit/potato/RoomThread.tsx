// One group's chat (plan 12, slice G10), in the middle of the Potato page. It reads the group's lines, draws them with
// the same rows as a bot's thread (so Markdown, notes, day lines and file cards look alike), and puts the bot's name
// and face above what it said. The person writes in the same message box as in a bot's thread (`@` lists the group's
// bots and `@everyone`). The page never shows a bot panel for a group.
//
// What the lines map to:
// - the person's line: a filled bubble on the right, no name
// - a bot's line: an outlined bubble with the bot's face and name above it; a file card opens with that bot's id
// - a note line (hop 255: a bot stopped, is over its limit...): a small grey centred note
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { AvatarId } from '../../../lib/companion';
import { plainError, type PotatoBotView, type PotatoMessage, type PotatoTransport } from '../../../lib/potato';
import { ROOM_COPY, ROOM_EVERYONE, ROOM_NOTE_HOP, isRoomGone, roomErrorText, roomProblem, type RoomLine, type RoomsTransport, type RoomView } from '../../../lib/potato-rooms';
import { THREAD_COPY, firstUnreadId } from '../../../lib/potato-thread';
import { Avatar } from '../../companion/Avatar';
import { Composer } from './Composer';
import { MessageRow } from './MessageRow';
import { useJumpTo, type JumpTarget } from './useJumpTo';
import { NewGroupSheet } from './NewGroupSheet';
import { DaySeparator, TimeSeparator, UnreadDivider } from './Separators';
import { ThreadIcon } from './ThreadIcons';
import { ToolActivity } from './ToolActivity';
import { WorkSteps } from './WorkSteps';
import { TypingRow } from './TypingRow';
import { runButton, threadItems } from './model';
import { useRoomLines } from './usePotatoRooms';
import '../../../styles/cockpit-potato-thread.css';

/** Closer than this to the bottom, a new line keeps the thread pinned to it. */
const STICK_PX = 120;
const MENU_WIDTH = 168;

export interface RoomThreadProps {
  room: RoomView;
  transport: RoomsTransport;
  /** The bots' own calls: Stop, and the file cards' Open and Save. */
  potato: PotatoTransport;
  /** All the person's bots: the member list reads their state, Change bots offers them. */
  bots: PotatoBotView[];
  /** Each bot's face. A bot with none shows the default. */
  avatars?: ReadonlyMap<string, AvatarId>;
  /** The group was deleted from this thread. */
  onDeleted: () => void;
  /** The app says the group no longer exists (deleted elsewhere). Default: same as `onDeleted`. */
  onGone?: () => void;
  /** Said when a command fails and no strip is there to say it. */
  onNotify: (text: string) => void;
  /** Called after Stop, so the bots' state is read again at once. */
  onChanged?: () => void;
  /** A Search chats result: the line to scroll to and mark (`n` makes the same line jump again). */
  jumpTo?: JumpTarget | null;
  /** Passed in so a test can pin the clock. */
  now?: number;
}

type Overlay = null | 'bots' | 'delete';

/** The room's lines as the thread's messages. The bot's id stays with the line (`senders`), because a file card needs it. */
function asMessages(lines: readonly RoomLine[]): { messages: PotatoMessage[]; senders: Map<string, RoomLine['from']> } {
  const senders = new Map<string, RoomLine['from']>();
  const messages = lines.map((line): PotatoMessage => {
    senders.set(line.id, line.from);
    const base = { id: line.id, at: line.at, text: line.text };
    if (line.hop === ROOM_NOTE_HOP) return { ...base, kind: 'note', from: { kind: 'note', name: line.from.name } };
    if (line.from.kind === 'person') return { ...base, kind: 'text', from: { kind: 'you' } };
    return { ...base, kind: 'text', from: { kind: 'bot', name: line.from.name }, file: line.file ?? undefined };
  });
  return { messages, senders };
}

export function RoomThread({ room, transport, potato, bots, avatars, onDeleted, onGone, onNotify, onChanged, jumpTo = null, now = Date.now() }: RoomThreadProps) {
  const { lines, gone, problem } = useRoomLines(transport, room.id);
  const [follow, setFollow] = useState(0);
  const [overlay, setOverlay] = useState<Overlay>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameError, setRenameError] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const [away, setAway] = useState(false);

  // The group was deleted somewhere else: close gracefully.
  const goneHandled = useRef(false);
  useEffect(() => {
    goneHandled.current = false;
  }, [room.id]);
  useEffect(() => {
    if (!gone || goneHandled.current) return;
    goneHandled.current = true;
    (onGone ?? onDeleted)();
  }, [gone, onGone, onDeleted]);

  // Opening a group reads it; a line that arrives while it is open reads again.
  const markRead = (id: string) => {
    void transport.markRead(id).catch(() => {});
  };
  useEffect(() => {
    markRead(room.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [room.id, transport]);
  useEffect(() => {
    if (room.unread > 0 && (typeof document === 'undefined' || document.visibilityState !== 'hidden')) markRead(room.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [room.unread, lines.length]);

  const members = useMemo(() => room.bots.map((chip) => bots.find((b) => b.id === chip.id)), [room.bots, bots]);
  const running = useMemo(() => room.bots.filter((_, i) => members[i] && runButton(members[i]!.brainState) === 'stop'), [room.bots, members]);
  const faceOf = (id: string): AvatarId => avatars?.get(id) ?? 'mochi';

  const { messages, senders } = useMemo(() => asMessages(lines), [lines]);

  // How many lines were unread when the group was opened (opening reads them, so `room.unread` drops to 0 at once).
  // The "New" line stays above the first of them until the person sends something or opens another group.
  const [seen, setSeen] = useState<{ roomId: string; unread: number }>({ roomId: room.id, unread: room.unread });
  const unread = seen.roomId === room.id ? seen.unread : room.unread;
  useEffect(() => {
    setSeen((was) => (was.roomId === room.id ? was : { roomId: room.id, unread: room.unread }));
  }, [room.id, room.unread]);
  // A line that arrives while the person reads further up counts too.
  useEffect(() => {
    if (room.unread > 0 && !pinned.current) setSeen((was) => (was.roomId === room.id ? { roomId: room.id, unread: Math.max(was.unread, room.unread) } : was));
  }, [room.id, room.unread]);
  useEffect(() => {
    if (follow > 0) setSeen({ roomId: room.id, unread: 0 });
  }, [follow, room.id]);
  const unreadId = useMemo(() => firstUnreadId(messages, unread), [messages, unread]);
  const items = useMemo(() => threadItems(messages, { now, unreadId }), [messages, now, unreadId]);

  // ---- scrolling: follow the bottom while the person is at the bottom ------------------------------------------
  useEffect(() => {
    pinned.current = true;
    setAway(false);
  }, [room.id, follow]);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [messages, room.typing, follow]);
  useJumpTo(scroller, jumpTo, messages, () => {
    pinned.current = false;
    setAway(true);
  });
  const toBottom = () => {
    const el = scroller.current;
    if (!el) return;
    pinned.current = true;
    setAway(false);
    const smooth = typeof el.scrollTo === 'function' && !window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (smooth) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
    else el.scrollTop = el.scrollHeight;
  };

  // ---- the menu ------------------------------------------------------------------------------------------------
  const menuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menu) return;
    menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    const onDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenu(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenu(null);
    };
    document.addEventListener('mousedown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('mousedown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
    };
  }, [menu]);

  // ---- commands ------------------------------------------------------------------------------------------------
  const stop = async () => {
    if (stopping) return;
    setStopping(true);
    const results = await Promise.allSettled(running.map((chip) => potato.stopBot(chip.id)));
    const failed = results.findIndex((r) => r.status === 'rejected');
    if (failed >= 0) onNotify(plainError((results[failed] as PromiseRejectedResult).reason, 'Could not stop a bot.'));
    setStopping(false);
    onChanged?.();
  };

  const saveName = async () => {
    const name = renaming ?? '';
    const bad = roomProblem(name, room.bots.map((b) => b.id));
    if (bad) {
      setRenameError(bad);
      return;
    }
    try {
      await transport.rename(room.id, name);
      setRenaming(null);
      setRenameError(null);
    } catch (error) {
      setRenameError(roomErrorText(error, 'Could not rename the group.'));
    }
  };

  const send = async (text: string) => {
    try {
      return await transport.send(room.id, text);
    } catch (error) {
      throw new Error(roomErrorText(error, 'Could not send that.'));
    }
  };

  // The box needs a "bot" only to name its draft and its quotes: the group stands in for one.
  const roomBot = useMemo<PotatoBotView>(
    () => ({ id: `room:${room.id}`, name: room.name, talkPersona: 'fish', brainState: 'ready', lastError: null, typing: false, mood: null, provider: null, unread: 0, lastLine: null, lastAt: null, usedTodayNano: 0, limitCents: 0, openCards: 0 }),
    [room.id, room.name],
  );
  const mentions = useMemo(() => [{ id: ROOM_EVERYONE, name: ROOM_EVERYONE }, ...room.bots.map((b) => ({ id: b.id, name: b.name }))], [room.bots]);

  // ---- the rows --------------------------------------------------------------------------------------------------
  const rows: JSX.Element[] = [];
  let lastSender: string | null = null;
  for (const item of items) {
    switch (item.type) {
      case 'day':
        rows.push(<DaySeparator key={item.key} label={item.label} />);
        lastSender = null;
        break;
      case 'time':
        rows.push(<TimeSeparator key={item.key} label={item.label} />);
        lastSender = null;
        break;
      case 'unread':
        rows.push(<UnreadDivider key={item.key} />);
        lastSender = null;
        break;
      case 'activity':
        rows.push(<ToolActivity key={item.key} steps={item.steps} before={null} />);
        lastSender = null;
        break;
      default: {
        const from = senders.get(item.message.id);
        if (item.type === 'note') {
          rows.push(<MessageRow key={item.message.id} row={item} botId={from?.id ?? ''} botName={from?.name ?? ''} agentLabel="its agent" transport={potato} onChanged={() => {}} now={now} canSave={false} canReact={false} />);
          lastSender = null;
          break;
        }
        // Helper and decision cards are not drawn in a group (version 1): the bot asks in its private chat.
        if (item.type === 'card' || item.type === 'decision' || !from) break;
        const isBot = item.side === 'bot';
        const fresh = lastSender !== from.id;
        lastSender = from.id;
        rows.push(
          <MessageRow
            key={item.message.id}
            row={item.type === 'bubble' && fresh ? { ...item, first: true } : item}
            botId={from.id}
            botName={from.name}
            agentLabel="its agent"
            transport={potato}
            onChanged={() => {}}
            now={now}
            canSave={false}
            canReact={false}
            quoteTarget={roomBot.id}
            header={
              isBot && fresh ? (
                <div className="ck-potato-roomline-from">
                  <Avatar id={faceOf(from.id)} size={16} state={room.typing.includes(from.name) ? 'working' : 'idle'} name={from.name} />
                  <span>{from.name}</span>
                </div>
              ) : undefined
            }
          />,
        );
      }
    }
  }

  const stack = room.bots.slice(0, 3);
  const firstBotName = room.bots[0]?.name ?? '';
  const empty = lines.length === 0 && room.typing.length === 0 && !problem;

  return (
    <section className="ck-potato-thread" aria-label={`Group chat ${room.name}`}>
      <header className="ck-potato-head">
        <span className="ck-potato-roomfaces" aria-hidden="true">
          {stack.map((chip) => (
            <Avatar key={chip.id} id={faceOf(chip.id)} size={28} state="idle" name={chip.name} />
          ))}
        </span>
        <div className="ck-potato-head-main">
          {renaming === null ? (
            <h1 className="ck-potato-name">{room.name}</h1>
          ) : (
            <form
              className="ck-potato-roomrename"
              onSubmit={(e) => {
                e.preventDefault();
                void saveName();
              }}
            >
              <input
                className="ck-potato-adv-input"
                type="text"
                aria-label={ROOM_COPY.nameLabel}
                maxLength={60}
                value={renaming}
                autoFocus
                onChange={(e) => setRenaming(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    setRenaming(null);
                    setRenameError(null);
                  }
                }}
              />
              <button type="submit" className="ck-btn ck-btn--secondary ck-btn--sm">
                {ROOM_COPY.save}
              </button>
              <button
                type="button"
                className="ck-btn ck-btn--plain ck-btn--sm"
                onClick={() => {
                  setRenaming(null);
                  setRenameError(null);
                }}
              >
                {ROOM_COPY.cancel}
              </button>
            </form>
          )}
          <p className="ck-sub ck-potato-brain">{ROOM_COPY.botsCount(room.bots.length)}</p>
          {renameError && (
            <p className="ck-sub ck-potato-starters-error" role="alert">
              {renameError}
            </p>
          )}
        </div>
        {running.length > 0 && (
          <button type="button" className="ck-btn ck-btn--secondary" disabled={stopping} onClick={() => void stop()}>
            {ROOM_COPY.stop}
          </button>
        )}
        <button
          type="button"
          className="ck-potato-round ck-potato-roommenu"
          aria-label={ROOM_COPY.menu}
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          onClick={(e) => {
            const box = e.currentTarget.getBoundingClientRect();
            setMenu(menu ? null : { x: Math.max(8, box.right - MENU_WIDTH), y: box.bottom + 4 });
          }}
        >
          <span aria-hidden="true">…</span>
        </button>
      </header>
      {menu && (
        <div ref={menuRef} className="ck-potato-menu" role="menu" aria-label={ROOM_COPY.menu} style={{ left: menu.x, top: menu.y }}>
          <button
            type="button"
            role="menuitem"
            className="ck-potato-menu-item"
            onClick={() => {
              setMenu(null);
              setRenameError(null);
              setRenaming(room.name);
            }}
          >
            {ROOM_COPY.menuRename}
          </button>
          <button
            type="button"
            role="menuitem"
            className="ck-potato-menu-item"
            onClick={() => {
              setMenu(null);
              setOverlay('bots');
            }}
          >
            {ROOM_COPY.menuBots}
          </button>
          <button
            type="button"
            role="menuitem"
            className="ck-potato-menu-item"
            onClick={() => {
              setMenu(null);
              setOverlay('delete');
            }}
          >
            {ROOM_COPY.menuDelete}
          </button>
        </div>
      )}

      <div className="ck-potato-body-wrap">
        <div
          className="ck-scroll ck-potato-body"
          ref={scroller}
          onScroll={(e) => {
            const el = e.currentTarget;
            const near = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
            pinned.current = near;
            setAway(!near);
          }}
        >
          <ol className="ck-potato-rows" aria-label={THREAD_COPY.messages}>
            {rows}
          </ol>
          {(room.work ?? []).filter((w) => w.run.steps.length > 0).map((w) => (
            <WorkSteps key={w.bot} run={w.run} owner={w.bot} />
          ))}
          {room.typing
            .filter((name) => !(room.work ?? []).some((w) => w.bot === name && w.run.steps.length > 0))
            .map((name) => (
              <TypingRow key={name} name={name} />
            ))}
          {empty && <p className="ck-sub ck-potato-empty">{ROOM_COPY.empty(firstBotName)}</p>}
          {problem && !gone && (
            <p className="ck-sub ck-potato-roomproblem" role="alert">
              {problem}
            </p>
          )}
        </div>
        {away && (
          <button type="button" className="ck-potato-jump" onClick={toBottom}>
            <ThreadIcon name="arrow-down" size={13} />
            <span>{ROOM_COPY.jump}</span>
          </button>
        )}
      </div>

      <Composer
        bot={roomBot}
        transport={potato}
        onChanged={() => {}}
        onNotify={onNotify}
        onSent={() => setFollow((n) => n + 1)}
        bots={mentions}
        attach={null}
        placeholder={ROOM_COPY.placeholder}
        onSend={send}
      />

      {overlay === 'bots' && (
        <NewGroupSheet
          bots={bots.map((b) => ({ id: b.id, name: b.name }))}
          transport={transport}
          edit={room}
          onDone={() => setOverlay(null)}
          onCancel={() => setOverlay(null)}
        />
      )}
      {overlay === 'delete' && (
        <DeleteSheet
          onCancel={() => setOverlay(null)}
          onConfirm={async () => {
            try {
              await transport.remove(room.id);
            } catch (error) {
              if (!isRoomGone(error)) throw new Error(roomErrorText(error, 'Could not delete the group.'));
            }
            setOverlay(null);
            onDeleted();
          }}
        />
      )}
    </section>
  );
}

function DeleteSheet({ onCancel, onConfirm }: { onCancel: () => void; onConfirm: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onCancel]);
  return (
    <div className="ck-potato-starters-overlay" role="presentation">
      <section className="ck-potato-starters" role="dialog" aria-modal="true" aria-label={ROOM_COPY.deleteTitle}>
        <header className="ck-potato-starters-head">
          <h2 className="ck-potato-starters-title">{ROOM_COPY.deleteTitle}</h2>
        </header>
        <p className="ck-sub ck-potato-starters-intro">{ROOM_COPY.deleteBody}</p>
        {error && (
          <p role="alert" className="ck-potato-starters-error">
            {error}
          </p>
        )}
        <footer className="ck-potato-starters-foot">
          <button type="button" className="ck-btn ck-btn--plain ck-btn--sm" onClick={onCancel}>
            {ROOM_COPY.cancel}
          </button>
          <button
            type="button"
            className="ck-btn ck-btn--secondary"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              setError(null);
              onConfirm().catch((e: unknown) => {
                setError(plainError(e, 'Could not delete the group.'));
                setBusy(false);
              });
            }}
          >
            {ROOM_COPY.deleteConfirm}
          </button>
        </footer>
      </section>
    </div>
  );
}
