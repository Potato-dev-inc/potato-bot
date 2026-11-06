// @vitest-environment jsdom
// The Sites section of the real Advanced tab: off by default, a switch and a list that only the owner sets, the owner's words saved
// through the engine and read back cleaned, plain sentences for a bad site, and a plain statement of what the list cannot cover.
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
const section = () => host.querySelector<HTMLElement>('[aria-labelledby="ck-adv-sites"]')!;
const box = () => section().querySelector<HTMLTextAreaElement>('textarea')!;
const toggle = () => section().querySelector<HTMLButtonElement>('[role="switch"]')!;
const saveButton = () => buttonByText(section(), 'Save sites')!;
const type = async (text: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(box(), text);
    box().dispatchEvent(new Event('input', { bubbles: true }));
  });
};
const setSites = (t: FakeBotSettingsTransport) => t.calls.filter((c) => c.method === 'setSites').map((c) => c.args);

describe('Advanced tab: Sites', () => {
  it('is off by default, with an empty list, and says what is true in that state', async () => {
    await mount(fakeBotSettingsTransport());
    expect(toggle().getAttribute('aria-checked')).toBe('false');
    expect(section().textContent).toContain('Off. The bot’s web tools can open any public site.');
    expect(box().value).toBe('');
    expect(saveButton().disabled).toBe(true);
    expect(section().querySelector('[role="note"]')).toBeNull();
  });

  it('says plainly what the list does not cover, whether it is on or off', async () => {
    await mount(fakeBotSettingsTransport());
    const note = section().textContent ?? '';
    expect(note).toContain('What this does not cover.');
    expect(note).toContain('commands the bot runs in its terminal');
    expect(note).toContain('links the bot clicks inside a page');
    expect(note).toContain('not a firewall');
    await act(async () => toggle().click());
    await flush();
    expect(section().textContent).toContain('not a firewall');
  });

  it('turning it on saves the switch with the list as it is, and warns when the list is empty', async () => {
    const t = fakeBotSettingsTransport();
    await mount(t);
    await act(async () => toggle().click());
    await flush();
    expect(setSites(t)).toEqual([['fish', { enabled: true, sites: [] }]]);
    expect(toggle().getAttribute('aria-checked')).toBe('true');
    expect(section().textContent).toContain('On. The bot’s web tools open only the sites below.');
    expect(section().querySelector('[role="note"]')?.textContent).toContain('open nothing');
    expect(section().querySelector('[role="status"]')?.textContent).toBe('The bot now opens only the sites in the list.');
  });

  it('saves the list as typed (lines or commas) and shows it back cleaned and without repeats', async () => {
    const t = fakeBotSettingsTransport();
    await mount(t);
    await type('https://Docs.RS/serde, example.com\n\nexample.com\n*.rust-lang.org');
    expect(saveButton().disabled).toBe(false);
    await act(async () => saveButton().click());
    await flush();
    expect(setSites(t)).toEqual([['fish', { enabled: false, sites: ['https://Docs.RS/serde', 'example.com', 'example.com', '*.rust-lang.org'] }]]);
    expect(box().value).toBe('docs.rs\nexample.com\nrust-lang.org');
    expect(section().querySelector('[role="status"]')?.textContent).toBe('Saved the list of sites.');
    expect(saveButton().disabled).toBe(true);
  });

  it('a site that is not a site name gets a plain sentence and nothing is saved', async () => {
    const t = fakeBotSettingsTransport({ options: { siteLimit: { enabled: true, sites: ['example.com'] } } });
    await mount(t);
    expect(box().value).toBe('example.com');
    await type('example.com\n1.2.3.4');
    await act(async () => saveButton().click());
    await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Type a site name like example.com.');
    expect(section().querySelector('[role="status"]')).toBeNull();
    expect(box().value).toBe('example.com\n1.2.3.4');
    expect(toggle().getAttribute('aria-checked')).toBe('true');
  });

  it('turning it off keeps the list', async () => {
    const t = fakeBotSettingsTransport({ options: { siteLimit: { enabled: true, sites: ['example.com'] } } });
    await mount(t);
    await act(async () => toggle().click());
    await flush();
    expect(setSites(t)).toEqual([['fish', { enabled: false, sites: ['example.com'] }]]);
    expect(box().value).toBe('example.com');
    expect(section().querySelector('[role="status"]')?.textContent).toBe('The bot can open any public site again.');
  });

  it('an older app that sends no limit reads as off', async () => {
    const t = fakeBotSettingsTransport();
    const original = t.options;
    t.options = async (botId) => {
      const options = await original(botId);
      delete options.siteLimit;
      return options;
    };
    await mount(t);
    expect(toggle().getAttribute('aria-checked')).toBe('false');
  });
});
