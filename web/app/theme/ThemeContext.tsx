"use client";

import { createContext, useContext, useState, useEffect, useCallback, useMemo } from "react";

type Theme = "light" | "dark";

interface ThemeContextValue {
  theme: Theme;
  toggleTheme: () => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  theme: "light",
  toggleTheme: () => {},
});

/// Resolve the initial theme from the same sources the inline FOUC
/// killer in `app/layout.tsx` consults — localStorage > system
/// preference > "light". Returning the resolved value from the lazy
/// initializer means React's first render already paints with the
/// correct CSS variables; the inline script keeps this in sync with
/// the `data-theme` attribute so styles apply pre-hydration.
function readInitialTheme(): Theme {
  if (typeof window === "undefined") return "light";
  try {
    const stored = window.localStorage.getItem("theme");
    if (stored === "dark" || stored === "light") return stored;
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  } catch {
    return "light";
  }
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  // Lazy initializer runs once on the client. SSR uses the
  // `typeof window === "undefined"` early-out and lands on "light";
  // hydration converges to the persisted value via the inline FOUC
  // bootstrap script in `app/layout.tsx`, so there is no visible flash.
  const [theme, setTheme] = useState<Theme>(readInitialTheme);

  // Side effects (DOM attribute write + localStorage persist) live in
  // an effect that *reads* `theme`, never inside the `setState` updater.
  // The previous implementation had `localStorage.setItem` and
  // `document.documentElement.setAttribute` *inside* the `setTheme`
  // updater, which violates React 19's purity rule (updaters must be
  // pure functions of prev state). React was free to call the updater
  // twice in dev StrictMode and produce duplicate writes; the new
  // shape side-steps that entirely.
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    try {
      localStorage.setItem("theme", theme);
    } catch {
      // Private mode or quota exhaustion — theme still works for the
      // session, just doesn't persist across reloads.
    }
  }, [theme]);

  const toggleTheme = useCallback(() => {
    setTheme((prev) => (prev === "light" ? "dark" : "light"));
  }, []);

  const value = useMemo(() => ({ theme, toggleTheme }), [theme, toggleTheme]);

  return (
    <ThemeContext.Provider value={value}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme() {
  return useContext(ThemeContext);
}
