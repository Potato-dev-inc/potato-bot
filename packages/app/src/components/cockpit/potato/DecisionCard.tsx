// A decision card in the thread (Professional mode, shaped like HelperCard): an approval (Allow once, Allow for this
// job, Deny) with the action in a code-style line, or a question with one button per choice. A Keep going card shows its
// choices as given. Once there is an answer (from the thread, or from this click) the buttons go and a quiet line says
// what was chosen. A failed answer shows its sentence and leaves the buttons on.
import { useState } from 'react';
import { openSettingsAt } from '../../../lib/openSettingsAt';
import { plainError, type PotatoDecisionCard } from '../../../lib/potato';
import '../../../styles/cockpit-potato-cards.css';

export const DECISION_COPY = {
  allowOnce: 'Allow once',
  allowJob: 'Allow for this job',
  alwaysAllow: 'Always allow',
  deny: 'Deny',
  cancelled: 'Cancelled',
  problem: 'That did not work. Try again.',
};

/** A choice number from a result row: `choice:2` or `2`. Null for any other answer. */
export function choiceIndex(answer: string): number | null {
  const match = /^(?:choice:)?(\d+)$/.exec(answer);
  return match ? Number(match[1]) : null;
}

/** The quiet line after an answer: what was chosen, in plain words. */
export function answeredLine(answer: string, choices: readonly string[]): string {
  switch (answer) {
    case 'cancelled':
      return DECISION_COPY.cancelled;
    case 'allow_once':
      return 'Allowed once.';
    case 'allow_job':
      return 'Allowed for this job.';
    case 'always_allow':
      return 'Always allowed for this kind of command.';
    case 'deny':
      return 'Denied.';
  }
  const index = choiceIndex(answer);
  const label = index === null ? undefined : choices[index];
  return label ? `You chose: ${label}.` : 'Answered.';
}

/** The row of Settings > Potato Bot that opens Connectors. */
export const CONNECTORS_ROW_ID = 'potato-connectors';

export interface DecisionCardProps {
  /** The bot's name, for the approval's title. */
  botName: string;
  card: PotatoDecisionCard;
  /** The card's text: the action an approval asks about, or the question. */
  text: string;
  /** The answer from the thread (`decision_result`), once there is one. */
  answer?: string | null;
  /** Sends the answer: `allow_once`, `allow_job`, `deny`, or a choice number from 0 as text. Rejects with a sentence on failure. */
  onAnswer: (answer: string) => Promise<void>;
  /** Sends Always allow through the app's own call. Without it the button is not shown. */
  onAlwaysAllow?: () => Promise<void>;
}

export function DecisionCard({ botName, card, text, answer, onAnswer, onAlwaysAllow }: DecisionCardProps) {
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [sent, setSent] = useState<string | null>(null);
  const settled = answer ?? sent;
  const approval = card.kind === 'approval' && !card.keepGoing;
  const choices = approval ? [] : card.choices;

  /** Runs one answer call; the buttons wait for it, and a failure shows its sentence and leaves them on. */
  const run = async (call: () => Promise<void>, wire: string) => {
    if (busy) return;
    setBusy(true);
    setProblem(null);
    try {
      await call();
      setSent(wire);
    } catch (error) {
      setProblem(plainError(error, DECISION_COPY.problem));
    } finally {
      setBusy(false);
    }
  };

  const send = async (wire: string) => {
    if (busy) return;
    // A connector card's first button opens Settings > Connectors, then answers the card. The bot never gets a token.
    if (card.connector && wire === '0') openSettingsAt('potato', CONNECTORS_ROW_ID);
    setBusy(true);
    setProblem(null);
    try {
      await onAnswer(wire);
      setSent(wire);
    } catch (error) {
      setProblem(plainError(error, DECISION_COPY.problem));
    } finally {
      setBusy(false);
    }
  };

  const buttons = settled === null && (
    <div className="ck-potato-card-actions">
      {approval ? (
        <>
          <button type="button" className="ck-btn ck-btn--primary" disabled={busy} onClick={() => void send('allow_once')}>
            {DECISION_COPY.allowOnce}
          </button>
          <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void send('allow_job')}>
            {DECISION_COPY.allowJob}
          </button>
          {card.canAlwaysAllow && onAlwaysAllow && (
            <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void run(onAlwaysAllow, 'always_allow')}>
              {DECISION_COPY.alwaysAllow}
            </button>
          )}
          <button type="button" className="ck-btn ck-btn--plain" disabled={busy} onClick={() => void send('deny')}>
            {DECISION_COPY.deny}
          </button>
        </>
      ) : (
        choices.map((label, index) => (
          <button
            key={`${index}-${label}`}
            type="button"
            className={`ck-btn ${index === 0 ? 'ck-btn--primary' : 'ck-btn--secondary'}`}
            disabled={busy}
            onClick={() => void send(String(index))}
          >
            {label}
          </button>
        ))
      )}
    </div>
  );

  return (
    <section
      className={`ck-potato-card ck-potato-decision${settled === null ? '' : ' is-closed'}`}
      aria-label={approval ? `Approval from ${botName}` : `Question from ${botName}`}
    >
      {approval ? (
        <>
          <p className="ck-potato-card-title">{botName} needs your OK to run:</p>
          <p className="ck-potato-decision-action">
            <code>{text}</code>
          </p>
        </>
      ) : (
        <p className="ck-potato-card-title">{text}</p>
      )}
      {card.why && <p className="ck-sub ck-potato-card-fact">{card.why}</p>}
      {settled !== null ? (
        <p className="ck-sub ck-potato-decision-result">{answeredLine(settled, card.choices)}</p>
      ) : (
        buttons
      )}
      {problem && (
        <p className="ck-sub ck-potato-card-problem" role="status">
          {problem}
        </p>
      )}
    </section>
  );
}
