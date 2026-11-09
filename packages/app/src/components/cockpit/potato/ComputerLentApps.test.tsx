// @vitest-environment jsdom
// "Apps from your PC": the rows by phase, the row actions (Take control, Hand back, Bring back, Visible / Hidden), and the
// picker (elevated windows disabled, Lend starts with the chosen mode). The Tauri invoke is mocked; each call is checked
// by its command name and arguments.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LentView, WinInfo } from '../../../lib/potatoLend';
import { ComputerLentApps } from './ComputerLentApps';
import { buttonByText } from './test-helpers';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', async (orig) => ({ ...(await orig<object>()), invoke }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  invoke.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.querySelectorAll('.ck-potato-lend-overlay').forEach((el) => el.remove());
});

const BOT = { name: 'Fish' };

const app = (over: Partial<LentView> = {}): LentView => ({ id: 'a1', hwnd: 10, pid: 2, appName: 'LINE', title: 'LINE', mode: 'visible', phase: 'bot', reason: null, ...over });

const win = (over: Partial<WinInfo> = {}): WinInfo => ({
  hwnd: 7,
  pid: 1,
  title: 'LINE',
  exe: 'LINE.exe',
  app_name: 'LINE',
  rect: { x: 0, y: 0, w: 800, h: 600 },
  elevated: false,
  minimized: false,
  ...over,
});

/** Routes each command to a canned answer; `list` and `candidates` are the two that change per test. */
function answer(list: LentView[], candidates: WinInfo[] = []) {
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'potato_lend_list':
        return list;
      case 'potato_lend_candidates':
        return candidates;
      case 'potato_lend_thumbnail':
        return 'AAAA';
      default:
        return null;
    }
  });
}

async function mount() {
  await act(async () => root.render(<ComputerLentApps bot={BOT} />));
  await act(async () => {});
}

const click = async (label: string, scope: HTMLElement | Document = host) => {
  const button = buttonByText(scope as HTMLElement, label) ?? [...scope.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === label);
  expect(button, `a button named ${label}`).toBeTruthy();
  await act(async () => button!.click());
  await act(async () => {});
};

const commands = () => invoke.mock.calls.map(([command, args]) => [command, args]).filter(([command]) => command !== 'potato_lend_list' && command !== 'potato_lend_thumbnail' && command !== 'potato_lend_candidates');

describe('the empty state', () => {
  it('says what the bot can use and offers the picker', async () => {
    answer([]);
    await mount();
    expect(host.textContent).toContain('Fish can use Windows apps you already have open, like LINE.');
    expect(buttonByText(host, 'Lend an open app')).toBeTruthy();
    expect(host.querySelector('.ck-potato-lend-row')).toBeNull();
  });
});

describe('the rows', () => {
  it('shows the state text for each phase', async () => {
    answer([
      app({ id: 'a', appName: 'LINE', phase: 'bot' }),
      app({ id: 'b', appName: 'Notepad', phase: 'you' }),
      app({ id: 'c', appName: 'Mail', phase: 'needs_you', reason: 'Log in again' }),
      app({ id: 'd', appName: 'Paint', phase: 'free' }),
    ]);
    await mount();
    const text = host.querySelector('.ck-potato-lend-rows')?.textContent ?? '';
    expect(text).toContain('Used by Fish');
    expect(text).toContain('You have control');
    expect(text).toContain('Fish needs you: Log in again');
    expect(text).toContain('Waiting');
  });

  it('shows Hand back when the person has control and Take control otherwise', async () => {
    answer([app({ id: 'b', appName: 'Notepad', phase: 'you' }), app({ id: 'a', appName: 'LINE', phase: 'bot' })]);
    await mount();
    const rows = [...host.querySelectorAll('.ck-potato-lend-row')];
    expect(rows[0].textContent).toContain('Hand back');
    expect(rows[0].textContent).not.toContain('Take control');
    expect(rows[1].textContent).toContain('Take control');
    expect(rows[1].textContent).not.toContain('Hand back');
  });
});

describe('the row actions', () => {
  it('Take control calls potato_lend_take_over with the app id', async () => {
    answer([app({ id: 'a1', phase: 'bot' })]);
    await mount();
    await click('Take control');
    expect(commands()).toContainEqual(['potato_lend_take_over', { id: 'a1' }]);
  });

  it('Hand back calls potato_lend_hand_back with the app id', async () => {
    answer([app({ id: 'a1', phase: 'you' })]);
    await mount();
    await click('Hand back');
    expect(commands()).toContainEqual(['potato_lend_hand_back', { id: 'a1' }]);
  });

  it('Bring back calls potato_lend_bring_back with the app id', async () => {
    answer([app({ id: 'a1' })]);
    await mount();
    await click('Bring back');
    expect(commands()).toContainEqual(['potato_lend_bring_back', { id: 'a1' }]);
  });

  it('the Hidden choice calls potato_lend_set_mode', async () => {
    answer([app({ id: 'a1', mode: 'visible' })]);
    await mount();
    await click('Hidden');
    expect(commands()).toContainEqual(['potato_lend_set_mode', { id: 'a1', mode: 'hidden' }]);
  });

  it('a failed action shows its words inline', async () => {
    answer([app({ id: 'a1', phase: 'bot' })]);
    await mount();
    invoke.mockImplementation(async (command: string) => {
      if (command === 'potato_lend_take_over') throw new Error('person has control');
      return [app({ id: 'a1', phase: 'bot' })];
    });
    await click('Take control');
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('You have control of this app right now.');
  });
});

describe('the picker', () => {
  it('disables elevated windows and says why', async () => {
    answer([], [win({ hwnd: 5, app_name: 'Regedit', elevated: true }), win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog).toBeTruthy();
    const elevated = dialog!.querySelector<HTMLInputElement>('input[value="5"]');
    expect(elevated?.disabled).toBe(true);
    expect(dialog!.textContent).toContain("Runs as administrator, can't be lent");
    expect(dialog!.querySelector<HTMLInputElement>('input[value="7"]')?.disabled).toBe(false);
    expect(dialog!.textContent).toContain('Fish will use this app as you.');
  });

  it('Lend starts the picked window with the default Visible mode', async () => {
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    dialog.querySelector<HTMLInputElement>('input[value="7"]')!.click();
    await act(async () => {});
    await click('Lend', dialog);
    expect(commands()).toContainEqual(['potato_lend_start', { hwnd: 7, mode: 'visible' }]);
  });

  it('Lend starts the picked window with Hidden when that is chosen', async () => {
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    dialog.querySelector<HTMLInputElement>('input[value="7"]')!.click();
    dialog.querySelector<HTMLInputElement>('input[name="ck-potato-lend-mode"]:not(:checked)')!.click();
    await act(async () => {});
    await click('Lend', dialog);
    expect(commands()).toContainEqual(['potato_lend_start', { hwnd: 7, mode: 'hidden' }]);
  });

  it('Esc closes the picker', async () => {
    answer([], [win()]);
    await mount();
    await click('Lend an open app');
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
});
