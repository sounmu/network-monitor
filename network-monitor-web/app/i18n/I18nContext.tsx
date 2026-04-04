"use client";

import {
  createContext,
  useCallback,
  useContext,
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
  const [locale, setLocaleState] = useState<Locale>(() => {
    if (typeof window === "undefined") return defaultLocale;
    const saved = localStorage.getItem(STORAGE_KEY) as Locale | null;
    if (saved && locales.includes(saved)) return saved;
    return defaultLocale;
  });

  const setLocale = useCallback((next: Locale) => {
    localStorage.setItem(STORAGE_KEY, next);
    setLocaleState(next);
  }, []);

  return (
    <I18nContext.Provider
      value={{ locale, t: translations[locale], setLocale }}
    >
      {children}
    </I18nContext.Provider>
  );
}

/** Returns the typed translation dictionary and locale helpers. */
export function useI18n() {
  return useContext(I18nContext);
}
