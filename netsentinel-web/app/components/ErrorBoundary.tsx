"use client";

import React from "react";
import { useI18n } from "@/app/i18n/I18nContext";

interface ErrorBoundaryProps {
  children: React.ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

/** Functional fallback — isolated so it can use hooks (`useI18n`) while the
 *  class boundary below owns the React error-handling lifecycle. */
function ErrorFallback({
  error,
  onReload,
}: {
  error: Error | null;
  onReload: () => void;
}) {
  const { t } = useI18n();
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        minHeight: "60vh",
        padding: 40,
        color: "var(--text-primary)",
      }}
    >
      <div style={{ fontSize: 48, marginBottom: 16, opacity: 0.3 }}>:(</div>
      <h2 style={{ fontSize: 18, fontWeight: 700, marginBottom: 8 }}>
        {t.errorBoundary.title}
      </h2>
      <p
        style={{
          fontSize: 13,
          color: "var(--text-muted)",
          marginBottom: 20,
          textAlign: "center",
          maxWidth: 400,
        }}
      >
        {error?.message || t.errorBoundary.fallbackMessage}
      </p>
      <button
        onClick={onReload}
        style={{
          padding: "8px 20px",
          borderRadius: 8,
          border: "1px solid var(--accent-blue)",
          background: "var(--accent-blue)",
          color: "var(--text-on-accent, #fff)",
          fontSize: 13,
          fontWeight: 600,
          cursor: "pointer",
        }}
      >
        {t.errorBoundary.reload}
      </button>
    </div>
  );
}

export default class ErrorBoundary extends React.Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[ErrorBoundary]", error, info.componentStack);
  }

  private handleReload = () => {
    this.setState({ hasError: false, error: null });
    window.location.reload();
  };

  render() {
    if (this.state.hasError) {
      return (
        <ErrorFallback error={this.state.error} onReload={this.handleReload} />
      );
    }
    return this.props.children;
  }
}
