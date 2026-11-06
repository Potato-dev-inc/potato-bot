// @vitest-environment jsdom
// The files a job changed, in the thread: a quiet row under the job bar that opens to the list, each file with Restore; a file
// that was not kept says so and has no button; the plain note says what is not covered (terminal commands); a finished job keeps
// its list; a bot with no changed file shows nothing.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport, type PotatoJob, type PotatoJobFile, type PotatoJobFiles, type PotatoSettings } from '../../../lib/potato';
import { ThreadView, type ThreadViewProps } from './ThreadView';
import { bot, buttonByText } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  window.localStorage.clear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const NOW = new Date(2026, 10, 6, 15, 0).getTime();
const SETTINGS: PotatoSettings = { v: 1, brain: 'deepseek', defaultPersona: 'pro', dailyLimitCents: 50 };
const job = (over: Partial<PotatoJob> = {}): PotatoJob => ({ goal: 'Tidy the notes', step: 'Writing', startedMs: NOW - 90_000, spendNano: 0, state: 'running', ...over });
const file = (over: Partial<PotatoJobFile> = {}): PotatoJobFile => ({ id: 'f-1', path: '/home/pm/notes/plan.md', state: 'saved', bytes: 40, at: NOW, op: 'write', restored: false, ...over });
const list = (files: PotatoJobFile[], over: Partial<PotatoJobFiles> = {}): PotatoJobFiles => ({ jobId: 'job-0a1b2c3d', files, overflow: false, ...over });

const flush = () => act(async () => {});
function view(transport: ReturnType<typeof fakePotatoTransport>) {
  const props: ThreadViewProps = {
    bot: bot({ id: 'fish', talkPersona: 'pro' }),
    messages: [],
    settings: SETTINGS,
    avatar: 'mochi',
    agentLabel: 'Claude',
    transport,
    onOpenSettings: () => {},
    onNotify: () => {},
    onChanged: () => {},
    now: NOW,
  };
  return <ThreadView {...props} />;
}
const row = () => host.querySelector('.ck-potato-jobfiles');
const open = async () => {
  await act(async () => (host.querySelector('.ck-potato-jobfiles-head') as HTMLElement).click());
};

describe('ThreadView: the files a job changed', () => {
  it('lists each file with what happened to it, and says what is not covered', async () => {
    const transport = fakePotatoTransport({
      bots: [bot({ id: 'fish' })],
      jobs: { fish: job() },
      jobFiles: { fish: list([file(), file({ id: 'f-2', path: '/home/pm/new.txt', state: 'created', bytes: 0, op: 'write' })]) },
    });
    await act(async () => root.render(view(transport)));
    await flush();
    expect(row()?.textContent).toContain('File changes');
    expect(row()?.querySelector('.ck-potato-jobfiles-body')).toBeNull();
    await open();
    const paths = [...host.querySelectorAll('.ck-potato-jobfile-path')].map((p) => p.textContent);
    expect(paths).toEqual(['notes/plan.md', 'new.txt']);
    expect([...host.querySelectorAll('.ck-potato-jobfile')].map((r) => r.textContent)).toEqual([expect.stringContaining('Changed'), expect.stringContaining('New file')]);
    expect(host.textContent).toContain('Changes made by terminal commands are not covered.');
  });

  it('Restore goes to the app with the job and the file, then the row says Restored', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs: { fish: job() }, jobFiles: { fish: list([file()]) } });
    const restore = vi.fn(fake.restoreJobFile!);
    const onChanged = vi.fn();
    const transport = { ...fake, restoreJobFile: restore };
    await act(async () => root.render(view(transport)));
    await flush();
    await open();
    await act(async () => buttonByText(host, 'Restore')!.click());
    await flush();
    expect(restore).toHaveBeenCalledWith('fish', 'job-0a1b2c3d', 'f-1');
    expect(buttonByText(host, 'Restore')).toBeUndefined();
    expect(host.querySelector('.ck-potato-jobfile')?.textContent).toContain('Restored');
    void onChanged;
  });

  it('a file that was too big to keep is named as not saved and has no Restore button', async () => {
    const transport = fakePotatoTransport({
      bots: [bot({ id: 'fish' })],
      jobs: { fish: job() },
      jobFiles: { fish: list([file({ id: 'f-3', path: '/home/pm/big.bin', state: 'too_big', bytes: 700 * 1024 }), file({ id: 'f-4', path: '/home/pm/late.txt', state: 'over_limit' })], { overflow: true }) },
    });
    await act(async () => root.render(view(transport)));
    await flush();
    await open();
    expect(buttonByText(host, 'Restore')).toBeUndefined();
    const text = host.querySelector('.ck-potato-jobfiles-body')?.textContent ?? '';
    expect(text).toContain('Not saved: the file was over 512 KB');
    expect(text).toContain('Not saved: this job changed more than could be kept');
    expect(text).toContain('more files than the list can hold');
  });

  it('shows the app\'s plain sentence when Restore fails, and the button stays', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs: { fish: job() }, jobFiles: { fish: list([file()]) } });
    const transport = { ...fake, restoreJobFile: () => Promise.reject(new Error('SANDBOX_NOT_SET_UP: The bot\'s computer is not set up yet, so the file cannot be restored.')) };
    await act(async () => root.render(view(transport)));
    await flush();
    await open();
    await act(async () => buttonByText(host, 'Restore')!.click());
    await flush();
    expect(host.querySelector('.ck-potato-jobfiles [role="alert"]')?.textContent).toBe("The bot's computer is not set up yet, so the file cannot be restored.");
    expect(buttonByText(host, 'Restore')).toBeDefined();
  });

  it('a finished job keeps its list, and a bot whose job changed no file shows nothing', async () => {
    const done = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs: { fish: job({ state: 'done' }) }, jobFiles: { fish: list([file()]) } });
    await act(async () => root.render(view(done)));
    await flush();
    expect(host.querySelector('.ck-potato-job')).toBeNull();
    expect(row()).not.toBeNull();
    act(() => root.unmount());
    root = createRoot(host);
    const none = fakePotatoTransport({ bots: [bot({ id: 'fish' })], jobs: { fish: job() }, jobFiles: { fish: list([]) } });
    await act(async () => root.render(view(none)));
    await flush();
    expect(row()).toBeNull();
  });
});
