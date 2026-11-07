// @vitest-environment jsdom
// The bot panel: every line of the spec, and which controls each role gets.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceBotList, SpaceChannel } from '../../lib/spaces';
import { botList, botRow, otherRow, typeInto } from './botTestKit';

const state = vi.hoisted(() => ({
  list: null as unknown,
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => null as unknown),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: state.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { BotPanel, heldBackText } from './BotPanel';

let host: HTMLDivElement;
let root: Root;
const channels: SpaceChannel[] = [
  { id: 'c1', name: 'general', archived: false, unread: 0 },
  { id: 'c2', name: 'books', archived: false, unread: 0 },
  { id: 'c3', name: 'old', archived: true, unread: 0 },
];
const calls = () => state.invoke.mock.calls.map(([c, a]) => [c, a] as [string, Record<string, unknown>]);
const called = (cmd: string) => calls().filter(([c]) => c === cmd);

async function render(list: SpaceBotList, onClose = () => {}) {
  state.list = list;
  await act(async () => root.render(<div />)); // a fresh panel each time
  state.invoke.mockImplementation(async () => state.list);
  await act(async () => root.render(<BotPanel space="s1" spaceName="Book club" channels={channels} onClose={onClose} />));
  await act(async () => {});
}
const text = () => document.body.textContent ?? '';
const buttons = (label: string) => [...document.querySelectorAll('button')].filter((b) => b.textContent?.trim() === label);
const button = (label: string) => buttons(label)[0] ?? null;
const byLabel = (label: string) =>
  [...document.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith(label))?.querySelector('input, select') as
    | HTMLInputElement | HTMLSelectElement | null;
const click = (el: Element | null) => act(async () => { (el as HTMLElement).click(); });
const blur = (el: HTMLElement) => act(async () => { el.dispatchEvent(new FocusEvent('focusout', { bubbles: true })); });

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.invoke.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('BotPanel lines', () => {
  it('shows the title and every bot with its owner', async () => {
    await render(botList());
    expect(text()).toContain('Bots in Book club');
    expect(text()).toContain('Ren的fish');
    expect(text()).toContain('Sam的cat');
    expect(text()).toContain('Owner: Sam');
    expect(text()).toContain('Your bot');
  });

  it('shows the non-owner chip when someone else owns a bot, and not otherwise', async () => {
    await render(botList());
    expect(text()).toContain("Bots here can chat and read their memory. Only a bot's owner can use its tools.");
    await render(botList({ bots: [botRow()] }));
    expect(text()).not.toContain("Only a bot's owner can use its tools");
  });

  it('shows the answered and held back lines for my bot with the numbers', async () => {
    await render(botList({ bots: [botRow({ usedToday: 7, dayCap: 40, heldBackToday: 4 })] }));
    expect(text()).toContain('Answered 7 of 40 today.');
    expect(text()).toContain('Held back 4 replies today because they looked private.');
    expect(heldBackText(1)).toBe('Held back 1 reply today because they looked private.');
  });

  it('shows the waiting line for each pending add', async () => {
    await render(botList({ pendingAdd: ['Sam的dog', 'Sam的owl'] }));
    expect(text()).toContain("Waiting for the space owner's computer: Sam的dog");
    expect(text()).toContain("Waiting for the space owner's computer: Sam的owl");
  });

  it('no waiting line when nothing is pending', async () => {
    await render(botList());
    expect(text()).not.toContain('Waiting for the space owner');
  });

  it('says so when there are no bots', async () => {
    await render(botList({ bots: [] }));
    expect(text()).toContain('No bots in this space yet.');
  });

  it('shows the add blocked sentence as it comes and disables Add', async () => {
    await render(botList({ addBlocked: 'You already have 3 bots here.' }));
    expect(text()).toContain('You already have 3 bots here.');
    expect((button('Add my bot') as HTMLButtonElement).disabled).toBe(true);
  });

  it('Add my bot is enabled when nothing blocks it', async () => {
    await render(botList());
    expect((button('Add my bot') as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows the silenced and hidden marks', async () => {
    await render(botList({ bots: [botRow({ silenced: true }), otherRow({ muted: true })] }));
    expect(text()).toContain('Silenced');
    expect(text()).toContain('Hidden here');
  });

  it('shows a refusal from the list command as it comes', async () => {
    state.invoke.mockImplementation(async () => {
      throw 'That name is taken here.';
    });
    await act(async () => root.render(<BotPanel space="s1" spaceName="Book club" channels={channels} onClose={() => {}} />));
    await act(async () => {});
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('That name is taken here.');
  });
});

describe('BotPanel roles', () => {
  it('the bot owner sees mode, memory, daily limit and silence on their own bot only', async () => {
    await render(botList({ isOwner: false, bots: [botRow(), otherRow()] }));
    expect(document.querySelectorAll('[role="radiogroup"][aria-label^="When"]').length).toBe(1);
    expect(text()).toContain('Only when called');
    expect(text()).toContain('When it helps');
    expect(byLabel('Memory')).not.toBeNull();
    expect(byLabel('Daily limit')).not.toBeNull();
    expect(byLabel('Silence this bot')).not.toBeNull();
    expect(document.querySelectorAll('fieldset').length).toBe(1);
  });

  it('a member who is not the space owner sees no space owner settings and Remove only on their own bot', async () => {
    await render(botList({ isOwner: false }));
    expect(text()).not.toContain('Space owner settings');
    expect(text()).not.toContain('Bots per person');
    expect(text()).not.toContain('Allowed channels');
    expect(buttons('Remove bot').length).toBe(1);
  });

  it('the space owner sees Bots per person from 0 to 10 and Allowed channels', async () => {
    await render(botList({ isOwner: true }));
    expect(text()).toContain('Space owner settings');
    const sel = byLabel('Bots per person') as HTMLSelectElement;
    expect([...sel.options].map((o) => o.value)).toEqual(['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '10']);
    expect(sel.value).toBe('3');
    expect(text()).toContain('Allowed channels');
    expect(text()).toContain('All channels');
    expect(text()).toContain('Only these');
  });

  it('the space owner can remove any bot', async () => {
    await render(botList({ isOwner: true }));
    expect(buttons('Remove bot').length).toBe(2);
  });

  it('every member can hide any bot on this computer', async () => {
    await render(botList({ isOwner: false }));
    expect(buttons('Hide on this computer').length).toBe(2);
  });
});

describe('BotPanel actions', () => {
  it('picks a mode', async () => {
    await render(botList({ bots: [botRow({ mode: 'called' })] }));
    await click(byLabel('When it helps'));
    expect(called('space_bot_local')[0]).toEqual(['space_bot_local', { space: 's1', bot: 'b1', patch: { mode: 'helps' } }]);
  });

  it('switches memory', async () => {
    await render(botList({ bots: [botRow({ memory: true })] }));
    expect(byLabel('Memory')!.getAttribute('role')).toBe('switch');
    await click(byLabel('Memory'));
    expect(called('space_bot_local')[0][1]).toEqual({ space: 's1', bot: 'b1', patch: { memory: false } });
  });

  it('silences my bot', async () => {
    await render(botList({ bots: [botRow()] }));
    await click(byLabel('Silence this bot'));
    expect(called('space_bot_local')[0][1]).toEqual({ space: 's1', bot: 'b1', patch: { silenced: true } });
  });

  it('commits the daily limit on blur and ignores an empty one', async () => {
    await render(botList({ bots: [botRow({ dayCap: 50 })] }));
    const cap = byLabel('Daily limit') as HTMLInputElement;
    typeInto(cap, '120');
    await blur(cap);
    expect(called('space_bot_local')[0][1]).toEqual({ space: 's1', bot: 'b1', patch: { dayCap: 120 } });
    state.invoke.mockClear();
    typeInto(cap, '');
    await blur(cap);
    expect(called('space_bot_local').length).toBe(0);
  });

  it('hides and shows a bot on this computer', async () => {
    await render(botList({ bots: [otherRow({ muted: false })] }));
    await click(button('Hide on this computer'));
    expect(called('space_bot_local')[0][1]).toEqual({ space: 's1', bot: 'b2', patch: { muted: true } });
    await render(botList({ bots: [otherRow({ muted: true })] }));
    state.invoke.mockClear();
    await click(button('Show on this computer'));
    expect(called('space_bot_local')[0][1]).toEqual({ space: 's1', bot: 'b2', patch: { muted: false } });
  });

  it('removing asks first, and Keep sends nothing', async () => {
    await render(botList({ bots: [botRow()] }));
    await click(button('Remove bot'));
    expect(text()).toContain('Remove Ren的fish from this space?');
    await click(button('Keep'));
    expect(called('space_bot_remove').length).toBe(0);
    await click(button('Remove bot'));
    await click(button('Yes, remove'));
    expect(called('space_bot_remove')[0]).toEqual(['space_bot_remove', { space: 's1', bot: 'b1' }]);
  });

  it('shows a refusal from a command as it comes', async () => {
    await render(botList({ bots: [botRow()] }));
    state.invoke.mockImplementation(async () => {
      throw 'Only the bot owner can change this.';
    });
    await click(byLabel('Memory'));
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('Only the bot owner can change this.');
  });

  it('saves the rules with all channels', async () => {
    await render(botList({ isOwner: true, bots: [] }));
    const sel = byLabel('Bots per person') as HTMLSelectElement;
    await act(async () => {
      sel.value = '5';
      sel.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await click(button('Save bot rules'));
    expect(called('space_bot_rules')[0]).toEqual(['space_bot_rules', { space: 's1', perPerson: 5, channels: null }]);
  });

  it('saves the rules with only some channels and leaves archived ones out', async () => {
    await render(botList({ isOwner: true, bots: [] }));
    await click(byLabel('Only these'));
    expect(text()).toContain('# general');
    expect(text()).toContain('# books');
    expect(text()).not.toContain('# old');
    await click(byLabel('# general'));
    await click(button('Save bot rules'));
    expect(called('space_bot_rules')[0][1]).toEqual({ space: 's1', perPerson: 3, channels: ['c2'] });
  });

  it('shows the saved channel choice', async () => {
    await render(botList({ isOwner: true, bots: [], rules: { perPerson: 1, channels: ['c1'] } }));
    expect((byLabel('Only these') as HTMLInputElement).checked).toBe(true);
    expect((byLabel('# general') as HTMLInputElement).checked).toBe(true);
    expect((byLabel('# books') as HTMLInputElement).checked).toBe(false);
  });

  it('Close calls onClose', async () => {
    const onClose = vi.fn();
    await render(botList(), onClose);
    await click(button('Close'));
    expect(onClose).toHaveBeenCalled();
  });
});
