// The Files section of the Computer tab: browse the bot's home folder, look at a file, download it to Windows, upload a file
// from Windows. The names mirror src-tauri/src/potato/files_browser.rs and commands/potato_files.rs in camelCase. Desktop only.
// The save and open dialogs are part of the transport (so a test never opens one). `fakeFilesTransport` is the in-memory twin
// for tests (the pattern of `fakeRoutinesTransport`). Nothing here draws.
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { isTauri } from './cockpit-api';

export type FileEntryKind = 'dir' | 'file' | 'other';

export interface FileEntry {
  name: string;
  kind: FileEntryKind;
  /** A symbolic link. Opened by the same checks as any path. */
  link: boolean;
  size: number;
  modifiedMs: number;
}

export interface FilesListing {
  /** `~` or `~/docs/reports`: where the list is, from the home folder. */
  path: string;
  entries: FileEntry[];
  /** There were more than 2000 entries; the rest are not shown. */
  truncated: boolean;
}

export interface FilePreview {
  name: string;
  size: number;
  kind: 'text' | 'image' | 'other';
  text: string | null;
  truncated: boolean;
  /** A picture as a data address. */
  image: string | null;
}

export interface FilesTransport {
  list(botId: string, path: string): Promise<FilesListing>;
  preview(botId: string, path: string): Promise<FilePreview>;
  /** Asks where to save (the save dialog), then saves. Resolves with the bytes saved, or null when the person cancelled. */
  download(botId: string, path: string, name: string): Promise<number | null>;
  /** Asks which file on this PC (the open dialog), then puts it in `dir`. Resolves with where it is, or null when cancelled. */
  upload(botId: string, dir: string): Promise<string | null>;
}

const NOT_ON_DESKTOP = 'Files need the Puppet Master desktop app.';

export const tauriFilesTransport: FilesTransport = {
  list: (botId, path) => (isTauri() ? invoke<FilesListing>('potato_files_list', { botId, path }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  preview: (botId, path) => (isTauri() ? invoke<FilePreview>('potato_files_preview', { botId, path }) : Promise.reject(new Error(NOT_ON_DESKTOP))),
  download: async (botId, path, name) => {
    if (!isTauri()) throw new Error(NOT_ON_DESKTOP);
    const dest = await save({ defaultPath: name });
    if (!dest) return null;
    return invoke<number>('potato_files_download', { botId, path, dest });
  },
  upload: async (botId, dir) => {
    if (!isTauri()) throw new Error(NOT_ON_DESKTOP);
    const src = await open({ multiple: false, directory: false });
    if (!src || Array.isArray(src)) return null;
    return invoke<string>('potato_files_upload', { botId, dir, src });
  },
};

// ---- paths ---------------------------------------------------------------------------------------

export const HOME_PATH = '~';

/** The path of an entry of the folder `dir`. */
export function childPath(dir: string, name: string): string {
  return dir === HOME_PATH ? `${HOME_PATH}/${name}` : `${dir}/${name}`;
}

/** The folder one level up; home stays home. */
export function parentPath(path: string): string {
  const cut = path.lastIndexOf('/');
  return cut <= 0 ? HOME_PATH : path.slice(0, cut);
}

/** The crumbs of a path: `Home`, then one per folder, each with the path it opens. */
export function crumbs(path: string): { label: string; path: string }[] {
  const parts = path === HOME_PATH ? [] : path.replace(/^~\//, '').split('/').filter(Boolean);
  const out = [{ label: 'Home', path: HOME_PATH }];
  let at = HOME_PATH;
  for (const part of parts) {
    at = childPath(at, part);
    out.push({ label: part, path: at });
  }
  return out;
}

/** `12 B`, `2 KB`, `1.5 MB`: a file's size in the units a person expects (the sandbox's own formatter starts at 1 MB). */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`.replace('.0 KB', ' KB');
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`.replace('.0 MB', ' MB');
}

// ---- copy ----------------------------------------------------------------------------------------

export const FILES_COPY = {
  title: 'Files',
  show: 'Show files',
  hide: 'Hide files',
  limits:
    'This is the bot’s home folder on its computer. Folders that start with a dot, and everything outside home, are not shown. Files up to 10 MB can be downloaded or uploaded.',
  asleep: (name: string) => `${name}’s computer is asleep. Wake it to browse its files.`,
  loading: 'Loading…',
  empty: 'This folder is empty.',
  truncated: 'Only the first 2000 items are shown.',
  folder: 'Folder',
  link: 'link',
  up: 'Up one folder',
  refresh: 'Refresh',
  download: 'Download',
  upload: 'Upload a file here',
  uploading: 'Uploading…',
  downloaded: (name: string) => `Saved ${name} to this PC.`,
  uploaded: (where: string) => `Uploaded to ${where}.`,
  noPreview: 'No preview. Download it to open it on this PC.',
  previewCut: 'Showing the start of the file.',
  loadFailed: 'Could not load the files.',
  actionFailed: 'That did not work. Try again.',
} as const;

// ---- the fake ------------------------------------------------------------------------------------

export interface FakeFilesSeed {
  /** Folders by path (`~`, `~/docs`): their entries. */
  folders?: Record<string, FileEntry[]>;
  /** Previews by path. */
  previews?: Record<string, FilePreview>;
  /** What the open dialog picks (null: the person cancels). */
  picked?: { name: string } | null;
  /** What the save dialog does: false cancels. */
  saves?: boolean;
}

export interface FakeFilesTransport extends FilesTransport {
  readonly calls: { method: keyof FilesTransport; args: unknown[] }[];
  /** Makes the next call of this method reject with that message. */
  failNext(method: keyof FilesTransport, message: string): void;
}

/** An in-memory `FilesTransport`. A path with no folder in the seed rejects like the real one: nothing at that path. */
export function fakeFilesTransport(seed: FakeFilesSeed = {}): FakeFilesTransport {
  const calls: FakeFilesTransport['calls'] = [];
  const failures = new Map<keyof FilesTransport, string>();
  const run = <T,>(method: keyof FilesTransport, args: unknown[], work: () => T): Promise<T> => {
    calls.push({ method, args });
    const failure = failures.get(method);
    if (failure) {
      failures.delete(method);
      return Promise.reject(new Error(failure));
    }
    return Promise.resolve().then(work);
  };
  return {
    calls,
    failNext: (method, message) => void failures.set(method, message),
    list: (botId, path) =>
      run('list', [botId, path], () => {
        const entries = seed.folders?.[path];
        if (!entries) throw new Error('POTATO_FILES_NOT_FOUND: There is nothing at that path.');
        return { path, entries: entries.map((e) => ({ ...e })), truncated: false };
      }),
    preview: (botId, path) =>
      run('preview', [botId, path], () => {
        const found = seed.previews?.[path];
        if (!found) throw new Error('POTATO_FILES_NOT_FOUND: There is nothing at that path.');
        return { ...found };
      }),
    download: (botId, path, name) => run('download', [botId, path, name], () => (seed.saves === false ? null : 1234)),
    upload: (botId, dir) =>
      run('upload', [botId, dir], () => {
        if (seed.picked === null) return null;
        const name = seed.picked?.name ?? 'photo.png';
        const where = childPath(dir, name);
        const entries = seed.folders?.[dir];
        if (entries && !entries.some((e) => e.name === name)) entries.push({ name, kind: 'file', link: false, size: 3, modifiedMs: 1_700_000_000_000 });
        return where;
      }),
  };
}
