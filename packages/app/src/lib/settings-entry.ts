// Entry route of the standalone Settings window (`index.html?settings=1&tab=potato`).
import { SECTION_IDS, type SettingsPageId } from '../components/settings/model';

export const DEFAULT_SETTINGS_TAB: SettingsPageId = 'potato';

export function isSettingsEntry(search: string): boolean {
  return new URLSearchParams(search).has('settings');
}

/** A page id from a section id; anything else opens Potato Bot. */
export function resolveSettingsTab(tab: string | null | undefined): SettingsPageId | null {
  if (!tab) return null;
  if ((SECTION_IDS as readonly string[]).includes(tab)) return tab as SettingsPageId;
  return null;
}

/** The page named by `?tab=`, or Potato Bot when absent or unknown. */
export function settingsEntryTab(search: string): SettingsPageId {
  return resolveSettingsTab(new URLSearchParams(search).get('tab')) ?? DEFAULT_SETTINGS_TAB;
}
