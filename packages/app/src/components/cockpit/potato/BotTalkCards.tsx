// The two cards for bots giving work to bots. Both are the app's record of what happened, not words a bot chose to show:
// - a hand-off card in a group: who gave the task, to whom, what it is, and whether it was passed on (the hop limit can stop it)
// - a peer card in a bot's private chat: this bot wrote to another of your bots, was written to by one, or got its answer
import type { PotatoHandoff, PotatoMessage, PotatoPeerCard } from '../../../lib/potato';

export const BOT_TALK_COPY = {
  handoffLabel: 'Hand-off',
  passed: 'Passed on',
  notPassed: 'Not passed on: the hand-off limit was reached',
  peerLabel: 'Message between bots',
  peerOut: (to: string) => `You asked this bot to write to ${to}`,
  peerIn: (from: string) => `${from} wrote to this bot`,
  peerReply: (from: string) => `${from} answered`,
} as const;

export function peerTitle(peer: PotatoPeerCard): string {
  return peer.dir === 'out' ? BOT_TALK_COPY.peerOut(peer.botName) : peer.dir === 'in' ? BOT_TALK_COPY.peerIn(peer.botName) : BOT_TALK_COPY.peerReply(peer.botName);
}

export function HandoffCard({ from, handoff }: { from: string; handoff: PotatoHandoff }) {
  return (
    <div className={`ck-potato-botcard ck-potato-handoff${handoff.passed ? '' : ' is-stopped'}`} role="group" aria-label={BOT_TALK_COPY.handoffLabel}>
      <p className="ck-potato-botcard-head">
        <span className="ck-potato-botcard-who">{from}</span>
        <span className="ck-potato-botcard-arrow" aria-hidden="true">
          {' → '}
        </span>
        <span className="ck-potato-botcard-who">{handoff.toName}</span>
      </p>
      <p className="ck-potato-botcard-body">{handoff.task}</p>
      <p className="ck-potato-botcard-state">{handoff.passed ? BOT_TALK_COPY.passed : BOT_TALK_COPY.notPassed}</p>
    </div>
  );
}

export function PeerCard({ message }: { message: PotatoMessage }) {
  const peer = message.peer!;
  return (
    <div className={`ck-potato-botcard ck-potato-peer is-${peer.dir}`} role="group" aria-label={BOT_TALK_COPY.peerLabel}>
      <p className="ck-potato-botcard-head">{peerTitle(peer)}</p>
      <p className="ck-potato-botcard-body">{message.text}</p>
    </div>
  );
}
