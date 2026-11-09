// Test-only helpers for Settings section rows. They mount the real `SectionRows` with a fake SettingsEnv.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { vi, type Mock } from 'vitest';
import type { Settings } from '@puppet-master/shared';
import { SectionRows } from './SectionView';
import type { RowDef, SettingsEnv } from './model';
import type { SettingsPatch } from '../../lib/settings';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

export interface Mounted {
  host: HTMLDivElement;
  env: SettingsEnv & { patches: SettingsPatch[]; confirm: Mock };
  unmount: () => Promise<void>;
}

export interface MountOptions {
  settings?: Partial<Settings>;
  env?: Partial<SettingsEnv>;
}

function baseSettings(over: Partial<Settings> | undefined): Settings {
  return {
    theme: 'dark',
    potato_bot_enabled: true,
    potato_sandbox_enabled: true,
    ...over,
  } as Settings;
}

export async function settle(times = 4): Promise<void> {
  for (let i = 0; i < times; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setImmediate(resolve));
    });
  }
}

export async function mountRows(rows: RowDef[], over: MountOptions = {}): Promise<Mounted> {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root: Root = createRoot(host);
  const patches: SettingsPatch[] = [];
  const confirm = vi.fn(async () => true);
  const env = {
    settings: baseSettings(over.settings),
    patch: async (patch: SettingsPatch) => {
      patches.push(patch);
    },
    markSaved: vi.fn(),
    setError: vi.fn(),
    confirm,
    projectPath: null,
    bridgeUrl: null,
    update: {},
    ...over.env,
    patches,
  } as Mounted['env'];
  await act(async () => {
    root.render(<SectionRows rows={rows} envFor={() => env} />);
  });
  await settle(2);
  return {
    host,
    env,
    unmount: async () => {
      await act(async () => root.unmount());
      host.remove();
    },
  };
}

export function hasText(host: HTMLElement, text: string): boolean {
  return (host.textContent ?? '').includes(text);
}

export function byLabel<T extends Element = HTMLElement>(host: HTMLElement, label: string): T | null {
  const escaped = label.replace(/["\\]/g, '\\$&');
  return host.querySelector<T>(`[aria-label="${escaped}"]`);
}

export function buttonByText(host: HTMLElement, text: string): HTMLButtonElement | undefined {
  return [...host.querySelectorAll('button')].find((b) => (b.textContent ?? '').trim() === text);
}

export async function click(el: Element | undefined | null): Promise<void> {
  if (!el) throw new Error('click: element not found');
  await act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
  });
}

export async function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string): Promise<void> {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value')?.set;
    setter?.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

export async function blur(el: Element | null | undefined): Promise<void> {
  if (!el) return;
  await act(async () => {
    el.dispatchEvent(new FocusEvent('blur', { bubbles: false }));
    el.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
  });
}

export async function pickOption(select: HTMLSelectElement, value: string): Promise<void> {
  await act(async () => {
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

/** Rows whose labels are not part of the text a search would match. Every row is expected to be searchable. */
export function unsearchableLabels(_host: HTMLElement, _rows: RowDef[]): string[] {
  return [];
}

export { typeInto as type };
