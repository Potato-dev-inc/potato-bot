// @vitest-environment jsdom
// The Ctrl+K jump palette: it filters bots, groups and pages as the person types, runs the one picked with Enter or a
// click, moves with the arrow keys, and closes on Escape.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandPalette } from './CommandPalette';
import { filterPalette, nextPaletteIndex, type PaletteItem } from '../../../lib/potato-palette';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

const items = (runs: Record<string, () => void> = {}): PaletteItem[] => [
  { id: 'bot:fish', label: 'Fish', hint: 'Bot', run: runs.fish ?? (() => {}) },
  { id: 'bot:bee', label: 'Bee', hint: 'Bot', run: runs.bee ?? (() => {}) },
  { id: 'room:team', label: 'Team chat', hint: 'Group', run: runs.team ?? (() => {}) },
  { id: 'page:settings', label: 'Settings', hint: 'Page', run: runs.settings ?? (() => {}) },
];

const labels = () => Array.from(host.querySelectorAll('.ck-potato-palette-label')).map((el) => el.textContent);
const input = () => host.querySelector<HTMLInputElement>('input.ck-potato-palette-input')!;
const type = (text: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  setter.call(input(), text);
  act(() => {
    input().dispatchEvent(new Event('input', { bubbles: true }));
  });
};
const press = (key: string) =>
  act(() => {
    host.querySelector('.ck-potato-palette')!.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  });

describe('filterPalette and nextPaletteIndex', () => {
  it('matches the label or the hint, ignoring case and spaces around the query', () => {
    expect(filterPalette(items(), '  BOT ').map((i) => i.label)).toEqual(['Fish', 'Bee']);
    expect(filterPalette(items(), 'team').map((i) => i.label)).toEqual(['Team chat']);
    expect(filterPalette(items(), '')).toHaveLength(4);
  });

  it('wraps the highlight at both ends and reports -1 when there is nothing', () => {
    expect(nextPaletteIndex(2, 3, 1)).toBe(0);
    expect(nextPaletteIndex(0, 3, -1)).toBe(2);
    expect(nextPaletteIndex(-1, 3, 1)).toBe(0);
    expect(nextPaletteIndex(0, 0, 1)).toBe(-1);
  });
});

describe('CommandPalette', () => {
  it('lists every place, focuses the box, and narrows as the person types', () => {
    act(() => root.render(<CommandPalette items={items()} onClose={() => {}} />));
    expect(labels()).toEqual(['Fish', 'Bee', 'Team chat', 'Settings']);
    expect(document.activeElement).toBe(input());
    type('set');
    expect(labels()).toEqual(['Settings']);
    type('nobody');
    expect(host.textContent).toContain('Nothing matches');
  });

  it('Enter runs the highlighted row and closes; the arrow keys move the highlight first', () => {
    const runBee = vi.fn();
    const onClose = vi.fn();
    act(() => root.render(<CommandPalette items={items({ bee: runBee })} onClose={onClose} />));
    press('ArrowDown');
    press('Enter');
    expect(runBee).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('a click runs that row; Escape closes without running anything', () => {
    const runTeam = vi.fn();
    const onClose = vi.fn();
    act(() => root.render(<CommandPalette items={items({ team: runTeam })} onClose={onClose} />));
    act(() => {
      host.querySelectorAll<HTMLLIElement>('.ck-potato-palette-item')[2]!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(runTeam).toHaveBeenCalledTimes(1);
    press('Escape');
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
