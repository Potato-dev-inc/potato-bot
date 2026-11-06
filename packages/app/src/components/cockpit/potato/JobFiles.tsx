// The files a bot's job changed with its file tools, in the thread under the job bar: each with Restore, which puts the old
// content back (or removes a file the job made). It says plainly what it does not cover: changes made by terminal commands, and
// files that were too big to keep. Shown as long as the job has changed a file, also after the job ended.
import { useState } from 'react';
import { plainError, type PotatoJobFile, type PotatoJobFiles } from '../../../lib/potato';
import { formatSize } from '../../../lib/potato-sandbox';
import '../../../styles/cockpit-potato-job.css';

export const JOBFILES_COPY = {
  title: 'File changes',
  cover: 'Covers files the bot wrote or edited with its file tools. Changes made by terminal commands are not covered.',
  restore: 'Restore',
  restored: 'Restored',
  created: 'New file',
  edited: 'Changed',
  tooBig: (bytes: number) => `Not saved: the file was over 512 KB (it was ${formatSize(bytes)})`,
  overLimit: 'Not saved: this job changed more than could be kept',
  overflow: 'This job changed more files than the list can hold. The rest are not listed.',
  failed: 'Could not restore the file.',
  show: 'Show',
  hide: 'Hide',
} as const;

const HOME = '/home/pm/';
/** The path as the person knows it: relative to the bot's home. */
export const shownPath = (path: string) => (path.startsWith(HOME) ? path.slice(HOME.length) : path);

export interface JobFilesProps {
  botId: string;
  list: PotatoJobFiles;
  restore: (jobId: string, fileId: string) => Promise<unknown>;
  onRestored: () => void;
}

function note(file: PotatoJobFile): string | null {
  if (file.state === 'too_big') return JOBFILES_COPY.tooBig(file.bytes);
  if (file.state === 'over_limit') return JOBFILES_COPY.overLimit;
  return null;
}

export function JobFiles({ list, restore, onRestored }: JobFilesProps) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<Set<string>>(new Set());

  const run = async (file: PotatoJobFile) => {
    setBusy(file.id);
    setError(null);
    try {
      await restore(list.jobId, file.id);
      setDone((was) => new Set(was).add(file.id));
      onRestored();
    } catch (e) {
      setError(plainError(e, JOBFILES_COPY.failed));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="ck-potato-jobfiles" aria-label={JOBFILES_COPY.title}>
      <button type="button" className="ck-potato-jobfiles-head" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <span className="ck-potato-jobfiles-title">{JOBFILES_COPY.title}</span>
        <span className="ck-sub">{list.files.length}</span>
        <span className="ck-sub ck-potato-jobfiles-toggle">{open ? JOBFILES_COPY.hide : JOBFILES_COPY.show}</span>
      </button>
      {open && (
        <div className="ck-potato-jobfiles-body">
          <ul className="ck-potato-jobfiles-list">
            {list.files.map((file) => {
              const why = note(file);
              const isDone = file.restored || done.has(file.id);
              return (
                <li key={file.id} className="ck-potato-jobfile">
                  <span className="ck-potato-jobfile-main">
                    <span className="ck-potato-jobfile-path" title={file.path}>
                      {shownPath(file.path)}
                    </span>
                    <span className="ck-sub">{why ?? (file.state === 'created' ? JOBFILES_COPY.created : JOBFILES_COPY.edited)}</span>
                  </span>
                  {isDone ? (
                    <span className="ck-sub">{JOBFILES_COPY.restored}</span>
                  ) : (
                    !why && (
                      <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" disabled={busy !== null} aria-label={`${JOBFILES_COPY.restore} ${shownPath(file.path)}`} onClick={() => void run(file)}>
                        {JOBFILES_COPY.restore}
                      </button>
                    )
                  )}
                </li>
              );
            })}
          </ul>
          {list.overflow && <p className="ck-sub">{JOBFILES_COPY.overflow}</p>}
          {error && (
            <p className="ck-sub" role="alert">
              {error}
            </p>
          )}
          <p className="ck-sub ck-potato-jobfiles-cover">{JOBFILES_COPY.cover}</p>
        </div>
      )}
    </section>
  );
}
