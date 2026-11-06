// @vitest-environment jsdom
// The real Advanced tab lists the saved Always allow rules (bot name, plain class description), each with a Remove button;
// Remove goes to the engine and the row goes at once. With no rules the section says so.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBotSettingsTransport, type FakeBotSettingsTransport } from '../../../lib/potato-botsettings';
import { AdvancedTab } from './AdvancedTab';
import { buttonByText } from './test-helpers';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'potato_notify_get':
        return 'reply';
      case 'potato_open_work_list':
        return { items: [], helpers: [] };
      case 'potato_memory_list':
        return [];
      default:
        throw new Error(`unexpected command ${command}`);
    }
  });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const flush = () => act(async () => {});
async function mount(transport: FakeBotSettingsTransport) {
  await act(async () => root.render(<AdvancedTab bot={{ id: 'fish', name: 'Fish' }} transport={transport} defaultBrain="deepseek" />));
  await flush();
}
const section = () => host.querySelector('[aria-labelledby="ck-adv-always-allow"]');
const RULE = { class: 'git push', description: 'pushes to a remote', at: 1 };

describe('Advanced tab: Always allow rules', () => {
  it('lists each saved rule with the bot name and the class in plain words', async () => {
    const transport = fakeBotSettingsTransport({ options: { alwaysAllow: [RULE, { class: 'publish', description: 'publishes a package', at: 2 }] } });
    await mount(transport);
    const rows = [...(section()?.querySelectorAll('li') ?? [])].map((li) => li.textContent);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toContain('Fish');
    expect(rows[0]).toContain('pushes to a remote');
    expect(rows[1]).toContain('publishes a package');
    expect(section()?.querySelectorAll('button')).toHaveLength(2);
  });

  it('Remove asks the engine and the row goes at once', async () => {
    const transport = fakeBotSettingsTransport({ options: { alwaysAllow: [RULE] } });
    await mount(transport);
    await act(async () => buttonByText(section() as HTMLElement, 'Remove')!.click());
    await flush();
    expect(transport.calls.find((c) => c.method === 'removeAlways')?.args).toEqual(['fish', 'git push']);
    expect(section()?.querySelectorAll('li')).toHaveLength(0);
    expect(section()?.textContent).toContain('Nothing is always allowed');
  });

  it('a failed Remove shows a sentence and keeps the row', async () => {
    const transport = fakeBotSettingsTransport({ options: { alwaysAllow: [RULE] }, fail: { removeAlways: 'Could not save that.' } });
    await mount(transport);
    await act(async () => buttonByText(section() as HTMLElement, 'Remove')!.click());
    await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Could not save that.');
    expect(section()?.querySelectorAll('li')).toHaveLength(1);
  });

  it('with no rules it says nothing is always allowed', async () => {
    await mount(fakeBotSettingsTransport());
    expect(section()?.textContent).toContain('Nothing is always allowed');
    expect(section()?.querySelectorAll('button')).toHaveLength(0);
  });
});
