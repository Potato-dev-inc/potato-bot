// @vitest-environment jsdom
// Screen level: the Advanced tab of a bot whose brain is "Through my agents" shows the add flow for the owner's own agent
// (paste, check, plain summary, approve), picks it as the brain, and every step reaches the host's command. The real
// transport is used; only `invoke` is faked.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBotSettingsTransport } from '../../../lib/potato-botsettings';
import { AdvancedTab } from './AdvancedTab';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const DEFINITION = 'schema_version = 1\n[agent]\nid = "scribe"\nlabel = "Scribe"\n[binary]\nnames = ["scribe"]\n';
const NOTES = ["This agent decides for itself inside the sandbox. Approval cards don't apply.", "This agent uses its own account. Its cost isn't counted here."];
const SUMMARY = {
  id: 'scribe',
  label: 'Scribe',
  description: 'A small writing agent.',
  command: 'scribe run -- <your message>',
  installs: 'Nothing. scribe must already be in the sandbox for every user.',
  envNames: [],
  secretNames: [],
  fingerprint: 'abc123',
  notes: NOTES,
};
const view = (agents: unknown[], brainAgent: string | null, sandboxReady = true) => ({ agents, brainAgent, ownAccountNote: NOTES[1], decidesNote: NOTES[0], sandboxReady });
const ROW = { id: 'scribe', label: 'Scribe', description: '', command: SUMMARY.command, fingerprint: 'abc123', approvedAt: 1, secrets: [] };

let host: HTMLDivElement;
let root: Root;
let checkError: string | null;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  checkError = null;
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'potato_notify_get':
        return 'reply';
      case 'potato_open_work_list':
        return { items: [], helpers: [] };
      case 'potato_memory_list':
        return [];
      case 'potato_agents_get':
        return view([], null);
      case 'potato_agent_check':
        if (checkError) throw new Error(checkError);
        return SUMMARY;
      case 'potato_agent_add':
        return view([ROW], null);
      case 'potato_agent_pick':
        return view([ROW], 'scribe');
      default:
        throw new Error(`unexpected command ${command}`);
    }
  });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const flush = async () => act(async () => {});
const button = (text: string) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === text) ?? null;

async function click(el: Element | null | undefined) {
  if (!el) throw new Error('nothing to click');
  await act(async () => (el as HTMLElement).click());
  await flush();
}

async function typeIn(el: HTMLTextAreaElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await flush();
}

async function mount() {
  const transport = fakeBotSettingsTransport({ options: { brain: 'agents' } });
  await act(async () => root.render(<AdvancedTab bot={{ id: 'fish', name: 'Fish' }} transport={transport} defaultBrain="deepseek" />));
  await flush();
}

describe('Agents in the sandbox on the Advanced tab', () => {
  it('shows the section with both notes only when the brain is "Through my agents"', async () => {
    await mount();
    const section = host.querySelector('[aria-label="Agents in the sandbox"]')!;
    expect(section).not.toBeNull();
    expect(section.textContent).toContain("This agent decides for itself inside the sandbox. Approval cards don't apply.");
    expect(section.textContent).toContain("This agent uses its own account. Its cost isn't counted here.");
    expect(section.textContent).toContain('No agents added yet.');
    expect(invoke).toHaveBeenCalledWith('potato_agents_get', { botId: 'fish' });
    act(() => root.unmount());
    root = createRoot(host);
    await act(async () => root.render(<AdvancedTab bot={{ id: 'fish', name: 'Fish' }} transport={fakeBotSettingsTransport()} defaultBrain="deepseek" />));
    await flush();
    expect(host.querySelector('[aria-label="Agents in the sandbox"]')).toBeNull();
  });

  it('says plainly when there is no sandbox', async () => {
    invoke.mockImplementation(async (command: string) => (command === 'potato_agents_get' ? view([], null, false) : command === 'potato_notify_get' ? 'reply' : command === 'potato_memory_list' ? [] : { items: [], helpers: [] }));
    await mount();
    expect(host.textContent).toContain('Agents run only in the sandbox, and it is not switched on or not set up.');
  });

  it('pastes a definition, shows the plain summary, approves it, and picks it as the brain: each step reaches the host', async () => {
    await mount();
    await click(button('Add an agent'));
    const area = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="Paste a definition.toml"]')!;
    await typeIn(area, DEFINITION);
    await click(button('Check'));
    expect(invoke).toHaveBeenCalledWith('potato_agent_check', { definition: DEFINITION });
    const summary = host.querySelector('[aria-label="Agent summary"]')!;
    expect(summary.textContent).toContain('Scribe');
    expect(summary.textContent).toContain('scribe run -- <your message>');
    expect(summary.textContent).toContain('Nothing. scribe must already be in the sandbox');
    // nothing is saved until the owner approves
    expect(invoke.mock.calls.some(([c]) => c === 'potato_agent_add')).toBe(false);
    await click(button('Approve and add'));
    expect(invoke).toHaveBeenCalledWith('potato_agent_add', { botId: 'fish', definition: DEFINITION, fingerprint: 'abc123' });
    expect(host.textContent).toContain('scribe run -- <your message>');
    await click(button('Use as brain'));
    expect(invoke).toHaveBeenCalledWith('potato_agent_pick', { botId: 'fish', agentId: 'scribe' });
    expect(host.textContent).toContain('In use');
  });

  it('shows a mistake with its line and saves nothing', async () => {
    checkError = 'POTATO_AGENT_INVALID: Line 14: This version does not take that setting.';
    await mount();
    await click(button('Add an agent'));
    await typeIn(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="Paste a definition.toml"]')!, DEFINITION + 'stdin = "x"\n');
    await click(button('Check'));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Line 14: This version does not take that setting.');
    expect(host.querySelector('[aria-label="Agent summary"]')).toBeNull();
    expect(invoke.mock.calls.some(([c]) => c === 'potato_agent_add')).toBe(false);
  });

  describe('the values an agent asks for', () => {
    const KEY = 'potato_agent_secret_fish_scribe_SCRIBE_TOKEN';
    const SECRET_ROW = { ...ROW, secrets: [{ env: 'SCRIBE_TOKEN', key: KEY }] };
    const TYPED = 'typed-live-value-98765';
    let stored: string | null;

    beforeEach(() => {
      stored = null;
      const base = invoke.getMockImplementation()!;
      invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
        switch (command) {
          case 'potato_agents_get':
            return view([SECRET_ROW], 'scribe');
          case 'potato_agent_check':
            return { ...SUMMARY, secretNames: ['SCRIBE_TOKEN'], notes: [...NOTES, 'It asks you for: SCRIBE_TOKEN. You type each value in Settings after you approve.'] };
          case 'secret_storage_info':
            return { secure: true, label: 'your keychain' };
          case 'secret_get_masked':
            return stored ? { set: true, last4: stored.slice(-4) } : { set: false, last4: null };
          case 'secret_set':
            stored = String(args?.value);
            return undefined;
          default:
            return base(command, args);
        }
      });
    });

    it('shows the names before approval', async () => {
      await mount();
      await click(button('Add an agent'));
      await typeIn(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="Paste a definition.toml"]')!, DEFINITION);
      await click(button('Check'));
      const summary = host.querySelector('[aria-label="Agent summary"]')!;
      expect(summary.textContent).toContain('Asks you for: SCRIBE_TOKEN');
    });

    it('has a masked field for each name; the value goes to the credential store under that agent and shows only its last four', async () => {
      await mount();
      const field = host.querySelector<HTMLInputElement>('input[aria-label="Scribe: SCRIBE_TOKEN"]')!;
      expect(field).not.toBeNull();
      expect(field.type).toBe('password');
      expect(host.querySelector('[aria-label="Values for Scribe"]')?.textContent).toContain('given only to this agent');
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field, TYPED);
        field.dispatchEvent(new Event('input', { bubbles: true }));
      });
      await act(async () => {
        field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      });
      await flush();
      expect(invoke).toHaveBeenCalledWith('secret_set', { name: KEY, value: TYPED });
      // once saved the field is gone and only the end of the value is shown
      expect(host.querySelector('input[aria-label="Scribe: SCRIBE_TOKEN"]')).toBeNull();
      expect(host.textContent).not.toContain(TYPED);
      expect(host.textContent).toContain('•••• ' + TYPED.slice(-4));
      // and no agent command ever carried it
      expect(invoke.mock.calls.filter(([c]) => String(c).startsWith('potato_agent') ).every(([, a]) => !JSON.stringify(a ?? {}).includes(TYPED))).toBe(true);
    });
  });
});
