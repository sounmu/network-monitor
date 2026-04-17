"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { defaultLocale, Locale, locales, translations, Translations } from "./translations";

const STORAGE_KEY = "nm-locale";

interface I18nContextValue {
  locale: Locale;
  t: Translations;
  setLocale: (locale: Locale) => void;
}

const I18nContext = createContext<I18nContextValue>({
  locale: defaultLocale,
  t: translations[defaultLocale],
  setLocale: () => {},
});

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(defaultLocale);

  // Read stored locale on mount to avoid SSR hydration mismatch.
  // localStorage is browser-only — initialising state after hydration is
  // exactly the shape react-hooks/set-state-in-effect flags. Suppressed.
  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    const saved = localStorage.getItem(STORAGE_KEY) as Locale | null;
    if (saved && locales.includes(saved)) {
      setLocaleState(saved);
    }
  }, []);
  /* eslint-enable react-hooks/set-state-in-effect */

  const setLocale = useCallback((next: Locale) => {
    localStorage.setItem(STORAGE_KEY, next);
    setLocaleState(next);
  }, []);

  // Keep <html lang> in sync with the active locale. Screen readers and
  // browser translation features key off this attribute; leaving it as the
  // SSR-baked "en" for Korean users mispronounces names and offers to
  // translate the page they are already reading. The root layout renders
  // lang="en" as the SSR default (necessary — locale is client-side only);
  // this effect upgrades it on the client once React hydrates.
  useEffect(() => {
    if (typeof document !== "undefined") {
      document.documentElement.lang = locale;
    }
  }, [locale]);

  const value = useMemo(
    () => ({ locale, t: translations[locale], setLocale }),
    [locale, setLocale],
  );

  return (
    <I18nContext.Provider value={value}>
      {children}
    </I18nContext.Provider>
  );
}

/** Returns the typed translation dictionary and locale helpers. */
export function useI18n() {
  return useContext(I18nContext);
}
