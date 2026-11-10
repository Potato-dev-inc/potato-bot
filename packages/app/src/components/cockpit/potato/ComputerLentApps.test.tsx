// @vitest-environment jsdom
// "Apps from your PC": the rows by phase, the row actions (Take control, Hand back, Bring back, Show it / Keep it out of
// my way), Bring back's "is yours again" line, and the picker (chat apps first, elevated windows behind a button, the
// 2 s refresh, the warning line, the primary Lend button). The Tauri invoke is mocked; each call is checked by its
// command name and arguments.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LEND_COPY } from '../../../lib/potato-computer';
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
  vi.useRealTimers();
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

const commandsNamed = (names: string[]) => invoke.mock.calls.map(([command]) => command).filter((command) => names.includes(command));
const commands = () => invoke.mock.calls.map(([command, args]) => [command, args]).filter(([command]) => command !== 'potato_lend_list' && command !== 'potato_lend_thumbnail' && command !== 'potato_lend_candidates');

/** The app names in the picker, in the order they are listed (the heading is not an app). */
const pickerNames = (dialog: HTMLElement) => [...dialog.querySelectorAll('.ck-potato-lend-choice-text .ck-potato-lend-name')].map((el) => el.textContent);

describe('the empty state', () => {
  it('says what the bot can use, and that it can be taken back at any time', async () => {
    answer([]);
    await mount();
    expect(host.textContent).toContain('Fish can use Windows apps you already have open, like LINE.');
    expect(host.textContent).toContain('You can take it back at any time. It never moves your mouse.');
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
    expect(text).toContain('Fish is using it');
    expect(text).toContain('You have control. Fish is waiting for you.');
    expect(text).toContain('Fish needs you: Log in again');
    expect(text).toContain('Ready. Fish can use it now.');
  });

  it('says Picture paused only for a row whose picture is not live', async () => {
    answer([app({ id: 'a', appName: 'LINE', pictureLive: false }), app({ id: 'b', appName: 'Notepad', pictureLive: true })]);
    await mount();
    const rows = host.querySelectorAll('.ck-potato-lend-row');
    expect(rows[0].textContent).toContain('Picture paused');
    expect(rows[1].textContent).not.toContain('Picture paused');
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

  it('the mode is one segmented control, Show it selected by default', async () => {
    answer([app({ id: 'a1', mode: 'visible' })]);
    await mount();
    const seg = host.querySelector<HTMLElement>('.ck-potato-lend-seg');
    expect(seg?.getAttribute('role')).toBe('group');
    const buttons = [...seg!.querySelectorAll<HTMLButtonElement>('button')];
    expect(buttons.map((b) => b.textContent)).toEqual(['Show it', 'Keep it out of my way']);
    expect(buttons.map((b) => b.getAttribute('aria-pressed'))).toEqual(['true', 'false']);
    expect(buttons[1].title).toBe('Still running, see-through');
  });

  it('Keep it out of my way calls potato_lend_set_mode with hidden', async () => {
    answer([app({ id: 'a1', mode: 'visible' })]);
    await mount();
    await click('Keep it out of my way');
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

  it('Bring back calls potato_lend_bring_back, then says "is yours again" in the row and drops it after 3 seconds', async () => {
    vi.useFakeTimers();
    let gone = false;
    invoke.mockImplementation(async (command: string) => {
      switch (command) {
        case 'potato_lend_list':
          return gone ? [] : [app({ id: 'a1', appName: 'LINE' })];
        case 'potato_lend_bring_back':
          gone = true;
          return null;
        case 'potato_lend_thumbnail':
          return 'AAAA';
        default:
          return null;
      }
    });
    await mount();
    await click('Bring back');
    expect(commands()).toContainEqual(['potato_lend_bring_back', { id: 'a1' }]);
    expect(host.querySelector('.ck-potato-lend-rows')?.textContent).toContain('LINE is yours again.');
    expect(host.querySelector('[role="dialog"]')).toBeNull();

    await act(async () => {
      vi.advanceTimersByTime(2999);
    });
    expect(host.textContent).toContain('LINE is yours again.');
    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    expect(host.textContent).not.toContain('is yours again');
    expect(host.textContent).toContain(LEND_COPY.empty('Fish'));
  });
});

describe('the picker', () => {
  it('hides elevated windows behind a button and says why once it is shown', async () => {
    answer([], [win({ hwnd: 5, app_name: 'Regedit', elevated: true }), win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog).toBeTruthy();
    expect(dialog!.querySelector('input[value="5"]')).toBeNull();
    expect(dialog!.textContent).not.toContain('Regedit');

    await click("Show 1 more that can't be lent", dialog!);
    const elevated = dialog!.querySelector<HTMLInputElement>('input[value="5"]');
    expect(elevated?.disabled).toBe(true);
    expect(dialog!.textContent).toContain('Opened as administrator. Close it and open it normally to lend it.');
    expect(dialog!.querySelector<HTMLInputElement>('input[value="7"]')?.disabled).toBe(false);
    expect(dialog!.querySelector('button')?.textContent).not.toContain("can't be lent");
  });

  it('shows no "Show more" button when there are no elevated windows', async () => {
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(buttonByText(dialog, "Show 0 more that can't be lent")).toBeUndefined();
    expect([...dialog.querySelectorAll('button')].some((b) => b.textContent?.includes("can't be lent"))).toBe(false);
  });

  it('lists known chat apps first, then the rest under "Other apps"', async () => {
    answer([], [win({ hwnd: 1, app_name: 'Notepad' }), win({ hwnd: 2, app_name: 'Telegram' }), win({ hwnd: 3, app_name: 'Paint' }), win({ hwnd: 4, app_name: 'LINE' })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(pickerNames(dialog)).toEqual(['Telegram', 'LINE', 'Notepad', 'Paint']);
    const heading = dialog.querySelector('.ck-potato-lend-group');
    expect(heading?.textContent).toBe('Other apps');
    // The heading sits between the chat apps and the rest.
    const items = [...dialog.querySelectorAll('.ck-potato-lend-choices > li')];
    expect(items[2]).toBe(heading?.closest('li'));
  });

  it('reads the list again every 2 seconds while open, and stops when closed', async () => {
    vi.useFakeTimers();
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const candidateCalls = () => commandsNamed(['potato_lend_candidates']).length;
    expect(candidateCalls()).toBe(1);

    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(candidateCalls()).toBe(2);
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(candidateCalls()).toBe(3);

    await click('Cancel', document.querySelector<HTMLElement>('[role="dialog"]')!);
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(candidateCalls()).toBe(3);
  });

  it('says "No open apps yet" when there is nothing to lend', async () => {
    answer([], []);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.textContent).toContain('No open apps yet. Open the app now and it will show up here.');
  });

  it('warns that the bot reads and types as the person, before and after a pick', async () => {
    answer([], [win({ hwnd: 7, app_name: 'LINE' })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.textContent).toContain("Fish will read and type in this app as you. It won't type passwords. Take control any time.");

    dialog.querySelector<HTMLInputElement>('input[value="7"]')!.click();
    await act(async () => {});
    expect(dialog.textContent).toContain("Fish will read and type in LINE as you. It won't type passwords. Take control any time.");
  });

  it('the Lend button is primary and names the picked app', async () => {
    answer([], [win({ hwnd: 7, app_name: 'LINE' })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    const before = buttonByText(dialog, 'Lend this app');
    expect(before?.classList.contains('ck-btn--primary')).toBe(true);
    expect(before?.disabled).toBe(true);

    dialog.querySelector<HTMLInputElement>('input[value="7"]')!.click();
    await act(async () => {});
    const lendButton = buttonByText(dialog, 'Lend LINE');
    expect(lendButton?.classList.contains('ck-btn--primary')).toBe(true);
    expect(lendButton?.disabled).toBe(false);
  });

  it('the mode choices read "Show it" and "Keep it out of my way", with hints', async () => {
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.textContent).toContain('Show it');
    expect(dialog.textContent).toContain('Stays where it is, with a border');
    expect(dialog.textContent).toContain('Keep it out of my way');
    expect(dialog.textContent).toContain('Still running, see-through');
  });

  it('Lend starts the picked window with the default Show it mode', async () => {
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    dialog.querySelector<HTMLInputElement>('input[value="7"]')!.click();
    await act(async () => {});
    await click('Lend LINE', dialog);
    expect(commands()).toContainEqual(['potato_lend_start', { hwnd: 7, mode: 'visible' }]);
  });

  it('Lend starts the picked window with Keep it out of my way when that is chosen', async () => {
    answer([], [win({ hwnd: 7 })]);
    await mount();
    await click('Lend an open app');
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    dialog.querySelector<HTMLInputElement>('input[value="7"]')!.click();
    dialog.querySelector<HTMLInputElement>('input[name="ck-potato-lend-mode"]:not(:checked)')!.click();
    await act(async () => {});
    await click('Lend LINE', dialog);
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
