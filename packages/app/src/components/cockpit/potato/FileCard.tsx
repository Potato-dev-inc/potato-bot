// The card a bot's `send_file` puts in the chat: the file's name and size, Open (a picture, or the text of the file) and
// Save (the Artifacts page's save dialog and download). An image shows as a picture at once. HTML is never drawn here:
// it is offered as a file to save. Nothing is run.
import { useEffect, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { ARTIFACT_COPY, type ArtifactPreview, type ArtifactTransport } from '../../../lib/potato-artifacts';
import { plainError, type PotatoFileCard } from '../../../lib/potato';
import { formatSize } from '../../../lib/potato-sandbox';

export interface FileCardProps {
  botId: string;
  file: PotatoFileCard;
  files: ArtifactTransport;
  /** Tells the person what happened after a save. Default: nothing. */
  onNotify?: (text: string) => void;
}

export function FileCard({ botId, file, files, onNotify }: FileCardProps) {
  const picture = file.kind === 'image';
  const [open, setOpen] = useState(picture);
  const [preview, setPreview] = useState<ArtifactPreview | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  // The picture is drawn at once; a text file is read when Open is pressed.
  useEffect(() => {
    if (!open || preview) return;
    let alive = true;
    files.open(botId, file.id).then(
      (found) => {
        if (alive) setPreview(found);
      },
      (error) => {
        if (alive) setProblem(plainError(error, 'Could not open the file.'));
      },
    );
    return () => {
      alive = false;
    };
  }, [open, preview, files, botId, file.id]);

  const saveFile = async () => {
    try {
      const dest = await save({ defaultPath: file.name });
      if (!dest) return;
      await files.download(botId, file.id, dest);
      onNotify?.(ARTIFACT_COPY.downloaded(file.name));
    } catch (error) {
      setProblem(plainError(error, 'Could not save the file.'));
    }
  };

  const showPicture = picture && !!preview?.image;
  const showText = !picture && file.kind !== 'html' && !!preview && preview.text !== null;

  return (
    <div className="ck-potato-file" role="group" aria-label={file.name}>
      <div className="ck-potato-file-head">
        <span className="ck-potato-file-name" title={file.name}>
          {file.name}
        </span>
        <span className="ck-potato-file-size">{formatSize(file.size)}</span>
      </div>
      <div className="ck-potato-file-actions">
        <button type="button" className="ck-potato-action" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
          Open
        </button>
        <button type="button" className="ck-potato-action" onClick={() => void saveFile()}>
          Save
        </button>
      </div>
      {open && (
        <div className="ck-potato-file-preview">
          {!preview && !problem && <p className="ck-sub">Loading...</p>}
          {showPicture && preview?.image && <img className="ck-potato-file-image" src={preview.image} alt={file.name} />}
          {showText && preview?.text != null && (
            <pre className="ck-potato-file-text">
              {preview.text}
              {preview.truncated ? '\n...' : ''}
            </pre>
          )}
          {preview && !showPicture && !showText && (
            <p className="ck-sub">
              {file.kind === 'html' ? 'HTML is not shown here. Save it and open it in a browser.' : 'No preview. Save it to open it on your computer.'}
            </p>
          )}
        </div>
      )}
      {problem && (
        <p className="ck-artifact-note" role="alert">
          {problem}
        </p>
      )}
    </div>
  );
}
