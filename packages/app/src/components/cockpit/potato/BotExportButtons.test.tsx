// @vitest-environment jsdom
// The Advanced tab's Bot card: Export bot asks where to save and says what is never in the file; Duplicate makes "<name> copy".
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { save } from '@tauri-apps/plugin-dialog';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeCompanionTransport } from '../../../lib/companion';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakePortableTransport } from '../../../lib/potato-portable';
import { BotExportButtons } from './BotExportButtons';
import { buttonByText } from './test-helpers';

vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn(), open: vi.fn() }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  vi.mocked(save).mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const click = async (el: Element | null | undefined) => {
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};

describe('BotExportButtons', () => {
  it('exports to the chosen file, with memory only when it is ticked, and says what never travels', async () => {
    const portable = fakePortableTransport();
    vi.mocked(save).mockResolvedValue('C:\\Users\\me\\nimbus.potato-bot.json');
    await act(async () => root.render(<BotExportButtons bot={{ id: 'nimbus', name: 'Nimbus' }} portable={portable} />));
    expect(host.textContent).toContain('never holds keys, tokens, connector sign-ins or cookies');
    expect(buttonByText(host, 'Duplicate bot'), 'no Duplicate without the calls that make a bot').toBeUndefined();
    await click(buttonByText(host, 'Export bot'));
    expect(portable.calls.find((c) => c.method === 'exportSave')?.args).toEqual(['nimbus', false, 'C:\\Users\\me\\nimbus.potato-bot.json']);
    expect(host.querySelector('[role="status"]')?.textContent).toContain('Saved to C:\\Users\\me\\nimbus.potato-bot.json');

    await click(host.querySelector('input[type="checkbox"]'));
    await click(buttonByText(host, 'Export bot'));
    expect(portable.calls.filter((c) => c.method === 'exportSave').at(-1)?.args[1]).toBe(true);
  });

  it('saves nothing when the dialog is cancelled', async () => {
    const portable = fakePortableTransport();
    vi.mocked(save).mockResolvedValue(null);
    await act(async () => root.render(<BotExportButtons bot={{ id: 'nimbus', name: 'Nimbus' }} portable={portable} />));
    await click(buttonByText(host, 'Export bot'));
    expect(portable.calls.some((c) => c.method === 'exportSave')).toBe(false);
  });

  it('duplicates into a second bot called "<name> copy" and tells the list to read again', async () => {
    const portable = fakePortableTransport();
    const companions = fakeCompanionTransport();
    const potato = fakePotatoTransport({ bots: [] });
    const create = vi.spyOn(companions, 'create');
    const onChanged = vi.fn();
    const onNotify = vi.fn();
    await act(async () => root.render(<BotExportButtons bot={{ id: 'nimbus', name: 'Nimbus' }} portable={portable} companionTransport={companions} potatoTransport={potato} onChanged={onChanged} onNotify={onNotify} />));
    await click(buttonByText(host, 'Duplicate bot'));
    expect(create.mock.calls[0][0].name).toBe('Nimbus copy');
    expect(portable.calls.find((c) => c.method === 'copyInto')?.args[0]).toBe('nimbus');
    expect(onChanged).toHaveBeenCalled();
    expect(onNotify).toHaveBeenCalledWith('Nimbus copy is ready. Its routines are paused.');
  });

  it('shows one plain line when the engine refuses', async () => {
    const portable = fakePortableTransport({ fail: 'POTATO_EXPORT_IO: Could not save the file: access denied.' });
    vi.mocked(save).mockResolvedValue('C:\\locked\\x.json');
    await act(async () => root.render(<BotExportButtons bot={{ id: 'nimbus', name: 'Nimbus' }} portable={portable} />));
    await click(buttonByText(host, 'Export bot'));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('Could not save the file');
  });
});
