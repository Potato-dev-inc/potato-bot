// The pinned job bar at the top of a thread (Professional mode): the goal, the step the bot is on, how long the job has run,
// what it has spent, and Stop. Stop is the same person Stop the thread header uses. Shown only while a job runs or waits.
import type { PotatoJob } from '../../../lib/potato';
import { moneyText } from '../../../lib/potato-usage';
import { elapsedLabel } from '../../../lib/potato-work';
import '../../../styles/cockpit-potato-job.css';

export const JOB_COPY = {
  label: 'Job',
  waiting: 'Waiting for you',
  starting: 'Starting',
  stop: 'Stop',
};

export interface JobBarProps {
  job: PotatoJob;
  /** The clock: the elapsed time is measured to this. */
  now: number;
  onStop: () => void;
}

export function JobBar({ job, now, onStop }: JobBarProps) {
  const step = job.state === 'waiting' ? JOB_COPY.waiting : job.step || JOB_COPY.starting;
  return (
    <section className="ck-potato-job" aria-label={JOB_COPY.label}>
      <div className="ck-potato-job-main">
        <p className="ck-potato-job-goal" title={job.goal}>
          {job.goal}
        </p>
        <p className="ck-sub ck-potato-job-facts">
          <span>{step}</span>
          <span aria-hidden="true"> · </span>
          <span>{elapsedLabel(now - job.startedMs)}</span>
          <span aria-hidden="true"> · </span>
          <span>{moneyText(job.spendNano)}</span>
        </p>
      </div>
      <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={onStop}>
        {JOB_COPY.stop}
      </button>
    </section>
  );
}
