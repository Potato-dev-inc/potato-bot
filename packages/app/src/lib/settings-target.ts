// Where a Settings deep link lands. Everything now belongs to the Potato Bot page.
import type { SettingsPageId } from '../components/settings/model';
import { DEFAULT_SETTINGS_TAB, resolveSettingsTab } from './settings-entry';

export interface SettingsTarget {
  page: SettingsPageId;
  row?: string;
}

export function resolveSettingsTarget(tab: string | null | undefined): SettingsTarget | null {
  const page = resolveSettingsTab(tab);
  return page ? { page } : null;
}

export function settingsEntryTarget(search: string): SettingsTarget {
  const q = new URLSearchParams(search);
  return resolveSettingsTarget(q.get('tab')) ?? { page: DEFAULT_SETTINGS_TAB };
}
