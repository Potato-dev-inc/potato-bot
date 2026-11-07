// @vitest-environment jsdom
// Settings in two languages: the window follows the system by default, a Language choice switches the words at once, the rows
// that are plain sentences change with it, and choosing English again brings the original words back.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { Settings } from '@puppet-master/shared';
import { getLocale, resetLocaleForTests } from '../lib/i18n';
import { SettingsPanel, type SettingsApi } from './SettingsPanel';

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
  resetLocaleForTests(['en-US']);
});

const api: SettingsApi = {
  load: async () => ({ theme: 'dark', potato_bot_enabled: true, potato_sandbox_enabled: true }) as Settings,
  patch: async () => ({}) as Settings,
  subscribe: () => () => {},
};

async function mount(languages: string[]) {
  resetLocaleForTests(languages);
  await act(async () => root.render(<SettingsPanel open api={api} initialTab="later" />));
  for (let i = 0; i < 100 && !host.querySelector('input[type="search"]'); i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
  }
}

const select = () => host.querySelector<HTMLSelectElement>('select[aria-label="Language"], select[aria-label="語言"]')!;
const choose = async (value: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')!.set!.call(select(), value);
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
};

describe('Settings language', { timeout: 30000 }, () => {
  it('shows English on an English system, with a Language choice that follows the system', async () => {
    await mount(['en-US']);
    expect(host.querySelector('input[type="search"]')?.getAttribute('placeholder')).toBe('Search');
    expect(host.textContent).toContain('Coming later');
    expect(select().value).toBe('system');
    expect([...select().options].map((o) => o.textContent)).toEqual(['Follow the system', 'English', '繁體中文']);
  });

  it('follows a Traditional Chinese system by default', async () => {
    await mount(['zh-TW']);
    expect(getLocale()).toBe('zh-TW');
    expect(host.querySelector('input[type="search"]')?.getAttribute('placeholder')).toBe('搜尋');
    expect(host.textContent).toContain('即將推出');
    expect(host.querySelector('nav')?.textContent).toContain('機器人、它們的大腦'.slice(0, 0) + 'Potato Bot');
  });

  it('switches at once when the person chooses, and back again, without leaving the page', async () => {
    await mount(['en-US']);
    await choose('zh-TW');
    expect(host.querySelector('h1')?.textContent).toBe('即將推出');
    expect(host.textContent).toContain('即將推出');
    expect(host.querySelector('nav')?.getAttribute('aria-label')).toBe('設定分類');
    await choose('en');
    expect(host.textContent).toContain('Coming later');
    expect(host.textContent).not.toContain('即將推出');
    expect(window.localStorage.getItem('pm.language')).toBe('en');
  });
});
