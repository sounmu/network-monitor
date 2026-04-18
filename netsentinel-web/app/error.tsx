"use client";

import { useEffect } from "react";

export default function ErrorPage({
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
    <div className="glass-card" style={{ padding: "48px 24px", textAlign: "center" }}>
      <h2 style={{ fontSize: 20, marginBottom: 12 }}>Something went wrong</h2>
      <p style={{ color: "var(--text-muted)", marginBottom: 20 }}>
        The dashboard hit an unexpected error while rendering this route.
      </p>
      <button
        type="button"
        onClick={reset}
        style={{
          padding: "10px 18px",
          borderRadius: 8,
          border: "1px solid var(--accent-blue)",
          background: "var(--accent-blue)",
          color: "var(--text-on-accent, #fff)",
          cursor: "pointer",
        }}
      >
        Try again
      </button>
    </div>
  );
}
