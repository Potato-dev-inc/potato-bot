// Languages: the app's own words in English or Traditional Chinese. The person picks one (or "follow the system", the default).
//
// How it works: the English sentence IS the key. `t('Daily limit')` gives the Traditional Chinese for it when the language is
// Traditional Chinese and a translation exists, and the English sentence in every other case, so a sentence that has no translation
// yet is never blank or broken. A sentence with a changing part keeps it in braces: `t('Saved to {path}', { path })`.
// The dictionary is `src/locales/zh-TW.ts`. Screens that keep their words in a COPY object wrap it once with `localized(...)`.
//
// The choice is kept in this browser's storage under `pm.language` (the cockpit and the Settings window share it, and a change in
// one reaches the other through the storage event). Reading and writing it never throws: without storage the language follows the system.
import { useEffect, useState } from 'react';
import { ZH_TW } from '../locales/zh-TW';

export type Locale = 'en' | 'zh-TW';
export type LocalePref = 'system' | Locale;

export const LOCALES: readonly { id: Locale; label: string }[] = [
  { id: 'en', label: 'English' },
  { id: 'zh-TW', label: '繁體中文' },
];
export const LANGUAGE_KEY = 'pm.language';

const DICTIONARIES: Record<Locale, Readonly<Record<string, string>>> = { en: {}, 'zh-TW': ZH_TW };

/** The language the system asks for: Traditional Chinese for zh-TW, zh-HK, zh-MO and zh-Hant; English for everything else. */
export function systemLocale(languages: readonly string[] | undefined = typeof navigator === 'undefined' ? undefined : navigator.languages?.length ? navigator.languages : [navigator.language]): Locale {
  for (const raw of languages ?? []) {
    const tag = raw.toLowerCase();
    if (tag === 'zh-tw' || tag === 'zh-hk' || tag === 'zh-mo' || tag.startsWith('zh-hant')) return 'zh-TW';
    if (tag.startsWith('en')) return 'en';
    if (tag.startsWith('zh')) return 'en';
  }
  return 'en';
}

export function resolveLocale(pref: LocalePref, languages?: readonly string[]): Locale {
  return pref === 'system' ? systemLocale(languages) : pref;
}

function readPref(): LocalePref {
  try {
    const value = window.localStorage.getItem(LANGUAGE_KEY);
    return value === 'en' || value === 'zh-TW' ? value : 'system';
  } catch {
    return 'system';
  }
}

let pref: LocalePref = readPref();
let locale: Locale = resolveLocale(pref);
const listeners = new Set<() => void>();

function apply(next: LocalePref) {
  pref = next;
  locale = resolveLocale(next);
  try {
    document.documentElement.lang = locale === 'zh-TW' ? 'zh-Hant-TW' : 'en';
  } catch {
    /* no document */
  }
  for (const listener of [...listeners]) listener();
}

export function getLocale(): Locale {
  return locale;
}
export function getLocalePref(): LocalePref {
  return pref;
}

/** Changes the language ("system" follows the computer) and remembers it. */
export function setLocalePref(next: LocalePref) {
  try {
    if (next === 'system') window.localStorage.removeItem(LANGUAGE_KEY);
    else window.localStorage.setItem(LANGUAGE_KEY, next);
  } catch {
    /* kept for this run only */
  }
  apply(next);
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', (event) => {
    if (event.key === LANGUAGE_KEY || event.key === null) apply(readPref());
  });
  try {
    document.documentElement.lang = locale === 'zh-TW' ? 'zh-Hant-TW' : 'en';
  } catch {
    /* no document */
  }
}

function fill(text: string, vars?: Record<string, string | number>): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole));
}

/** The sentence in the current language. Unknown sentences come back as they are. */
export function t(text: string, vars?: Record<string, string | number>): string {
  const found = locale === 'en' ? undefined : DICTIONARIES[locale][text];
  return fill(found ?? text, vars);
}

/** Wraps a COPY object so that every plain-sentence property is read through `t`. Functions and nested values are left as they are. */
export function localized<T extends object>(copy: T): T {
  return new Proxy(copy, {
    get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver) as unknown;
      return typeof value === 'string' ? t(value) : value;
    },
  });
}

/** Re-renders the caller when the language changes. Returns the language in use and the person's choice. */
export function useLocale(): { locale: Locale; pref: LocalePref; setPref: (next: LocalePref) => void } {
  const [, bump] = useState(0);
  useEffect(() => {
    const listener = () => bump((n) => n + 1);
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);
  return { locale, pref, setPref: setLocalePref };
}

/** For tests: forget the saved choice and follow the given system languages. */
export function resetLocaleForTests(languages?: readonly string[]) {
  try {
    window.localStorage.removeItem(LANGUAGE_KEY);
  } catch {
    /* no storage */
  }
  pref = 'system';
  locale = resolveLocale('system', languages ?? ['en-US']);
  for (const listener of [...listeners]) listener();
}
