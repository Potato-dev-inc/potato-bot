import { formatSize, type SpaceFile } from '../../lib/spaces';

export { formatSize };

export interface FileCardProps {
  file: SpaceFile;
  error: string | null;
  savedAs: string | null;
  onGet: () => void;
  onSave: () => void;
}

/** A file in a message. It only ever offers Get file and Save to Downloads: nothing opens or previews a file. */
export function FileCard({ file, error, savedAs, onGet, onSave }: FileCardProps) {
  return (
    <div className="ck-spaces-file">
      <p className="ck-spaces-file-name">{file.name} · {formatSize(file.size)}</p>
      {file.state === 'getting' ? (
        <p className="ck-spaces-hint" role="status">Getting file… {file.percent}%</p>
      ) : file.have ? (
        <button type="button" className="ck-spaces-btn" onClick={onSave}>Save to Downloads</button>
      ) : (
        <button type="button" className="ck-spaces-btn" onClick={onGet}>Get file</button>
      )}
      {savedAs && <p className="ck-spaces-hint" role="status">Saved to Downloads as {savedAs}.</p>}
      {error && <p className="ck-spaces-hint is-error" role="alert">{error}</p>}
      <p className="ck-spaces-hint">Files from other people can be harmful. Only open files you expect.</p>
    </div>
  );
}
