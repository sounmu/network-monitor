"use client";

import Navbar from "./components/Navbar";
import { SSEProvider } from "./lib/sse-context";
import { I18nProvider } from "./i18n/I18nContext";
import { ThemeProvider } from "./theme/ThemeContext";
import { AuthProvider } from "./auth/AuthContext";
import ServiceWorkerRegistration from "./components/ServiceWorkerRegistration";
import ErrorBoundary from "./components/ErrorBoundary";

/**
 * All client-side providers in a single component, so the root `layout.tsx`
 * can stay a Server Component.
 *
 * Previous layout inlined the 5-deep `ThemeProvider → I18nProvider →
 * AuthProvider → SSEProvider → ErrorBoundary → Navbar` tree directly in a
 * file without `"use client"`, which meant Next.js inferred `RootLayout`
 * as a client boundary and every page below it lost the option to be
 * server-rendered. Pulling the tree behind this barrier restores the
 * server/client split — metadata, `<html>`, fonts all stay on the server
 * while the interactive subtree is isolated to this file.
 */
export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider>
      <I18nProvider>
        <AuthProvider>
          <ServiceWorkerRegistration />
          <SSEProvider>
            <ErrorBoundary>
              <div className="app-layout">
                <Navbar />
                {/* `tabIndex={-1}` makes the element programmatically
                    focusable so the "Skip to content" link actually moves
                    focus when followed. Without it, Safari and older
                    Firefox do not move keyboard focus to a div/main that
                    has no native tabindex (WCAG 2.1 G1). */}
                <main id="main-content" tabIndex={-1}>
                  {children}
                </main>
              </div>
            </ErrorBoundary>
          </SSEProvider>
        </AuthProvider>
      </I18nProvider>
    </ThemeProvider>
  );
}
