// @vitest-environment jsdom
// The add sheet: the filled name, the memory line, the refusal shown as it comes.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { botList, typeInto } from './botTestKit';

const state = vi.hoisted(() => ({
  invoke: vi.fn(async (_cmd: string, _args?: unknown) => null as unknown),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: state.invoke }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { AddBotSheet } from './AddBotSheet';

let host: HTMLDivElement;
let root: Root;
const one = [{ id: 'p1', name: 'fish', suggestedName: 'Ren的fish' }];
const two = [...one, { id: 'p2', name: 'cat', suggestedName: 'Ren的cat' }];
const text = () => document.body.textContent ?? '';
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === label) as HTMLButtonElement | undefined;
const nameInput = () => document.querySelector('.ck-spaces-field input') as HTMLInputElement;
const added = vi.fn();
const closed = vi.fn();

async function render(candidates: unknown[], add?: (a: unknown) => unknown) {
  state.invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'space_bot_candidates') return { space: 's1', candidates };
    if (cmd === 'space_bot_add') return add ? add(args) : botList();
    return null;
  });
  await act(async () => root.render(<AddBotSheet space="s1" onAdded={added} onClose={closed} />));
  await act(async () => {});
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  state.invoke.mockReset();
  added.mockReset();
  closed.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('AddBotSheet', () => {
  it('is a dialog titled Add a bot', async () => {
    await render(one);
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    expect(document.querySelector('h2')?.textContent).toBe('Add a bot');
  });

  it('fills the name with the owner name and the bot name', async () => {
    await render(one);
    expect(nameInput().value).toBe('Ren的fish');
    expect(text()).toContain('Name in this space');
  });

  it('shows the memory line every time it opens', async () => {
    await render(one);
    expect(text()).toContain('People here may get your bot to repeat what it remembers.');
    await act(async () => root.render(<div />));
    await render(one);
    expect(text()).toContain('People here may get your bot to repeat what it remembers.');
  });

  it('adds with the bot id and the edited name, then closes and hands over the list', async () => {
    await render(one);
    typeInto(nameInput(), 'Ren的helper');
    await act(async () => button('Add bot')!.click());
    expect(state.invoke).toHaveBeenCalledWith('space_bot_add', { space: 's1', potatoBot: 'p1', name: 'Ren的helper' });
    expect(added).toHaveBeenCalledTimes(1);
    expect(closed).toHaveBeenCalledTimes(1);
  });

  it('with two bots offers a picker and refills the name', async () => {
    await render(two);
    const sel = document.querySelector('select') as HTMLSelectElement;
    expect([...sel.options].map((o) => o.textContent)).toEqual(['fish', 'cat']);
    await act(async () => {
      sel.value = 'p2';
      sel.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(nameInput().value).toBe('Ren的cat');
    await act(async () => button('Add bot')!.click());
    expect(state.invoke).toHaveBeenCalledWith('space_bot_add', { space: 's1', potatoBot: 'p2', name: 'Ren的cat' });
  });

  it('shows a taken name refusal as it comes and stays open', async () => {
    await render(one, () => {
      throw 'The name Ren的fish is already taken in this space.';
    });
    await act(async () => button('Add bot')!.click());
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The name Ren的fish is already taken in this space.');
    expect(closed).not.toHaveBeenCalled();
    expect(added).not.toHaveBeenCalled();
    expect(button('Add bot')!.disabled).toBe(false);
  });

  it('disables Add with an empty name', async () => {
    await render(one);
    typeInto(nameInput(), '   ');
    expect(button('Add bot')!.disabled).toBe(true);
  });

  it('says so when no bot can join and has nothing to add', async () => {
    await render([]);
    expect(text()).toContain("You have no Potato bot that can join a space. Bots made with custom agents can't join.");
    expect(button('Add bot')!.disabled).toBe(true);
    expect(text()).not.toContain('People here may get your bot');
  });

  it('Cancel closes without adding', async () => {
    await render(one);
    await act(async () => button('Cancel')!.click());
    expect(closed).toHaveBeenCalled();
    expect(state.invoke).not.toHaveBeenCalledWith('space_bot_add', expect.anything());
  });
});
