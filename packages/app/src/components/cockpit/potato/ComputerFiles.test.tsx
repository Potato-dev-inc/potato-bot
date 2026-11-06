// @vitest-environment jsdom
// The Files section of the Computer tab: closed until opened (a sleeping computer is not woken by looking at the tab), the
// bot's home folder with folders first, opening folders and going up, a text or picture preview, Download and Upload through the
// native dialogs, plain errors, and what the section says it cannot show.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakeFilesTransport, type FakeFilesSeed, type FakeFilesTransport, type FileEntry } from '../../../lib/potato-files';
import { fakeSandboxTransport, type FakeSandboxSeed } from '../../../lib/potato-sandbox';
import { fakeScreenTransport } from '../../../lib/potatoScreen';
import { ComputerTab } from './ComputerTab';
import { buttonByText } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const BOT = { id: 'fish', name: 'Fish' };
const running: FakeSandboxSeed = { status: { state: 'running', hasResetPoint: true } };
const entry = (name: string, kind: FileEntry['kind'] = 'file', over: Partial<FileEntry> = {}): FileEntry => ({ name, kind, link: false, size: 2048, modifiedMs: 1_700_000_000_000, ...over });

const SEED: FakeFilesSeed = {
  folders: {
    '~': [entry('docs', 'dir', { size: 0 }), entry('notes.txt'), entry('shortcut', 'dir', { link: true, size: 0 })],
    '~/docs': [entry('plan.md'), entry('logo.png'), entry('tool.bin')],
    '~/docs/empty': [],
  },
  previews: {
    '~/notes.txt': { name: 'notes.txt', size: 12, kind: 'text', text: 'remember milk', truncated: false, image: null },
    '~/docs/plan.md': { name: 'plan.md', size: 900_000, kind: 'text', text: '# Plan', truncated: true, image: null },
    '~/docs/logo.png': { name: 'logo.png', size: 5, kind: 'image', text: null, truncated: false, image: 'data:image/png;base64,AAAA' },
    '~/docs/tool.bin': { name: 'tool.bin', size: 5, kind: 'other', text: null, truncated: false, image: null },
  },
};

async function mount(files: FakeFilesTransport | undefined, over: { sandboxSeed?: FakeSandboxSeed; notes?: string[] } = {}) {
  const sandbox = fakeSandboxTransport(over.sandboxSeed ?? running);
  const notes = over.notes ?? [];
  await act(async () =>
    root.render(<ComputerTab bot={BOT} screen={fakeScreenTransport()} sandbox={sandbox} files={files} through={false} onNotify={(t) => notes.push(t)} />),
  );
  await act(async () => {});
  return { sandbox, notes };
}
const settle = () => act(async () => {});
const rowButton = (label: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('.ck-potato-files-row')].find((r) => r.querySelector('.ck-potato-files-name')?.textContent === label);
const click = async (label: string) => {
  const button = rowButton(label) ?? buttonByText(host, label) ?? [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.getAttribute('aria-label') === label);
  expect(button, `a button named ${label}`).toBeTruthy();
  await act(async () => button!.click());
  await settle();
};
const section = () => host.querySelector('.ck-potato-files');
const rows = () => [...host.querySelectorAll('.ck-potato-files-row .ck-potato-files-name')].map((n) => n.textContent);
const crumbs = () => [...host.querySelectorAll('.ck-potato-files-crumbs li')].map((n) => n.textContent);
const problem = () => host.querySelector('.ck-potato-files-problem')?.textContent;
const listCalls = (files: FakeFilesTransport) => files.calls.filter((c) => c.method === 'list').map((c) => c.args);

describe('the section', () => {
  it('is not drawn without a files transport', async () => {
    await mount(undefined);
    expect(section()).toBeNull();
  });

  it('starts closed and asks the computer for nothing until it is opened', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    expect(section()).toBeTruthy();
    expect(files.calls).toEqual([]);
    expect(host.querySelector('.ck-potato-files-limits')).toBeNull();
    await click('Show files');
    expect(listCalls(files)).toEqual([['fish', '~']]);
    expect(buttonByText(host, 'Hide files')).toBeTruthy();
    await click('Hide files');
    expect(host.querySelector('.ck-potato-files-body')).toBeNull();
  });

  it('says plainly what it cannot show', async () => {
    await mount(fakeFilesTransport(SEED));
    await click('Show files');
    const note = host.querySelector('.ck-potato-files-limits')?.textContent ?? '';
    expect(note).toContain('home folder');
    expect(note).toContain('start with a dot');
    expect(note).toContain('outside home');
    expect(note).toContain('10 MB');
  });

  it('lists the home folder, marks folders and links', async () => {
    await mount(fakeFilesTransport(SEED));
    await click('Show files');
    expect(rows()).toEqual(['docs/', 'notes.txt', 'shortcut/']);
    expect(crumbs()).toEqual(['Home']);
    const meta = [...host.querySelectorAll('.ck-potato-files-meta')].map((n) => n.textContent);
    expect(meta).toEqual(['Folder', '2 KB', 'link · Folder']);
    expect(host.querySelector('.ck-potato-files-up')).toBeNull();
  });

  it('is asleep with the computer: it says so, loads nothing and offers Wake', async () => {
    const files = fakeFilesTransport(SEED);
    const { sandbox } = await mount(files, { sandboxSeed: { status: { state: 'stopped', hasResetPoint: true } } });
    await click('Show files');
    expect(host.querySelector('.ck-potato-files-asleep')?.textContent).toContain('Fish’s computer is asleep');
    expect(files.calls).toEqual([]);
    const woke = sandbox.calls.length;
    const wake = [...host.querySelectorAll('.ck-potato-files-asleep button')].find((b) => b.textContent === 'Wake') as HTMLButtonElement;
    await act(async () => wake.click());
    expect(sandbox.calls.slice(woke).map((c) => c.method)).toContain('start');
  });

  it('loads the folder when the computer wakes while the section is open', async () => {
    const files = fakeFilesTransport(SEED);
    const sandbox = fakeSandboxTransport({ status: { state: 'stopped', hasResetPoint: true } });
    const render = () => act(async () => root.render(<ComputerTab bot={BOT} screen={fakeScreenTransport()} sandbox={sandbox} files={files} through={false} onNotify={() => {}} />));
    await render();
    await click('Show files');
    expect(files.calls).toEqual([]);
    sandbox.view.state = 'running';
    await act(async () => {
      await sandbox.status();
    });
    await render();
    await settle();
  });
});

describe('moving around', () => {
  it('opens a folder, shows where you are, and goes up', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    await click('Show files');
    await click('docs/');
    expect(listCalls(files).at(-1)).toEqual(['fish', '~/docs']);
    expect(rows()).toEqual(['plan.md', 'logo.png', 'tool.bin']);
    expect(crumbs()).toEqual(['Home', 'docs']);
    await click('Up one folder');
    expect(rows()).toEqual(['docs/', 'notes.txt', 'shortcut/']);
    await click('docs/');
    await click('Home');
    expect(crumbs()).toEqual(['Home']);
  });

  it('never sends a path with .. in it, whatever is clicked', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    await click('Show files');
    await click('docs/');
    await click('Up one folder');
    await click('Up one folder').catch(() => undefined);
    await click('Refresh');
    for (const call of files.calls) expect(String(call.args[1])).not.toContain('..');
  });

  it('says an empty folder is empty', async () => {
    const files = fakeFilesTransport({ folders: { '~': [entry('empty', 'dir')], '~/empty': [] } });
    await mount(files);
    await click('Show files');
    await click('empty/');
    expect(host.querySelector('.ck-potato-files-body')?.textContent).toContain('This folder is empty.');
  });

  it('shows the computer’s plain sentence when a folder cannot be opened, without its code', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    await click('Show files');
    files.failNext('list', 'POTATO_FILES_OUTSIDE: That is outside the bot’s home folder (or a hidden folder of it), so it cannot be opened here.');
    await click('shortcut/');
    expect(problem()).toBe('That is outside the bot’s home folder (or a hidden folder of it), so it cannot be opened here.');
    expect(host.textContent).not.toContain('POTATO_FILES_OUTSIDE');
  });
});

describe('looking at a file', () => {
  it('shows the start of a text file and says when it was cut', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    await click('Show files');
    await click('notes.txt');
    expect(files.calls.at(-1)).toMatchObject({ method: 'preview', args: ['fish', '~/notes.txt'] });
    expect(host.querySelector('.ck-potato-files-text')?.textContent).toBe('remember milk');
    expect(host.querySelector('.ck-potato-files-preview')?.textContent).not.toContain('Showing the start');
    await click('docs/');
    await click('plan.md');
    expect(host.querySelector('.ck-potato-files-preview')?.textContent).toContain('Showing the start of the file.');
  });

  it('draws a picture, and says there is no preview for a file it cannot show', async () => {
    await mount(fakeFilesTransport(SEED));
    await click('Show files');
    await click('docs/');
    await click('logo.png');
    expect(host.querySelector<HTMLImageElement>('.ck-potato-files-image')?.getAttribute('src')).toBe('data:image/png;base64,AAAA');
    await click('tool.bin');
    expect(host.querySelector('.ck-potato-files-image')).toBeNull();
    expect(host.querySelector('.ck-potato-files-preview')?.textContent).toContain('No preview. Download it to open it on this PC.');
  });

  it('a slow answer for an earlier file does not replace the one you chose last', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    await click('Show files');
    await click('docs/');
    // two quick choices: the second wins
    const a = rowButton('plan.md');
    const b = rowButton('logo.png');
    await act(async () => {
      a!.click();
      b!.click();
    });
    await settle();
    expect(host.querySelector('.ck-potato-files-preview')?.getAttribute('aria-label')).toBe('logo.png');
    expect(host.querySelector('.ck-potato-files-image')).toBeTruthy();
  });
});

describe('download and upload', () => {
  it('Download saves the chosen file through the save dialog and says so', async () => {
    const files = fakeFilesTransport(SEED);
    const { notes } = await mount(files);
    await click('Show files');
    await click('docs/');
    await click('plan.md');
    await click('Download');
    expect(files.calls.at(-1)).toMatchObject({ method: 'download', args: ['fish', '~/docs/plan.md', 'plan.md'] });
    expect(notes).toEqual(['Saved plan.md to this PC.']);
  });

  it('a cancelled save says nothing and shows no error', async () => {
    const files = fakeFilesTransport({ ...SEED, saves: false });
    const { notes } = await mount(files);
    await click('Show files');
    await click('notes.txt');
    await click('Download');
    expect(notes).toEqual([]);
    expect(problem()).toBeUndefined();
  });

  it('Upload puts a file in the folder you are in and shows it', async () => {
    const files = fakeFilesTransport({ ...SEED, picked: { name: 'photo.png' } });
    const { notes } = await mount(files);
    await click('Show files');
    await click('docs/');
    await click('Upload a file here');
    expect(files.calls.filter((c) => c.method === 'upload').map((c) => c.args)).toEqual([['fish', '~/docs']]);
    expect(notes).toEqual(['Uploaded to ~/docs/photo.png.']);
    expect(rows()).toContain('photo.png');
  });

  it('a cancelled upload changes nothing', async () => {
    const files = fakeFilesTransport({ ...SEED, picked: null });
    const { notes } = await mount(files);
    await click('Show files');
    await click('Upload a file here');
    expect(notes).toEqual([]);
    expect(rows()).toEqual(['docs/', 'notes.txt', 'shortcut/']);
  });

  it('shows the plain sentence when an upload or a download is refused', async () => {
    const files = fakeFilesTransport(SEED);
    await mount(files);
    await click('Show files');
    files.failNext('upload', 'POTATO_FILES_EXISTS: A file with that name is already in that folder. Rename the file on this PC and try again.');
    await click('Upload a file here');
    expect(problem()).toBe('A file with that name is already in that folder. Rename the file on this PC and try again.');
    await click('notes.txt');
    files.failNext('download', 'POTATO_FILES_TOO_BIG: That file is bigger than 10 MB. Ask the bot to zip it or split it into parts.');
    await click('Download');
    expect(problem()).toBe('That file is bigger than 10 MB. Ask the bot to zip it or split it into parts.');
  });
});
