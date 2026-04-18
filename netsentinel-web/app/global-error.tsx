"use client";

import { useEffect } from "react";

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  return (
    <html lang="en">
      <body>
        <div style={{ minHeight: "100vh", display: "grid", placeItems: "center", padding: 24 }}>
          <div
            style={{
              maxWidth: 520,
              width: "100%",
              padding: 32,
              borderRadius: 16,
              border: "1px solid #d0d7de",
              background: "#fff",
              textAlign: "center",
            }}
          >
            <h1 style={{ fontSize: 24, marginBottom: 12 }}>Application Error</h1>
            <p style={{ color: "#57606a", marginBottom: 20 }}>
              NetSentinel could not recover from a root-level rendering failure.
            </p>
            <button
              type="button"
              onClick={reset}
              style={{
                padding: "10px 18px",
                borderRadius: 8,
                border: "1px solid #0969da",
                background: "#0969da",
                color: "#fff",
                cursor: "pointer",
              }}
            >
              Retry
            </button>
          </div>
        </div>
      </body>
    </html>
  );
}
