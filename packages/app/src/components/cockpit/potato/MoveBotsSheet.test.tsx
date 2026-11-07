// @vitest-environment jsdom
// "Move bots" on screen: a bot file shows what is in it before anything is made and only the ticked parts come in; pasted text from
// ChatGPT or Claude becomes drafts that are reviewed and edited before saving; a backup folder lists its bots and restores them;
// at ten bots nothing more can be made.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { open } from '@tauri-apps/plugin-dialog';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeCompanionTransport } from '../../../lib/companion';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakePortableTransport, type AccountPreview, type PortablePreview } from '../../../lib/potato-portable';
import { MoveBotsSheet } from './MoveBotsSheet';
import { buttonByText } from './test-helpers';

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  vi.mocked(open).mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const PREVIEW: PortablePreview = {
  bot: { name: 'Nimbus', avatar: 'tofu', persona: 'calm', talkPersona: 'fish' },
  instructions: 'Be kind and brief.',
  skills: [{ name: 'Weekly report', description: 'how I like it', body: 'Step one' }],
  routines: [{ title: 'Morning', prompt: 'Say good morning', summary: 'Every day at 9am' }],
  memory: [{ folder: 'nodes', name: 'tea', chars: 13 }],
  notes: ['2 of 5 skills were left out (empty, repeated, too long, or more than 30).'],
  promise: 'A file adds only text. It cannot turn on tools, connect an account, or change how much this bot asks first, and routines come in paused.',
};

function setup(over: { botCount?: number; preview?: PortablePreview; account?: AccountPreview; text?: string; fail?: string } = {}) {
  const companions = fakeCompanionTransport();
  const potato = fakePotatoTransport({ bots: [] });
  const portable = fakePortableTransport({ preview: over.preview ?? PREVIEW, account: over.account, text: over.text, fail: over.fail });
  const create = vi.spyOn(companions, 'create');
  const addBot = vi.spyOn(potato, 'addBot');
  const onMade = vi.fn();
  const onNotify = vi.fn();
  const onClose = vi.fn();
  return { companions, potato, portable, create, addBot, onMade, onNotify, onClose, botCount: over.botCount ?? 1 };
}

async function render(s: ReturnType<typeof setup>) {
  await act(async () => root.render(<MoveBotsSheet companions={s.companions} potato={s.potato} portable={s.portable} botCount={s.botCount} onMade={s.onMade} onNotify={s.onNotify} onClose={s.onClose} />));
}

const click = async (el: Element | null | undefined) => {
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};
const tick = async (label: string) => {
  const box = [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find((i) => i.closest('label')?.textContent?.includes(label));
  await click(box);
};
const typeInto = async (el: HTMLTextAreaElement | HTMLInputElement, value: string) => {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto, 'value')!.set!.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
};

describe('Move bots: a bot file', () => {
  it('shows what the file holds first, and makes nothing until the button is pressed', async () => {
    const s = setup();
    vi.mocked(open).mockResolvedValue('C:\\Users\\me\\nimbus.potato-bot.json');
    await render(s);
    expect(s.create).not.toHaveBeenCalled();
    await click(buttonByText(host, 'Open a bot file…'));
    const preview = host.querySelector('[data-testid="file-preview"]')!;
    for (const text of ['Be kind and brief.', 'Weekly report', 'Morning', 'Every day at 9am', 'routines come in paused', '2 of 5 skills were left out']) {
      expect(preview.textContent, text).toContain(text);
    }
    expect(s.create).not.toHaveBeenCalled();
    expect(s.addBot).not.toHaveBeenCalled();
    expect(s.portable.calls.map((c) => c.method)).toEqual(['fileInspect']);
  });

  it('makes the bot with the face and voice in the file and brings in only the ticked parts', async () => {
    const s = setup();
    vi.mocked(open).mockResolvedValue('C:\\f\\nimbus.potato-bot.json');
    await render(s);
    await click(buttonByText(host, 'Open a bot file…'));
    await tick('Routines');
    await tick('Memory notes');
    await click(buttonByText(host, 'Make this bot'));
    expect(s.create).toHaveBeenCalledTimes(1);
    expect(s.create.mock.calls[0][0]).toMatchObject({ name: 'Nimbus', avatar: 'tofu', persona: 'calm' });
    expect(s.addBot).toHaveBeenCalledTimes(1);
    const apply = s.portable.calls.find((c) => c.method === 'fileApply')!;
    expect(apply.args[1]).toBe('C:\\f\\nimbus.potato-bot.json');
    expect(apply.args[2]).toEqual({ instructions: true, skills: true, routines: false, memory: false });
    expect(s.onMade).toHaveBeenCalledTimes(1);
  });

  it('says in one sentence why a bad file was refused and makes nothing', async () => {
    const s = setup({ fail: 'POTATO_IMPORT_INVALID: That is a compressed file. Unzip it first, then pick the bot file inside. Nothing was imported.' });
    vi.mocked(open).mockResolvedValue('C:\\f\\bomb.zip');
    await render(s);
    await click(buttonByText(host, 'Open a bot file…'));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('compressed file');
    expect(s.create).not.toHaveBeenCalled();
    expect(host.querySelector('[data-testid="file-preview"]')).toBeNull();
  });

  it('cannot make an eleventh bot', async () => {
    const s = setup({ botCount: 10 });
    vi.mocked(open).mockResolvedValue('C:\\f\\nimbus.potato-bot.json');
    await render(s);
    await click(buttonByText(host, 'Open a bot file…'));
    expect(host.textContent).toContain('You have ten bots');
    expect((buttonByText(host, 'Make this bot') as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('Move bots: ChatGPT or Claude', () => {
  const PASTED = 'Instructions:\nAnswer in short sentences.\n\nMemories:\n- Likes oolong tea\n- Lives in Taipei\n';

  it('turns pasted text into drafts, lets the person edit and untick, and saves only that', async () => {
    const s = setup();
    await render(s);
    await click([...host.querySelectorAll('button')].find((b) => b.textContent === 'ChatGPT or Claude'));
    expect(host.textContent).toContain('What to copy, and from where');
    expect(host.textContent).toContain('Custom instructions');
    await typeInto(host.querySelector('textarea')!, PASTED);
    await click(buttonByText(host, 'Read it'));
    const drafts = host.querySelector('[data-testid="draft-preview"]')!;
    expect(drafts.textContent).toContain('Likes oolong tea');
    expect(s.create).not.toHaveBeenCalled();
    await typeInto(drafts.querySelector('textarea')!, 'Answer in short, warm sentences.');
    await tick('Lives in Taipei');
    await click(buttonByText(host, 'Make this bot'));
    const save = s.portable.calls.find((c) => c.method === 'saveDrafts')!;
    expect(save.args[1]).toBe('Answer in short, warm sentences.');
    expect(save.args[2]).toEqual(['Likes oolong tea']);
    expect(s.create).toHaveBeenCalledTimes(1);
    expect(s.onMade).toHaveBeenCalledTimes(1);
  });

  it('reads an export file the person opens, and says so when it finds nothing', async () => {
    const s = setup({ text: JSON.stringify({ user: { about_user_message: 'I teach maths.' }, memories: [{ content: 'Has a cat' }] }) });
    vi.mocked(open).mockResolvedValue('C:\\f\\export.json');
    await render(s);
    await click([...host.querySelectorAll('button')].find((b) => b.textContent === 'ChatGPT or Claude'));
    await click(buttonByText(host, 'Open an export file…'));
    expect(host.querySelector('[data-testid="draft-preview"]')?.textContent).toContain('Has a cat');
    expect((host.querySelector('[data-testid="draft-preview"] textarea') as HTMLTextAreaElement).value).toBe('I teach maths.');

    const empty = setup({ text: '{"nothing":1}' });
    await act(async () => root.render(<MoveBotsSheet companions={empty.companions} potato={empty.potato} portable={empty.portable} botCount={0} onMade={empty.onMade} onNotify={empty.onNotify} onClose={empty.onClose} />));
    await click([...host.querySelectorAll('button')].find((b) => b.textContent === 'ChatGPT or Claude'));
    await click(buttonByText(host, 'Open an export file…'));
    expect(host.textContent).toContain('Nothing that looks like instructions or memories was found');
  });
});

describe('Move bots: backups', () => {
  const ACCOUNT: AccountPreview = {
    exportedMs: 1,
    bots: [
      { id: 'nimbus', bot: { name: 'Nimbus', avatar: 'mochi', persona: 'calm', talkPersona: 'fish' }, skills: 1, routines: 1, notes: 2, messages: 3 },
      { id: 'mochi', bot: { name: 'Mochi', avatar: 'bit', persona: 'plain', talkPersona: 'potato' }, skills: 0, routines: 0, notes: 0, messages: 0 },
    ],
    notes: [],
    promise: 'This copy holds no keys, tokens or sign-ins. You add those again on this PC.',
  };

  it('backs up to the chosen folder and says it holds no keys', async () => {
    const s = setup();
    vi.mocked(open).mockResolvedValue('D:\\Backups');
    await render(s);
    await click([...host.querySelectorAll('button')].find((b) => b.textContent === 'Backup'));
    expect(host.textContent).toContain('It holds no keys, tokens or sign-ins');
    await click(buttonByText(host, 'Back up to a folder…'));
    expect(s.portable.calls.find((c) => c.method === 'accountExport')?.args).toEqual(['D:\\Backups', true]);
    expect(host.querySelector('[role="status"]')?.textContent).toContain('Backup saved in D:\\Backups');
  });

  it('lists the bots of a backup folder, then makes each one and fills it', async () => {
    const s = setup({ account: ACCOUNT });
    vi.mocked(open).mockResolvedValue('D:\\Backups\\potato-bots-1');
    await render(s);
    await click([...host.querySelectorAll('button')].find((b) => b.textContent === 'Backup'));
    await click(buttonByText(host, 'Open a backup folder…'));
    const list = host.querySelector('[data-testid="backup-preview"]')!;
    expect(list.textContent).toContain('Nimbus');
    expect(list.textContent).toContain('3 messages');
    expect(list.textContent).toContain('no keys, tokens or sign-ins');
    expect(s.create).not.toHaveBeenCalled();
    await click(buttonByText(host, 'Restore these bots'));
    expect(s.create).toHaveBeenCalledTimes(2);
    const restored = s.portable.calls.filter((c) => c.method === 'accountRestoreBot');
    expect(restored.map((c) => c.args[1])).toEqual(['nimbus', 'mochi']);
    expect(s.onMade).toHaveBeenCalledTimes(1);
  });

  it('restores only as many bots as there is room for', async () => {
    const s = setup({ account: ACCOUNT, botCount: 9 });
    vi.mocked(open).mockResolvedValue('D:\\Backups\\potato-bots-1');
    await render(s);
    await click([...host.querySelectorAll('button')].find((b) => b.textContent === 'Backup'));
    await click(buttonByText(host, 'Open a backup folder…'));
    await click(buttonByText(host, 'Restore these bots'));
    expect(s.create).toHaveBeenCalledTimes(1);
  });
});
