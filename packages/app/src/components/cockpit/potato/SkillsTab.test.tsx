// @vitest-environment jsdom
// The Skills tab, as the person meets it in the bot panel: the list, reading a skill, editing it, deleting it after a
// question, and the plain messages when the app refuses. The first test goes through the panel's own tab.
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBotSettingsTransport } from '../../../lib/potato-botsettings';
import type { PanelTab } from '../../../lib/potato-panel';
import { fakeSkillsTransport, type Skill } from '../../../lib/potato-skills';
import { fakeScreenTransport } from '../../../lib/potatoScreen';
import { BotPanel } from './BotPanel';
import { SkillsTab } from './SkillsTab';
import { buttonByText } from './test-helpers';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => { throw new Error('no engine'); }) }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

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

const REPORT: Skill = { slug: 'weekly-report', name: 'Weekly report', description: 'Summarise the week', body: '1. Open the notes\n2. Write five lines' };
const TAXES: Skill = { slug: 'taxes', name: 'Taxes', description: '', body: 'Collect the receipts' };

const settle = () => act(async () => new Promise((r) => setTimeout(r, 0)));
const click = (el: Element | null | undefined) => {
  if (!el) throw new Error('missing element');
  act(() => (el as HTMLElement).click());
};
function setValue(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value')!.set!.call(el, value);
  act(() => {
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
const rows = () => [...host.querySelectorAll('.ck-skill')];
const head = (name: string) => [...host.querySelectorAll<HTMLElement>('.ck-skill-head')].find((h) => h.textContent?.includes(name));

async function mount(transport = fakeSkillsTransport({ fish: [REPORT, TAXES] }), onNotify = vi.fn()) {
  await act(async () => root.render(<SkillsTab bot={{ id: 'fish', name: 'Fish' }} transport={transport} onNotify={onNotify} />));
  await settle();
  return { transport, onNotify };
}

describe('the Skills tab in the bot panel', () => {
  it('is a tab of the panel and lists the bot\'s skills with their descriptions', async () => {
    const skills = fakeSkillsTransport({ fish: [REPORT, TAXES] });
    function Panel() {
      const [tab, setTab] = useState<PanelTab>('computer');
      return (
        <BotPanel bot={{ id: 'fish', name: 'Fish' }} avatar="mochi" tab={tab} onTab={setTab} onClose={() => {}} screen={fakeScreenTransport()} through={false} onNotify={() => {}} botSettings={fakeBotSettingsTransport({ sandboxOn: true })} skills={skills} />
      );
    }
    await act(async () => root.render(<Panel />));
    await settle();
    click([...host.querySelectorAll('[role="tab"], button')].find((b) => b.textContent === 'Skills'));
    await settle();
    const panel = host.querySelector('[role="tabpanel"][aria-label="Skills"]') as HTMLElement;
    expect(panel.hidden).toBe(false);
    expect([...panel.querySelectorAll('.ck-skill-name')].map((n) => n.textContent)).toEqual(['Weekly report', 'Taxes']);
    expect(panel.textContent).toContain('Summarise the week');
    expect(panel.textContent).toContain('No description');
  });

  it('says how to make the first one when there are none', async () => {
    await mount(fakeSkillsTransport({ fish: [] }));
    expect(host.textContent).toContain('No skills yet');
  });

  it('opens a skill to read its steps and closes it again', async () => {
    await mount();
    expect(host.querySelector('.ck-skill-steps')).toBeNull();
    click(head('Weekly report'));
    expect(host.querySelector('.ck-skill-steps')?.textContent).toBe('1. Open the notes\n2. Write five lines');
    click(head('Weekly report'));
    expect(host.querySelector('.ck-skill-steps')).toBeNull();
  });

  it('edits the description and the steps, keeps the name, and says it was saved', async () => {
    const { transport, onNotify } = await mount();
    click(head('Weekly report'));
    click(buttonByText(host, 'Edit'));
    setValue(host.querySelector<HTMLInputElement>('.ck-skill-input')!, 'Friday summary');
    setValue(host.querySelector<HTMLTextAreaElement>('.ck-skill-textarea')!, 'Write three lines');
    click(buttonByText(host, 'Save'));
    await settle();
    expect(transport.calls.find((c) => c.method === 'save')?.args).toEqual(['fish', 'weekly-report', 'Friday summary', 'Write three lines']);
    expect(onNotify).toHaveBeenCalledWith('Saved "Weekly report".');
    expect(host.querySelector('.ck-skill-steps')?.textContent).toBe('Write three lines');
    expect(host.textContent).toContain('Friday summary');
    expect(rows()).toHaveLength(2);
  });

  it('will not save empty steps or a skill over the file limit, and Cancel drops the draft', async () => {
    const { transport } = await mount();
    click(head('Weekly report'));
    click(buttonByText(host, 'Edit'));
    const area = host.querySelector<HTMLTextAreaElement>('.ck-skill-textarea')!;
    setValue(area, '   ');
    expect(buttonByText(host, 'Save')!.disabled).toBe(true);
    setValue(area, 'x'.repeat(4100));
    expect(buttonByText(host, 'Save')!.disabled).toBe(true);
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('4096');
    click(buttonByText(host, 'Cancel'));
    expect(host.querySelector('.ck-skill-steps')?.textContent).toBe('1. Open the notes\n2. Write five lines');
    expect(transport.calls.some((c) => c.method === 'save')).toBe(false);
  });

  it('asks before it deletes, and deletes only on the second click', async () => {
    const { transport, onNotify } = await mount();
    click(head('Taxes'));
    click(buttonByText(host, 'Delete'));
    expect(host.textContent).toContain('Delete "Taxes"?');
    expect(transport.calls.some((c) => c.method === 'remove')).toBe(false);
    click(buttonByText(host, 'Cancel'));
    expect(transport.calls.some((c) => c.method === 'remove')).toBe(false);
    click(buttonByText(host, 'Delete'));
    click(buttonByText(host, 'Yes, delete'));
    await settle();
    expect(transport.calls.find((c) => c.method === 'remove')?.args).toEqual(['fish', 'taxes']);
    expect(onNotify).toHaveBeenCalledWith('Deleted "Taxes".');
    expect(rows().map((r) => r.querySelector('.ck-skill-name')?.textContent)).toEqual(['Weekly report']);
  });

  it('shows the app\'s plain sentence when a save fails, and when the list cannot be read', async () => {
    const failing = fakeSkillsTransport({ fish: [REPORT] });
    await mount(failing);
    failing.save = () => Promise.reject(new Error('POTATO_SKILL_INVALID: A skill can have at most 4096 characters. Nothing was saved.'));
    click(head('Weekly report'));
    click(buttonByText(host, 'Edit'));
    click(buttonByText(host, 'Save'));
    await settle();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('A skill can have at most 4096 characters. Nothing was saved.');
    act(() => root.unmount());
    root = createRoot(host);
    await mount(fakeSkillsTransport({}, { fail: 'POTATO_NOT_READY: Potato Bot is not ready yet.' }));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Potato Bot is not ready yet.');
  });
});
