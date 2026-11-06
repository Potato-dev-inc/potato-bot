// The live step list under a working bot: a header with the running step, the elapsed time and a chevron, then a vertical
// list on a thin rail that fills as steps finish. The running step has a spinning arc, a soft letter wave and a small
// three-line log; finished steps show their time. When the turn ends the list folds to "Worked for 1m 20s · 4 steps"
// and can be opened again. An <output> announces the step. Reduced motion turns every movement off (see the CSS).
//
// Design idea: KokonutUI "AI State Loading" v2 (https://kokonutui.com/docs/ai/ai-loading), MIT, Copyright (c) 2025
// kokonutUI. This is a rewrite from that idea in plain CSS and the cockpit's monochrome tokens; none of its source
// (or its `motion` and `lucide-react` dependencies) is copied, so no licence text travels with this file.
import { useEffect, useId, useState } from 'react';
import {
  WORK_COPY,
  announce,
  elapsedLabel,
  foldLong,
  hasMore,
  headStep,
  isSettled,
  stepText,
  stepTime,
  workTitle,
  type WorkRunView,
  type WorkStepView,
} from '../../../lib/potato-work';
import { ThreadIcon } from './ThreadIcons';
import '../../../styles/cockpit-potato-work.css';

export interface WorkStepsProps {
  run: WorkRunView;
  /** The bot's name, in a group (one list per working bot). */
  owner?: string;
  /** The clock, for tests. */
  now?: number;
}

/** Re-reads the clock every second while `on`. */
function useClock(on: boolean, fixed?: number): number {
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => {
    if (!on || fixed !== undefined) return undefined;
    const timer = window.setInterval(() => setTick(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [on, fixed]);
  return fixed ?? tick;
}

export function WorkSteps({ run, owner, now }: WorkStepsProps) {
  const settled = isSettled(run);
  const clock = useClock(!settled, now);
  const [picked, setPicked] = useState<boolean | null>(null);
  const [allRows, setAllRows] = useState(false);
  const [openRows, setOpenRows] = useState<ReadonlySet<string>>(new Set());
  const listId = useId();
  // Open while it works (and when it failed, so the X is seen); folded once the turn ended well. The person's own click wins.
  const open = picked ?? (!settled || run.failed);
  const head = headStep(run);
  const end = run.endedMs ?? clock;
  const total = elapsedLabel(end - run.startedMs);
  const done = run.steps.filter((s) => s.status === 'done').length;
  // Only a turn that really failed is "stopped"; a step with an X in a turn that goes on is just a row with an X.
  const failed = run.failed;
  const title = workTitle(run, end);
  const { hidden, shown } = allRows ? { hidden: [], shown: run.steps } : foldLong(run.steps);
  const toggleRow = (key: string) =>
    setOpenRows((now) => {
      const next = new Set(now);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  return (
    <div role="group" className={`ck-work${failed ? ' is-failed' : ''}${settled ? ' is-settled' : ''}`} data-testid="work-steps" aria-label={owner ? `${owner}: ${WORK_COPY.steps}` : WORK_COPY.steps}>
      {owner && <span className="ck-work-owner">{owner}</span>}
      <button type="button" className="ck-work-head" aria-expanded={open} aria-controls={listId} onClick={() => setPicked(!open)} title={open ? WORK_COPY.hide : WORK_COPY.show}>
        <span className="ck-work-chevron" data-open={open || undefined}>
          <ThreadIcon name="chevron" size={12} />
        </span>
        <span className="ck-work-title">{!settled && head ? <Wave text={title} /> : title}</span>
        {!settled && <span className="ck-work-clock">{total}</span>}
      </button>
      <output className="ck-work-sr" aria-live="polite">
        {announce(run, owner)}
      </output>
      {open && (
        <ol id={listId} className="ck-work-list" aria-label={WORK_COPY.steps}>
          <span className="ck-work-rail" aria-hidden="true">
            <span className="ck-work-rail-fill" style={{ transform: `scaleY(${run.steps.length === 0 ? 0 : done / run.steps.length})` }} />
          </span>
          {(hidden.length > 0 || allRows) && run.steps.length > 6 && (
            <li className="ck-work-earlier">
              <button type="button" className="ck-work-earlier-btn" onClick={() => setAllRows(!allRows)}>
                {allRows ? WORK_COPY.earlierHide : WORK_COPY.earlier(hidden.length || run.steps.length - 5)}
              </button>
            </li>
          )}
          {shown.map((step, i) => {
            const key = `${step.id}:${step.startedMs}:${i}`;
            return <Row key={key} step={step} open={openRows.has(key)} onToggle={() => toggleRow(key)} />;
          })}
        </ol>
      )}
    </div>
  );
}

function Row({ step, open, onToggle }: { step: WorkStepView; open: boolean; onToggle: () => void }) {
  const live = step.status === 'running' || step.status === 'retry';
  const time = stepTime(step);
  const label = stepText(step);
  const more = hasMore(step);
  return (
    <li className={`ck-work-step is-${step.status}`}>
      <span className="ck-work-mark" data-status={step.status}>
        <Mark status={step.status} />
      </span>
      {more ? (
        <button type="button" className="ck-work-label ck-work-label-btn" aria-expanded={open} onClick={onToggle} title={label}>
          {live ? <Wave text={label} /> : label}
        </button>
      ) : (
        <span className="ck-work-label" title={label}>
          {live ? <Wave text={label} /> : label}
        </span>
      )}
      {time && <span className="ck-work-time">{time}</span>}
      {step.why && <span className="ck-work-why">{step.why}</span>}
      {step.status === 'error' && step.error && <span className="ck-work-error">{step.error}</span>}
      {open && more && (
        <span className="ck-work-more">
          {step.full && <pre className="ck-work-full">{step.full}</pre>}
          {step.tail && <pre className="ck-work-tail">{step.tail}</pre>}
          {(step.exit !== undefined || time) && (
            <span className="ck-work-meta">{[step.exit !== undefined ? WORK_COPY.exit(step.exit) : null, time].filter(Boolean).join(' \u00b7 ')}</span>
          )}
        </span>
      )}
      {live && step.log.length > 0 && (
        <span className="ck-work-log" aria-hidden="true">
          {step.log.slice(-3).map((line, i) => (
            <span key={`${i}-${line}`} className="ck-work-logline">
              {line}
            </span>
          ))}
        </span>
      )}
    </li>
  );
}

/** pending dot, spinning arc, drawn tick, X. */
function Mark({ status }: { status: WorkStepView['status'] }) {
  switch (status) {
    case 'done':
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path className="ck-work-draw" d="M3.5 8.5l3 3 6-7" pathLength={1} />
        </svg>
      );
    case 'error':
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
          <path className="ck-work-draw" d="M4.5 4.5l7 7M11.5 4.5l-7 7" pathLength={1} />
        </svg>
      );
    case 'running':
    case 'retry':
      return <span className="ck-work-arc" />;
    default:
      return <span className="ck-work-dot" />;
  }
}

/** A soft wave through the letters of the running label. */
function Wave({ text }: { text: string }) {
  return (
    <span className="ck-work-wave">
      {Array.from(text).map((ch, i) => (
        <span key={i} style={{ animationDelay: `${i * 45}ms` }}>
          {ch === ' ' ? ' ' : ch}
        </span>
      ))}
    </span>
  );
}
