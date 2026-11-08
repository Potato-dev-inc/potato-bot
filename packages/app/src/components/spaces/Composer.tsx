import { useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { open as pickFile } from '@tauri-apps/plugin-dialog';
import { MAX_TEXT_CHARS, type SpaceMessage } from '../../lib/spaces';
import '../../styles/cockpit-chat.css';
import '../../styles/cockpit-acp.css';
import '../../styles/cockpit-potato-composer.css';

/** The paperclip of the Potato chat composer. */
function PaperclipIcon() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M13 7.4l-4.6 4.6a2.6 2.6 0 0 1-3.7-3.7l5-5a1.7 1.7 0 0 1 2.4 2.4l-5 5a.8.8 0 0 1-1.2-1.2l4.4-4.4" />
    </svg>
  );
}

export const SENDING_TEXT = 'Sending…';
export const SENDING_FILE_TEXT = 'Sending file…';
export const KEPT_TEXT = 'Your message is still in the box. Press Enter to try again.';

/** The counter shows from this size on (90% of the cap). */
export const COUNT_FROM_CHARS = 3600;
/** The box grows with the text up to this many lines, then scrolls. */
const MAX_LINES = 6;

/** Characters as the Rust side counts them (code points, not UTF-16 units). */
export function charLength(text: string): number {
  let n = 0;
  for (const _ of text) n += 1;
  return n;
}

/** A bot the `@` picker can offer. */
export interface CallableBot {
  id: string;
  name: string;
  ownerName: string;
}

/** The `@word` the caret is in, if any: where it starts and what was typed after the `@`. */
export function atToken(text: string, caret: number): { start: number; query: string } | null {
  const m = /(?<![\p{L}\p{N}_])@([^\s@]*)$/u.exec(text.slice(0, caret));
  return m ? { start: m.index, query: m[1] } : null;
}

/** The bots whose name holds what was typed after `@` (any case), names that start with it first. */
export function filterBots(bots: CallableBot[], query: string): CallableBot[] {
  const q = query.toLowerCase();
  const has = (b: CallableBot) => b.name.toLowerCase().indexOf(q);
  return bots.filter((b) => has(b) >= 0).sort((a, b) => (has(a) === 0 ? 0 : 1) - (has(b) === 0 ? 0 : 1));
}

export interface ComposerProps {
  channelName: string;
  replyTo: SpaceMessage | null;
  onCancelReply: () => void;
  onSend: (text: string) => Promise<string | null>; // returns an error text or null
  disabled?: boolean;
  /** Send the file at this path. Returns an error text or null. Without it there is no Attach button. */
  onAttach?: (path: string) => Promise<string | null>;
  /** A line of help under the box (how to call a bot). */
  hint?: string;
  /** Why the line being typed will not get a bot's answer, or null. Shown under the box while typing. */
  problem?: (text: string) => string | null;
  /** Sentences about earlier calls that nobody answered (shown until a bot takes a later line). */
  notes?: string[];
  /** The bots a person can call here: typing `@` offers them. Empty or missing: no picker. */
  bots?: CallableBot[];
}

export function Composer({ channelName, replyTo, onCancelReply, onSend, disabled, onAttach, hint, problem, notes = [], bots = [] }: ComposerProps) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState<'message' | 'file' | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The last failure was a message that stayed in the box (not a file). */
  const [kept, setKept] = useState(false);
  const [caret, setCaret] = useState(0);
  const [pickAt, setPickAt] = useState(0);
  const [pickShut, setPickShut] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const caretAfter = useRef<number | null>(null);
  const chars = charLength(text);
  const tooLong = chars > MAX_TEXT_CHARS;
  const typed = problem?.(text) ?? null;
  const token = atToken(text, caret);
  const options = token && !pickShut && !disabled && !busy ? filterBots(bots, token.query) : [];
  const pickOpen = options.length > 0;
  const pickIx = Math.min(pickAt, Math.max(options.length - 1, 0));

  // Grow with the text, up to MAX_LINES lines.
  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = 'auto';
    const line = parseFloat(getComputedStyle(el).lineHeight);
    const max = (Number.isFinite(line) ? line : 21) * MAX_LINES;
    el.style.height = `${Math.min(el.scrollHeight, max)}px`;
  }, [text]);
  // Put the cursor after a picked name, once the text has been rendered.
  useLayoutEffect(() => {
    const el = field.current;
    if (!el || caretAfter.current === null) return;
    el.setSelectionRange(caretAfter.current, caretAfter.current);
    caretAfter.current = null;
  }, [text]);

  const pick = (bot: CallableBot) => {
    if (!token) return;
    const before = text.slice(0, token.start);
    const after = text.slice(caret);
    const insert = `@${bot.name} `;
    const next = `${before}${insert}${after.startsWith(' ') ? after.slice(1) : after}`;
    caretAfter.current = before.length + insert.length;
    setCaret(before.length + insert.length);
    setText(next);
    setPickAt(0);
    field.current?.focus();
  };
  const send = async () => {
    if (busy || tooLong || text.trim() === '') return;
    setBusy('message');
    const err = await onSend(text);
    setBusy(null);
    if (err) { setError(err); setKept(true); return; }
    setKept(false); setError(null); setText(''); setCaret(0); onCancelReply();
  };
  const attach = async () => {
    if (!onAttach || busy) return;
    let picked: string | string[] | null = null;
    try {
      picked = await pickFile({ multiple: false, directory: false });
    } catch {
      setError("Can't open the file picker.");
      return;
    }
    if (typeof picked !== 'string') return;
    setBusy('file');
    const err = await onAttach(picked);
    setBusy(null);
    setKept(false);
    setError(err);
  };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const composing = e.nativeEvent.isComposing;
    if (pickOpen && !composing) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        setPickAt((pickIx + (e.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length);
        return;
      }
      if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
        e.preventDefault();
        pick(options[pickIx]);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        setPickShut(true);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && !composing) { e.preventDefault(); void send(); }
  };
  return (
    <div className="ck-spaces-composer ck-chat-composer">
      {pickOpen && (
        <div className="ck-slash-anchor">
          <ul className="ck-slash-menu" id="ck-spaces-bot-menu" role="listbox" aria-label="Bots here">
            {options.map((b, i) => (
              <li key={b.id} id={`ck-spaces-bot-opt-${i}`} role="option" aria-selected={i === pickIx} className="ck-slash-opt"
                onMouseDown={(e) => e.preventDefault()} onMouseEnter={() => setPickAt(i)} onClick={() => pick(b)}>
                <span className="ck-slash-name">{b.name}</span>
                <span className="ck-slash-desc">Bot · {b.ownerName}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {replyTo && (
        <div className="ck-spaces-composer-reply">
          <span>Replying to {replyTo.authorName}</span>
          <button type="button" className="ck-spaces-quiet" onClick={onCancelReply}>Cancel reply</button>
        </div>
      )}
      <div className="ck-chat-composer-row">
        <textarea ref={field} className="ck-chat-composer-field" aria-label={`Message #${channelName}`}
          placeholder={`Message #${channelName}`} rows={1} value={text} disabled={disabled || !!busy}
          {...(bots.length > 0 ? {
            role: 'combobox' as const, 'aria-autocomplete': 'list' as const, 'aria-expanded': pickOpen,
            'aria-controls': 'ck-spaces-bot-menu', 'aria-activedescendant': pickOpen ? `ck-spaces-bot-opt-${pickIx}` : undefined,
          } : {})}
          onChange={(e) => { setText(e.target.value); setCaret(e.target.selectionStart); setPickAt(0); setPickShut(false); }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
          onKeyDown={onKey} />
        <div className="ck-chat-composer-toolbar">
          {onAttach && (
            <button type="button" className="ck-potato-round ck-spaces-attach" aria-label="Attach file" title="Attach file"
              disabled={disabled || !!busy} onClick={() => void attach()}>
              <PaperclipIcon />
            </button>
          )}
        </div>
        <button type="button" className="ck-btn ck-btn--primary ck-chat-send"
          disabled={disabled || !!busy || tooLong || text.trim() === ''} onClick={() => void send()}>
          <svg className="ck-chat-send-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="M12 19V5" />
            <path d="m5 12 7-7 7 7" />
          </svg>
          <span className="ck-chat-send-label">Send</span>
        </button>
      </div>
      {busy && <p className="ck-spaces-callnote" role="status">{busy === 'file' ? SENDING_FILE_TEXT : SENDING_TEXT}</p>}
      {hint && <p className="ck-spaces-composer-hint">{hint}</p>}
      {typed && <p className="ck-spaces-callnote" role="status">{typed}</p>}
      {!typed && notes.map((n) => <p key={n} className="ck-spaces-callnote" role="status">{n}</p>)}
      {chars > COUNT_FROM_CHARS && (
        <span className={`ck-spaces-count${tooLong ? ' is-error' : ''}`}>{chars} / {MAX_TEXT_CHARS}</span>
      )}
      {tooLong && <p className="ck-spaces-hint is-error">This message is too long. The limit is {MAX_TEXT_CHARS} characters.</p>}
      {error && <p className="ck-spaces-hint is-error" role="alert">{error}</p>}
      {error && kept && <p className="ck-spaces-composer-hint">{KEPT_TEXT}</p>}
    </div>
  );
}
