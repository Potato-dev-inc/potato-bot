// A helper pane opened from the Potato Bot thread: the real terminal, its approval card and any
// questions the helper asked the person.
import { useMemo } from 'react';
import type { CockpitApi } from '../../hooks/useCockpit';
import { TerminalViewport } from '../TerminalViewport';
import { PaneApprovalCard } from './PaneApprovalCard';
import { QuestionCard } from './QuestionCard';

export function PotatoPaneView({ c, paneId, onBack }: { c: CockpitApi; paneId: string; onBack: () => void }) {
  const pane = c.registry.panes.get(paneId);
  const name = c.names.get(paneId);
  const questions = useMemo(
    () => c.asked.questions.filter((q) => q.pane_id === paneId && !q.consent && !q.limit && !q.kind),
    [c.asked.questions, paneId],
  );

  if (!pane) {
    return (
      <div className="ck-spawn ck-spawn--plain">
        <h1 className="ck-title">This pane is closed</h1>
        <button type="button" className="ck-btn ck-btn--secondary" onClick={onBack}>
          Back to the bot
        </button>
      </div>
    );
  }

  return (
    <div className="ck-pane-view">
      <header className="ck-pane-head">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={onBack}>
          Back to the bot
        </button>
        <strong>{name ?? pane.info.title ?? pane.info.agent_type}</strong>
        <button type="button" className="ck-btn ck-btn--secondary" onClick={() => void c.closePane(paneId)}>
          Close pane
        </button>
      </header>
      <div className="ck-docked-cards">
        {c.prompts[paneId] && <PaneApprovalCard c={c} paneId={paneId} autoFocus />}
        {questions.map((q) => (
          <QuestionCard
            key={q.id}
            q={q}
            paneName={name}
            now={c.now}
            onAnswer={(answer) => void c.asked.answerQuestion(q.id, answer)}
            onDismiss={() => void c.asked.dismissQuestion(q.id)}
          />
        ))}
      </div>
      <TerminalViewport pane={pane} registry={c.registry} chrome={false} />
    </div>
  );
}
