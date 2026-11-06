// The Search chats page: one box over every bot chat and group thread. It takes the thread's place while it is open
// ("Back to chat" gives it back). Typing searches after a short pause; the search runs in the app off the UI thread and
// an answer that is no longer for what is typed is dropped. Each result shows the bot (or group), who wrote it, when, and
// a snippet with the words marked; choosing one opens that thread and jumps to the message.
import { useEffect, useRef, useState } from 'react';
import { formatWhen, plainError } from '../../../lib/potato';
import { SEARCH_COPY, searchWords, snippetParts, type SearchHit, type SearchResults, type SearchTransport } from '../../../lib/potato-search';
import '../../../styles/cockpit-potato-search.css';

export interface SearchChatsPageProps {
  transport: SearchTransport;
  /** Open the thread of the hit and jump to its message. */
  onOpenHit: (hit: SearchHit) => void;
  /** "Back to chat". */
  onClose: () => void;
  /** The pause after typing before a search starts (milliseconds). Tests pass 0. */
  debounceMs?: number;
  now?: number;
}

type State = { phase: 'idle' } | { phase: 'searching' } | { phase: 'done'; results: SearchResults } | { phase: 'error'; message: string };

export function SearchChatsPage({ transport, onOpenHit, onClose, debounceMs = 250, now = Date.now() }: SearchChatsPageProps) {
  const [text, setText] = useState('');
  const [state, setState] = useState<State>({ phase: 'idle' });
  const asked = useRef(0);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
  }, []);

  useEffect(() => {
    const query = searchWords(text);
    const mine = ++asked.current;
    if (query === '') {
      setState({ phase: 'idle' });
      return;
    }
    setState({ phase: 'searching' });
    const timer = window.setTimeout(() => {
      transport.search(query).then(
        (results) => {
          if (mine === asked.current) setState({ phase: 'done', results });
        },
        (error) => {
          if (mine === asked.current) setState({ phase: 'error', message: plainError(error, SEARCH_COPY.error) });
        },
      );
    }, debounceMs);
    return () => window.clearTimeout(timer);
  }, [text, transport, debounceMs]);

  const results = state.phase === 'done' ? state.results : null;

  return (
    <section className="ck-search" aria-label={SEARCH_COPY.title}>
      <header className="ck-search-head">
        <h2 className="ck-search-title">{SEARCH_COPY.title}</h2>
        <button type="button" className="ck-btn ck-btn--secondary" onClick={onClose}>
          {SEARCH_COPY.back}
        </button>
      </header>
      <div className="ck-search-bar">
        <input
          ref={input}
          type="search"
          className="ck-search-input"
          aria-label={SEARCH_COPY.title}
          placeholder={SEARCH_COPY.placeholder}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') onClose();
          }}
        />
      </div>
      <div className="ck-search-body" aria-live="polite">
        {state.phase === 'idle' && <p className="ck-sub ck-search-note">{SEARCH_COPY.hint}</p>}
        {state.phase === 'searching' && <p className="ck-sub ck-search-note">{SEARCH_COPY.searching}</p>}
        {state.phase === 'error' && (
          <p className="ck-sub ck-search-note" role="alert">
            {state.message}
          </p>
        )}
        {results && results.hits.length === 0 && <p className="ck-sub ck-search-note">{SEARCH_COPY.none}</p>}
        {results && results.hits.length > 0 && (
          <>
            <ul className="ck-search-list">
              {results.hits.map((hit) => (
                <li key={`${hit.kind}:${hit.ownerId}:${hit.messageId}`}>
                  <button type="button" className="ck-search-hit" aria-label={SEARCH_COPY.open(hit.ownerName)} onClick={() => onOpenHit(hit)}>
                    <span className="ck-search-meta">
                      <span className="ck-search-owner">{hit.ownerName}</span>
                      {hit.kind === 'group' && <span className="ck-search-kind">{SEARCH_COPY.group}</span>}
                      <span className="ck-search-who">{hit.speaker}</span>
                      <span className="ck-search-when">{formatWhen(hit.at, now)}</span>
                    </span>
                    <span className="ck-search-snippet">
                      {snippetParts(hit).map((part, i) => (part.hit ? <mark key={i}>{part.text}</mark> : <span key={i}>{part.text}</span>))}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
            {results.matches > results.hits.length && <p className="ck-sub ck-search-note">{SEARCH_COPY.more(results.hits.length, results.matches)}</p>}
          </>
        )}
      </div>
    </section>
  );
}
