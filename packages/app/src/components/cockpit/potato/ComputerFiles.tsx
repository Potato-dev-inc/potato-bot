// The Files section of the Computer tab: browse the bot's home folder on its computer, look at a file, download it to Windows,
// upload a file from Windows. It is closed until the person opens it (so a sleeping computer is not woken just by looking at
// the tab), and says plainly what it cannot show. Every call is a host call (potato_files_*); the checks that keep it inside
// the bot's home are made in the computer, not here.
import { useCallback, useEffect, useRef, useState } from 'react';
import { plainError } from '../../../lib/potato';
import { childPath, crumbs, FILES_COPY, fileSize, HOME_PATH, parentPath, type FilePreview, type FilesListing, type FilesTransport } from '../../../lib/potato-files';
import '../../../styles/cockpit-potato-files.css';

export interface ComputerFilesProps {
  bot: { id: string; name: string };
  transport: FilesTransport;
  /** The computer is awake. While it is asleep the section says so and loads nothing. */
  awake: boolean;
  onWake?: () => void;
  onNotify?: (text: string) => void;
}

export function ComputerFiles({ bot, transport, awake, onWake, onNotify }: ComputerFilesProps) {
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState(HOME_PATH);
  const [listing, setListing] = useState<FilesListing | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [preview, setPreview] = useState<FilePreview | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState<'upload' | 'download' | null>(null);
  const latest = useRef(0);
  const looking = useRef(0);

  const load = useCallback(
    (where: string) => {
      const mine = ++latest.current;
      looking.current++;
      setListing(null);
      setProblem(null);
      setChosen(null);
      setPreview(null);
      transport.list(bot.id, where).then(
        (found) => {
          if (latest.current !== mine) return;
          setListing(found);
          setPath(found.path);
        },
        (error: unknown) => {
          if (latest.current === mine) setProblem(plainError(error, FILES_COPY.loadFailed));
        },
      );
    },
    [transport, bot.id],
  );

  // Opening the section (or the computer waking while it is open) loads the folder the person was in.
  useEffect(() => {
    if (open && awake) load(path);
    // `path` is read on purpose only when the section opens or the computer wakes; moving around calls `load` itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, awake, load]);

  const choose = (name: string) => {
    const where = childPath(path, name);
    const mine = ++looking.current;
    setChosen(name);
    setPreview(null);
    setProblem(null);
    transport.preview(bot.id, where).then(
      (found) => {
        if (looking.current === mine) setPreview(found);
      },
      (error: unknown) => {
        if (looking.current === mine) setProblem(plainError(error, FILES_COPY.loadFailed));
      },
    );
  };

  const download = async () => {
    if (!chosen || busy) return;
    setBusy('download');
    setProblem(null);
    try {
      const saved = await transport.download(bot.id, childPath(path, chosen), chosen);
      if (saved !== null) onNotify?.(FILES_COPY.downloaded(chosen));
    } catch (error) {
      setProblem(plainError(error, FILES_COPY.actionFailed));
    } finally {
      setBusy(null);
    }
  };

  const upload = async () => {
    if (busy) return;
    setBusy('upload');
    setProblem(null);
    try {
      const where = await transport.upload(bot.id, path);
      if (where !== null) {
        onNotify?.(FILES_COPY.uploaded(where));
        load(path);
      }
    } catch (error) {
      setProblem(plainError(error, FILES_COPY.actionFailed));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="ck-potato-files" aria-label={FILES_COPY.title}>
      <header className="ck-potato-files-head">
        <h3 className="ck-potato-files-title">{FILES_COPY.title}</h3>
        <button type="button" className="ck-btn ck-btn--secondary" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
          {open ? FILES_COPY.hide : FILES_COPY.show}
        </button>
      </header>
      {open && (
        <div className="ck-potato-files-body">
          <p className="ck-sub ck-potato-files-limits">{FILES_COPY.limits}</p>
          {!awake ? (
            <p className="ck-sub ck-potato-files-asleep" role="status">
              {FILES_COPY.asleep(bot.name)}{' '}
              {onWake && (
                <button type="button" className="ck-potato-link" onClick={onWake}>
                  Wake
                </button>
              )}
            </p>
          ) : (
            <>
              <nav className="ck-potato-files-bar" aria-label="Folder">
                {path !== HOME_PATH && (
                  <button type="button" className="ck-potato-files-up" aria-label={FILES_COPY.up} title={FILES_COPY.up} onClick={() => load(parentPath(path))}>
                    ↑
                  </button>
                )}
                <ol className="ck-potato-files-crumbs">
                  {crumbs(path).map((crumb, index, all) => (
                    <li key={crumb.path}>
                      {index === all.length - 1 ? (
                        <span aria-current="location">{crumb.label}</span>
                      ) : (
                        <button type="button" className="ck-potato-link" onClick={() => load(crumb.path)}>
                          {crumb.label}
                        </button>
                      )}
                    </li>
                  ))}
                </ol>
                <span className="ck-potato-files-gap" />
                <button type="button" className="ck-btn ck-btn--secondary" onClick={() => load(path)}>
                  {FILES_COPY.refresh}
                </button>
                <button type="button" className="ck-btn ck-btn--secondary" disabled={busy !== null || listing === null} onClick={() => void upload()}>
                  {busy === 'upload' ? FILES_COPY.uploading : FILES_COPY.upload}
                </button>
              </nav>
              {problem && (
                <p className="ck-potato-files-problem" role="alert">
                  {problem}
                </p>
              )}
              {!listing && !problem && <p className="ck-sub">{FILES_COPY.loading}</p>}
              {listing && listing.entries.length === 0 && <p className="ck-sub">{FILES_COPY.empty}</p>}
              {listing && listing.entries.length > 0 && (
                <ul className="ck-potato-files-list">
                  {listing.entries.map((entry) => (
                    <li key={entry.name}>
                      <button
                        type="button"
                        className="ck-potato-files-row"
                        aria-pressed={chosen === entry.name}
                        onClick={() => (entry.kind === 'dir' ? load(childPath(path, entry.name)) : choose(entry.name))}
                      >
                        <span className="ck-potato-files-name">
                          {entry.name}
                          {entry.kind === 'dir' ? '/' : ''}
                        </span>
                        <span className="ck-sub ck-potato-files-meta">
                          {entry.link ? `${FILES_COPY.link} · ` : ''}
                          {entry.kind === 'dir' ? FILES_COPY.folder : fileSize(entry.size)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              {listing?.truncated && <p className="ck-sub">{FILES_COPY.truncated}</p>}
              {chosen && (
                <div className="ck-potato-files-preview" role="group" aria-label={chosen}>
                  <div className="ck-potato-files-preview-head">
                    <span className="ck-potato-files-name" title={chosen}>
                      {chosen}
                    </span>
                    {preview && <span className="ck-sub">{fileSize(preview.size)}</span>}
                    <span className="ck-potato-files-gap" />
                    <button type="button" className="ck-btn ck-btn--secondary" disabled={busy !== null} onClick={() => void download()}>
                      {FILES_COPY.download}
                    </button>
                  </div>
                  {!preview && !problem && <p className="ck-sub">{FILES_COPY.loading}</p>}
                  {preview?.kind === 'image' && preview.image && <img className="ck-potato-files-image" src={preview.image} alt={chosen} />}
                  {preview?.kind === 'text' && preview.text !== null && (
                    <>
                      <pre className="ck-potato-files-text">{preview.text}</pre>
                      {preview.truncated && <p className="ck-sub">{FILES_COPY.previewCut}</p>}
                    </>
                  )}
                  {preview?.kind === 'other' && <p className="ck-sub">{FILES_COPY.noPreview}</p>}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
