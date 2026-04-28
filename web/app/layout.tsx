import type { Metadata, Viewport } from "next";
import Script from "next/script";
import { Inter, JetBrains_Mono } from "next/font/google";
import "./globals.css";
import { Toaster } from "sonner";
import { Providers } from "./providers";

/// Inline FOUC-killer for the dark/light theme. Runs synchronously in
/// `<head>` *before* hydration so the first paint already carries the
/// correct `data-theme` attribute — without this, `ThemeContext` reads
/// `localStorage` only after the client hydrates and dark-mode users
/// see a brief flash of the default light palette. The expression is
/// kept tiny on purpose; it ships in every HTML page from the static
/// export, so size matters more than readability.
const THEME_BOOTSTRAP = `(function(){try{var t=localStorage.getItem('theme');if(t!=='dark'&&t!=='light'){t=window.matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';}document.documentElement.setAttribute('data-theme',t);}catch(_){}})();`;

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
  title: "NetSentinel — Infrastructure Dashboard",
  description: "Real-time server infrastructure monitoring dashboard",
  appleWebApp: {
    capable: true,
    statusBarStyle: "black-translucent",
    title: "NetSentinel",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" data-scroll-behavior="smooth" className={`${inter.variable} ${jetbrainsMono.variable}`}>
      <head>
        {/* `beforeInteractive` ensures the snippet runs before React hydrates,
            so the `data-theme` attribute is set before the first paint and
            no light-flash occurs on dark-mode reloads. */}
        <Script
          id="theme-bootstrap"
          strategy="beforeInteractive"
          dangerouslySetInnerHTML={{ __html: THEME_BOOTSTRAP }}
        />
      </head>
      <body>
        <a href="#main-content" className="skip-to-content">Skip to content</a>
        <Toaster position="top-right" theme="system" richColors duration={4000} />
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
