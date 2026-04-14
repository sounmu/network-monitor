import type { Metadata, Viewport } from "next";
import { Inter, JetBrains_Mono } from "next/font/google";
import "./globals.css";
import SidebarShell from "./components/SidebarShell";
import { SSEProvider } from "./lib/sse-context";
import { I18nProvider } from "./i18n/I18nContext";
import { ThemeProvider } from "./theme/ThemeContext";
import { AuthProvider } from "./auth/AuthContext";
import ServiceWorkerRegistration from "./components/ServiceWorkerRegistration";
import ErrorBoundary from "./components/ErrorBoundary";
import { Toaster } from "sonner";

const inter = Inter({
  subsets: ["latin"],
  weight: ["300", "400", "500", "600", "700", "800"],
  variable: "--font-inter",
  display: "swap",
});

const jetbrainsMono = JetBrains_Mono({
  subsets: ["latin"],
  weight: ["400", "500"],
  variable: "--font-mono",
  display: "swap",
});

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  themeColor: "#3B82F6",
};

export const metadata: Metadata = {
  title: "NetMonitor — Infrastructure Dashboard",
  description: "Real-time server infrastructure monitoring dashboard",
  appleWebApp: {
    capable: true,
    statusBarStyle: "black-translucent",
    title: "NetMonitor",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" data-scroll-behavior="smooth" className={`${inter.variable} ${jetbrainsMono.variable}`}>
      <body>
        <a href="#main-content" className="skip-to-content">Skip to content</a>
        <Toaster position="top-right" theme="system" richColors duration={4000} />
        <ThemeProvider>
        <I18nProvider>
        <AuthProvider>
          <ServiceWorkerRegistration />
          <SSEProvider>
            <ErrorBoundary>
            <div className="dashboard-grid">
              <SidebarShell />
              <main
                id="main-content"
                style={{
                  overflowY: "auto",
                  minHeight: "100vh",
                  background: "var(--bg-primary)",
                }}
              >
                {children}
              </main>
            </div>
            </ErrorBoundary>
          </SSEProvider>
        </AuthProvider>
        </I18nProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
