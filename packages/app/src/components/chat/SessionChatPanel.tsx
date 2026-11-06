// The chat over a terminal pane: reads the pane's session file while it is mounted. `dock` is what
// waits on the person, shown under the conversation. `reply` gives the chat a message box.
import { useMemo, type ReactNode } from 'react';
import { usePaneOutbox } from '../../hooks/usePaneOutbox';
import { tauriOutboxTransport } from '../../lib/paneOutbox';
import { useComposerDraft } from '../../lib/composerDraft';
import { usePendingBubbles } from '../../hooks/usePendingBubbles';
import { useSessionChat } from '../../hooks/useSessionChat';
import { ChatComposer } from './ChatComposer';
import type { EndingActions } from './TurnEndingButtons';
import { PendingMessages } from './PendingMessages';
import { ScreenQuestionCard } from './ScreenQuestionCard';
import { SessionChatView } from './SessionChatView';
import { useChatJump } from '../../lib/chatJump';
import type { ChatHelper } from '../../lib/chat-helpers';
import type { AcpCommand } from '../../lib/acpChat';

export interface ChatReply {
  /** Why the message box is switched off, or null. */
  blockedReason: string | null;
  /** The agent is working: the box shows Stop instead of Send (see ChatComposer). */
  running?: boolean;
  /** Stops the agent. Set for a Chat (terminal) pane only: a chat session over ACP has its own Stop (AcpChatDock). */
  onStop?: () => void;
  /** Stops the running turn so a waiting message can go at once ("Send now"). */
  onInterrupt?: () => void;
}

/** What the chat needs to show "Agent is asking" for a question that only the terminal screen holds. */
export interface ScreenAsk {
  waiting: boolean;
  idle: boolean;
  running?: boolean;
  covered: boolean;
  fallbackText?: string | null;
  onOpenTerminal?: () => void;
  answerable?: string | null;
}

export function SessionChatPanel({
  paneId,
  dock,
  reply,
  emptyNote,
  onOpenPane,
  screenAsk,
  helpers,
  slashCommands,
  composerTop,
}: {
  paneId: string;
  dock?: ReactNode;
  reply?: ChatReply;
  emptyNote?: ReactNode;
  onOpenPane?: (paneId: string) => void;
  screenAsk?: ScreenAsk;
  helpers?: readonly ChatHelper[];
  slashCommands?: readonly AcpCommand[];
  composerTop?: ReactNode;
}) {
  const chat = useSessionChat(paneId, true);
  const outbox = usePaneOutbox(tauriOutboxTransport, paneId, !!reply);
  const userTexts = useMemo(() => chat.turns.flatMap((turn) => (turn.user ? [turn.user.text] : [])), [chat.turns]);
  const { bubbles, hide } = usePendingBubbles(outbox.messages, userTexts);
  const jump = useChatJump(paneId);
  const draft = useComposerDraft(reply ? paneId : null);
  const newest = chat.turns[chat.turns.length - 1];
  const newestText = newest?.user?.text;
  const endings: EndingActions | undefined =
    chat.state !== 'unavailable' && reply && newestText ? { onRetry: () => outbox.send(newestText) } : undefined;

  return (
    <SessionChatView
      state={chat.state}
      reason={chat.reason}
      waitingFor={chat.waitingFor}
      source={chat.source}
      turns={chat.turns}
      model={chat.model}
      cliVersion={chat.cliVersion}
      paneId={paneId}
      jump={jump}
      helpers={helpers}
      onOpenPane={onOpenPane}
      endings={endings}
      emptyNote={emptyNote}
      dock={dock}
      aside={
        screenAsk && (
          <ScreenQuestionCard
            paneId={paneId}
            waiting={screenAsk.waiting}
            idle={screenAsk.idle}
            running={screenAsk.running}
            covered={screenAsk.covered}
            queued={outbox.messages}
            fallbackText={screenAsk.fallbackText}
            onOpenTerminal={screenAsk.onOpenTerminal}
            answerable={screenAsk.answerable}
          />
        )
      }
      pending={reply ? <PendingMessages messages={bubbles} onCancel={outbox.cancel} onHide={hide} onSendNow={reply.onInterrupt} /> : undefined}
      composer={
        reply ? (
          <ChatComposer
            onSend={outbox.send}
            blockedReason={reply.blockedReason}
            draftKey={paneId}
            draft={draft}
            slashCommands={slashCommands}
            toolbar={composerTop}
            running={reply.running}
            onStop={reply.onStop}
          />
        ) : undefined
      }
    />
  );
}
