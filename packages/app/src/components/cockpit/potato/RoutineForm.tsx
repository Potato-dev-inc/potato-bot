// "New routine": a small form in the Routines tab. A name, what to do, and when (every hour, every day at a time,
// weekdays at a time, every few minutes, or once at a date and time). Saving hands the routine to the Rust side,
// which checks it again and gives the answer.
import { useEffect, useId, useState, type FormEvent } from 'react';
import { plainError } from '../../../lib/potato';
import {
  buildRoutineInput,
  cronRunWords,
  DEFAULT_ROUTINE_FORM,
  ROUTINE_COPY,
  type RoutineFormValues,
  type RoutineFreq,
  watchIsAddress,
  type RoutineView,
  type RoutinesTransport,
} from '../../../lib/potato-routines';

export interface RoutineFormProps {
  botId: string;
  transport: RoutinesTransport;
  onSaved: (routine: RoutineView) => void;
  onCancel: () => void;
}

const FREQ_OPTIONS: readonly { value: RoutineFreq; label: string }[] = [
  { value: 'hourly', label: 'Every hour' },
  { value: 'daily', label: 'Every day' },
  { value: 'weekdays', label: 'Weekdays' },
  { value: 'every', label: 'Every few minutes' },
  { value: 'once', label: 'Once' },
  { value: 'cron', label: 'Cron (advanced)' },
];

export function RoutineForm({ botId, transport, onSaved, onCancel }: RoutineFormProps) {
  const [values, setValues] = useState<RoutineFormValues>(DEFAULT_ROUTINE_FORM);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // The coming runs of the typed cron, worked out by the Rust side (it owns the cron reader), or why it is refused.
  const [cronRuns, setCronRuns] = useState<number[]>([]);
  const [cronError, setCronError] = useState<string | null>(null);
  const cronText = values.freq === 'cron' ? values.cron.trim() : '';
  useEffect(() => {
    setCronRuns([]);
    setCronError(null);
    if (!cronText) return;
    let stale = false;
    transport.cronPreview(cronText).then(
      (runs) => {
        if (!stale) setCronRuns(runs);
      },
      (e: unknown) => {
        if (!stale) setCronError(plainError(e, ROUTINE_COPY.saveFailed));
      },
    );
    return () => {
      stale = true;
    };
  }, [cronText, transport]);
  const ids = { cron: useId(), title: useId(), prompt: useId(), freq: useId(), time: useId(), minutes: useId(), at: useId(), watch: useId(), watchCheck: useId(), watchContains: useId() };

  const set = <K extends keyof RoutineFormValues>(key: K, value: RoutineFormValues[K]) =>
    setValues((current) => ({ ...current, [key]: value }));

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (saving) return;
    const built = buildRoutineInput(values);
    if (typeof built === 'string') {
      setError(built);
      return;
    }
    setError(null);
    setSaving(true);
    transport
      .create(botId, built)
      .then(
        (routine) => onSaved(routine),
        (e: unknown) => {
          setError(plainError(e, ROUTINE_COPY.saveFailed));
          setSaving(false);
        },
      );
  };

  return (
    <form className="ck-potato-routine-form" onSubmit={submit} aria-label="New routine" noValidate>
      <label className="ck-potato-routine-field" htmlFor={ids.title}>
        <span>Name</span>
        <input id={ids.title} type="text" value={values.title} placeholder="Morning sweep" maxLength={80} onChange={(e) => set('title', e.target.value)} />
      </label>
      <label className="ck-potato-routine-field" htmlFor={ids.prompt}>
        <span>{values.watchOn ? ROUTINE_COPY.watchPromptLabel : 'What to do'}</span>
        <textarea id={ids.prompt} rows={3} value={values.prompt} placeholder="Check my inbox and list what needs a reply." onChange={(e) => set('prompt', e.target.value)} />
      </label>
      <label className="ck-potato-routine-field" htmlFor={ids.freq}>
        <span>How often</span>
        <select id={ids.freq} value={values.freq} onChange={(e) => set('freq', e.target.value as RoutineFreq)}>
          {FREQ_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      {values.freq === 'daily' || values.freq === 'weekdays' ? (
        <label className="ck-potato-routine-field" htmlFor={ids.time}>
          <span>Time</span>
          <input id={ids.time} type="time" value={values.time} onChange={(e) => set('time', e.target.value)} />
        </label>
      ) : null}
      {values.freq === 'every' ? (
        <label className="ck-potato-routine-field" htmlFor={ids.minutes}>
          <span>Every how many minutes (15 or more)</span>
          <input id={ids.minutes} type="number" min={15} max={1440} step={1} value={values.minutes} onChange={(e) => set('minutes', Number(e.target.value))} />
        </label>
      ) : null}
      {values.freq === 'once' ? (
        <label className="ck-potato-routine-field" htmlFor={ids.at}>
          <span>Date and time</span>
          <input id={ids.at} type="datetime-local" value={values.at} onChange={(e) => set('at', e.target.value)} />
        </label>
      ) : null}
      {values.freq === 'cron' ? (
        <div className="ck-potato-routine-field">
          <label htmlFor={ids.cron}>
            <span>{ROUTINE_COPY.cronLabel}</span>
          </label>
          <input id={ids.cron} aria-label={ROUTINE_COPY.cronLabel} type="text" spellCheck={false} autoComplete="off" value={values.cron} placeholder="0 9 * * mon-fri" onChange={(e) => set('cron', e.target.value)} />
          <span className="ck-sub">{ROUTINE_COPY.cronHint}</span>
          {cronError ? (
            <p className="ck-potato-routine-error ck-potato-cron-error">{cronError}</p>
          ) : cronRuns.length > 0 ? (
            <div className="ck-potato-cron-preview">
              <span className="ck-sub">{ROUTINE_COPY.cronNext}</span>
              <ul>
                {cronRuns.map((at) => (
                  <li key={at}>{cronRunWords(at)}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}
      <div className="ck-potato-routine-field">
        <label className="ck-potato-routine-check" htmlFor={ids.watch}>
          <input id={ids.watch} type="checkbox" checked={values.watchOn} onChange={(e) => set('watchOn', e.target.checked)} />
          <span>{ROUTINE_COPY.watchLabel}</span>
        </label>
        <span className="ck-sub">{ROUTINE_COPY.watchHint}</span>
      </div>
      {values.watchOn ? (
        <>
          <label className="ck-potato-routine-field" htmlFor={ids.watchCheck}>
            <span>{ROUTINE_COPY.watchCheckLabel}</span>
            <input
              id={ids.watchCheck}
              type="text"
              spellCheck={false}
              autoComplete="off"
              maxLength={1000}
              value={values.watchCheck}
              placeholder={ROUTINE_COPY.watchPlaceholder}
              onChange={(e) => set('watchCheck', e.target.value)}
            />
            <span className="ck-sub">{ROUTINE_COPY.watchCheckHint}</span>
          </label>
          {watchIsAddress(values.watchCheck) ? (
            <label className="ck-potato-routine-field" htmlFor={ids.watchContains}>
              <span>{ROUTINE_COPY.watchContainsLabel}</span>
              <input id={ids.watchContains} type="text" maxLength={200} value={values.watchContains} onChange={(e) => set('watchContains', e.target.value)} />
            </label>
          ) : null}
        </>
      ) : null}
      {error ? (
        <p role="alert" className="ck-potato-routine-error">
          {error}
        </p>
      ) : null}
      <div className="ck-potato-routine-actions">
        <button type="button" className="ck-potato-routine-btn" onClick={onCancel}>
          {ROUTINE_COPY.cancel}
        </button>
        <button type="submit" className="ck-potato-routine-btn ck-potato-routine-btn-primary" disabled={saving}>
          {saving ? ROUTINE_COPY.saving : ROUTINE_COPY.save}
        </button>
      </div>
    </form>
  );
}
