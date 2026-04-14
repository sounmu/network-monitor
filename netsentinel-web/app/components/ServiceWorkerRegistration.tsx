"use client";

import { useEffect } from "react";
import { useAuth } from "@/app/auth/AuthContext";

export default function ServiceWorkerRegistration() {
  const { user } = useAuth();

  useEffect(() => {
    if (!user) return;

    // Dynamically add manifest link after authentication
    // (avoids Cloudflare Access intercepting the request before login)
    if (!document.querySelector('link[rel="manifest"]')) {
      const link = document.createElement("link");
      link.rel = "manifest";
      link.href = "/manifest.json";
      link.crossOrigin = "use-credentials";
      document.head.appendChild(link);
    }

    // Register service worker
    if ("serviceWorker" in navigator) {
      navigator.serviceWorker.register("/sw.js").catch(() => {
        // Service worker registration failed — non-critical, ignore silently
      });
    }
  }, [user]);

  return null;
}
