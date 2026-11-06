// One row of a thread: a bubble, a small centred note, or a helper card. The bot's words are Markdown (no raw HTML; a
// web link opens in the system browser); yours stay as typed. Each bubble has a hover toolbar (Copy, Reply). Transcript
// only decides the order and the lines between rows.
import { useRef, useState, type ReactNode } from 'react';
import { formatWhen, type PotatoMessage, type PotatoTransport, type PotatoVoice } from '../../../lib/potato';
import { emitQuote } from '../../../lib/potato-thread';
import { tauri } from '../../../lib/tauri';
import { ChatMarkdown } from '../../chat/ChatMarkdown';
import { DecisionCard } from './DecisionCard';
import { LoginCard } from './LoginCard';
import { FileCard } from './FileCard';
import { PlotCard } from './PlotCard';
import { HelperCard } from './HelperCard';
import { HoverActions } from './HoverActions';
import { ReactButton, ReactionChips } from './Reactions';
import { SaveAsFile } from './SaveAsFile';
import { SettingChangeNote } from './SettingChangeNote';
import { HandoffCard, PeerCard } from './BotTalkCards';
import { SpeakButton } from './SpeakButton';
import type { ThreadRow } from './model';

/** The app's external-link helper; outside the desktop shell it does nothing. */
const openInBrowser = (url: string) => {
  void Promise.resolve(tauri.openExternalUrl(url)).catch(() => {});
};

export interface MessageRowProps {
  row: ThreadRow;
  botId: string;
  botName: string;
  agentLabel: string;
  transport: PotatoTransport;
  onChanged: () => void;
  /** Opens a helper's pane (the Open button of a finished helper). Absent: no button. */
  onOpenPane?: (paneId: string) => void;
  /** Whether that pane is still there; the button is shown only then. Default: yes. */
  paneExists?: (paneId: string) => boolean;
  now: number;
  /** Opens a link from the bot's text. Default: the system browser. */
  openLink?: (url: string) => void;
  /** The voice settings: a bot reply's Speak button reads with them. */
  voice?: PotatoVoice;
  /** Above the bubble: in a group, who said it (name and face). Absent in a bot's own thread. */
  header?: ReactNode;
  /** Where Reply sends its quote. Default: the bot (a group passes its own id). */
  quoteTarget?: string;
  /** Whether the Save as file button is offered. Default: yes (a group line is not a message of one bot's thread). */
  canSave?: boolean;
  /** Whether the person can react to this message. Default: yes (a group line is not a message of one bot's thread). */
  canReact?: boolean;
  /** A sign-in card's Take over: takes the screen and opens the computer. Absent: Take over only answers the card. */
  onTakeOver?: () => Promise<void>;
}

export function MessageRow({ row, botId, botName, agentLabel, transport, onChanged, onOpenPane, paneExists, now, openLink = openInBrowser, voice, header, quoteTarget, canSave = true, canReact = true, onTakeOver }: MessageRowProps) {
  // The text the person has selected in this bubble (kept for the Quote selection button). Hooks stay above the early returns.
  const bubble = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState('');
  const readSelection = () => {
    const sel = typeof window !== 'undefined' ? window.getSelection() : null;
    const inside = !!sel && !sel.isCollapsed && !!bubble.current && !!sel.anchorNode && bubble.current.contains(sel.anchorNode);
    setPicked(inside ? sel!.toString().trim() : '');
  };
  const m: PotatoMessage = row.message;
  if (row.type === 'note' && m.peer) {
    return (
      <li className="ck-potato-botcard-row" data-message-id={m.id}>
        <PeerCard message={m} />
      </li>
    );
  }
  if (row.type === 'note' && m.handoff) {
    return (
      <li className="ck-potato-botcard-row" data-message-id={m.id}>
        <HandoffCard from={botName} handoff={m.handoff} />
      </li>
    );
  }
  if (row.type === 'note' && m.change) {
    return (
      <li className="ck-potato-note ck-potato-change" role="note" data-message-id={m.id}>
        <SettingChangeNote message={m} botId={botId} transport={transport} onChanged={onChanged} />
      </li>
    );
  }
  if (row.type === 'note') {
    const canOpen = m.kind === 'helper_result' && !!m.paneId && !!onOpenPane && (paneExists?.(m.paneId) ?? true);
    return (
      <li className="ck-potato-note" role="note">
        {m.text}
        {canOpen && (
          <>
            {' '}
            <button type="button" className="ck-potato-note-open" onClick={() => onOpenPane?.(m.paneId!)}>
              Open
            </button>
          </>
        )}
      </li>
    );
  }
  if (row.type === 'decision' && m.card?.login) {
    // A sign-in card: the password goes from its box to the app and never through here. `answer` for Sign in is only ever
    // recorded by the app after it filled the page; a plain `answerCard` of 0 is refused there.
    const cardId = m.requestId ?? '';
    const answer = async (wire: string) => {
      await transport.answerCard(botId, cardId, wire);
      onChanged();
    };
    return (
      <li className="ck-potato-card-row">
        <LoginCard
          botName={botName}
          card={m.card}
          text={m.text}
          answer={row.result?.card?.answer ?? null}
          loadOrigin={() => transport.loginOrigin(botId, cardId)}
          onSignIn={async (username, password) => {
            await transport.submitLogin(botId, cardId, username, password);
            onChanged();
          }}
          onConfirm={async () => {
            await transport.confirmLogin(botId, cardId);
            onChanged();
          }}
          onTakeOver={async () => {
            await onTakeOver?.();
            await answer('1');
          }}
          onNotNow={() => answer('2')}
          onForget={async () => (await transport.forgetSite(botId, cardId)).cookiesCleared}
        />
      </li>
    );
  }
  if (row.type === 'decision') {
    return (
      <li className="ck-potato-card-row">
        <DecisionCard
          botName={botName}
          card={m.card ?? { kind: 'approval', why: '', choices: [] }}
          text={m.text}
          answer={row.result?.card?.answer ?? null}
          onAnswer={async (answer) => {
            await transport.answerCard(botId, m.requestId ?? '', answer);
            onChanged();
          }}
          onAlwaysAllow={async () => {
            await transport.answerCardAlways(botId, m.requestId ?? '');
            onChanged();
          }}
        />
      </li>
    );
  }
  if (row.type === 'card') {
    return (
      <li className="ck-potato-card-row">
        <HelperCard
          name={botName}
          task={m.text}
          project={m.project}
          agentLabel={agentLabel}
          open={row.open}
          onStart={async (folder) => {
            await transport.helperStart(botId, m.requestId ?? '', folder);
            onChanged();
          }}
          onNotNow={async () => {
            await transport.helperDecline(botId, m.requestId ?? '');
            onChanged();
          }}
          loadFolders={() => transport.helperFolders(botId)}
        />
      </li>
    );
  }
  const reactions = canReact ? (m.reactions ?? []) : [];
  const toggleReaction = (emoji: string) => {
    void transport
      .toggleReaction(botId, m.id, emoji)
      .then(() => onChanged())
      .catch(() => {});
  };
  return (
    <li data-message-id={m.id} className={`ck-potato-line ck-potato-line--${row.side}${row.first ? ' is-first' : ''}${header ? ' has-from' : ''}${reactions.length > 0 ? ' has-reactions' : ''}`}>
      {header}
      <div
        ref={bubble}
        className={`ck-potato-bubble ck-potato-bubble--${row.side}${row.side === 'bot' ? ' ck-potato-bubble--md' : ''}`}
        title={formatWhen(m.at, now)}
        onMouseUp={readSelection}
        onKeyUp={readSelection}
      >
        {m.file ? (
          <FileCard botId={botId} file={m.file} files={transport.artifacts} />
        ) : m.chart ? (
          <PlotCard chart={m.chart} />
        ) : row.side === 'bot' ? (
          <ChatMarkdown text={m.text} onOpenLink={openLink} />
        ) : (
          <>
            {m.text}
            {m.queued && <span className="ck-potato-queued">Queued</span>}
          </>
        )}
      </div>
      <HoverActions
        text={m.text}
        onReply={() => emitQuote(quoteTarget ?? botId, m.text)}
        costNano={m.costNano}
        selectedText={picked}
        onQuoteSelected={(text) => {
          emitQuote(quoteTarget ?? botId, text);
          setPicked('');
        }}
      >
        {row.side === 'bot' && !m.file && !m.chart && <SpeakButton id={m.id} text={m.text} voice={voice} />}
        {canReact && <ReactButton current={reactions} onPick={toggleReaction} />}
        {canSave && !m.file && !m.chart && <SaveAsFile botId={botId} messageId={m.id} files={transport.artifacts} />}
      </HoverActions>
      <ReactionChips reactions={reactions} onToggle={toggleReaction} />
    </li>
  );
}
