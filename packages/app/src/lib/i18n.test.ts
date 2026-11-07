// @vitest-environment jsdom
// Languages: the system language picks the default, the choice is remembered, a sentence without a translation stays English,
// and the Traditional Chinese dictionary keeps every placeholder of its English sentence.
import { afterEach, describe, expect, it } from 'vitest';
import { LANGUAGE_KEY, getLocale, getLocalePref, localized, resetLocaleForTests, resolveLocale, setLocalePref, systemLocale, t } from './i18n';
import { ZH_TW } from '../locales/zh-TW';

afterEach(() => resetLocaleForTests(['en-US']));

describe('which language is used', () => {
  it('follows the system: Traditional Chinese for zh-TW, zh-HK, zh-MO and zh-Hant, English for the rest', () => {
    expect(systemLocale(['zh-TW'])).toBe('zh-TW');
    expect(systemLocale(['zh-HK', 'en'])).toBe('zh-TW');
    expect(systemLocale(['zh-Hant-TW'])).toBe('zh-TW');
    expect(systemLocale(['en-US', 'zh-TW'])).toBe('en');
    expect(systemLocale(['zh-CN'])).toBe('en');
    expect(systemLocale(['fr-FR'])).toBe('en');
    expect(systemLocale([])).toBe('en');
  });

  it('a chosen language beats the system, and "system" goes back to following it', () => {
    expect(resolveLocale('en', ['zh-TW'])).toBe('en');
    expect(resolveLocale('zh-TW', ['en-US'])).toBe('zh-TW');
    expect(resolveLocale('system', ['zh-TW'])).toBe('zh-TW');
  });

  it('remembers the choice in storage and forgets it when set back to the system', () => {
    setLocalePref('zh-TW');
    expect(window.localStorage.getItem(LANGUAGE_KEY)).toBe('zh-TW');
    expect(getLocale()).toBe('zh-TW');
    expect(getLocalePref()).toBe('zh-TW');
    expect(document.documentElement.lang).toBe('zh-Hant-TW');
    setLocalePref('system');
    expect(window.localStorage.getItem(LANGUAGE_KEY)).toBeNull();
  });
});

describe('t', () => {
  it('gives the Traditional Chinese sentence, and the English one when there is none or the language is English', () => {
    expect(t('Daily limit')).toBe('Daily limit');
    setLocalePref('zh-TW');
    expect(t('Language')).toBe('語言');
    expect(t('A sentence nobody has translated yet')).toBe('A sentence nobody has translated yet');
    setLocalePref('en');
    expect(t('Language')).toBe('Language');
  });

  it('fills the changing parts of a sentence in either language', () => {
    expect(t('Saved to {path}', { path: 'C:\\x' })).toBe('Saved to C:\\x');
    setLocalePref('zh-TW');
    expect(t('No settings match “{query}”.', { query: 'tea' })).toBe('找不到符合「tea」的設定。');
    expect(t('{missing} stays', {})).toBe('{missing} stays');
  });

  it('reads a COPY object through the dictionary each time it is read', () => {
    const copy = localized({ title: 'Settings', count: (n: number) => `${n} bots` });
    expect(copy.title).toBe('Settings');
    setLocalePref('zh-TW');
    expect(copy.title).toBe('設定');
    expect(copy.count(3)).toBe('3 bots');
  });
});

describe('the Traditional Chinese dictionary', () => {
  const holes = (text: string) => (text.match(/\{\w+\}/g) ?? []).sort().join(',');

  it('has no empty translation, keeps every placeholder, and does not repeat the English as its own translation by accident', () => {
    for (const [english, chinese] of Object.entries(ZH_TW)) {
      expect(chinese.trim(), english).not.toBe('');
      expect(holes(chinese), english).toBe(holes(english));
      expect(chinese, english).not.toMatch(/[\r\n]\s*$/);
    }
  });

  it('is written in Traditional characters, not Simplified', () => {
    const simplified = /[设这们个为过还时现发进与会学开关应当对说话谁钟气点样书车马们国]/;
    for (const [english, chinese] of Object.entries(ZH_TW)) expect(chinese, english).not.toMatch(simplified);
  });
});
