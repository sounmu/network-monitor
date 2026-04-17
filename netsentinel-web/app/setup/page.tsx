"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import useSWR from "swr";
import { Shield } from "lucide-react";
import { useAuth } from "@/app/auth/AuthContext";
import { useI18n } from "@/app/i18n/I18nContext";
import {
  setupAdmin,
  AuthStatus,
  getAuthStatusUrl,
  fetcher,
} from "@/app/lib/api";

export default function SetupPage() {
  const auth = useAuth();
  const { t } = useI18n();
  const router = useRouter();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const { data: authStatus } = useSWR<AuthStatus>(getAuthStatusUrl(), fetcher);

  // If setup is not required (users already exist), redirect to login
  useEffect(() => {
    if (authStatus && !authStatus.setup_required) {
      router.replace("/login");
    }
  }, [authStatus, router]);

  if (authStatus && !authStatus.setup_required) {
    return null;
  }

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setError("");

    if (!username.trim()) {
      setError(t.auth.usernameRequired);
      return;
    }

    if (password.length < 8 || password.length > 128) {
      setError(t.auth.passwordTooShort);
      return;
    }

    const hasUpper = /[A-Z]/.test(password);
    const hasLower = /[a-z]/.test(password);
    const hasDigit = /\d/.test(password);
    const hasSpecial = /[^A-Za-z0-9]/.test(password);
    if (!(hasUpper && hasLower && hasDigit && hasSpecial)) {
      setError(t.auth.passwordPolicy);
      return;
    }

    if (password !== confirmPassword) {
      setError(t.auth.passwordMismatch);
      return;
    }

    setLoading(true);
    try {
      const response = await setupAdmin(username, password);
      auth.login(response.token, response.user);
      router.replace("/");
    } catch (err) {
      setError(err instanceof Error ? err.message : t.auth.setupFailed);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      style={{
        display: "flex",
        justifyContent: "center",
        alignItems: "center",
        minHeight: "100vh",
      }}
    >
      <div className="glass-card" style={{ maxWidth: 400, width: "100%", padding: 32 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            marginBottom: 12,
            justifyContent: "center",
          }}
        >
          <Shield size={28} style={{ color: "var(--accent-blue)" }} />
          <h1 style={{ color: "var(--text-primary)", fontSize: 24, margin: 0 }}>
            {t.auth.setupTitle}
          </h1>
        </div>

        <p
          style={{
            color: "var(--text-muted)",
            fontSize: 14,
            textAlign: "center",
            marginBottom: 24,
          }}
        >
          {t.auth.setupDescription}
        </p>

        <form onSubmit={handleSubmit}>
          <div style={{ marginBottom: 16 }}>
            <label
              htmlFor="setup-username"
              style={{
                display: "block",
                color: "var(--text-muted)",
                marginBottom: 6,
                fontSize: 14,
              }}
            >
              {t.auth.username}
            </label>
            <input
              id="setup-username"
              className="date-input"
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              style={{ width: "100%", boxSizing: "border-box" }}
              autoFocus
            />
          </div>

          <div style={{ marginBottom: 16 }}>
            <label
              htmlFor="setup-password"
              style={{
                display: "block",
                color: "var(--text-muted)",
                marginBottom: 6,
                fontSize: 14,
              }}
            >
              {t.auth.password}
            </label>
            <input
              id="setup-password"
              className="date-input"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              style={{ width: "100%", boxSizing: "border-box" }}
            />
          </div>

          <div style={{ marginBottom: 24 }}>
            <label
              htmlFor="setup-confirm"
              style={{
                display: "block",
                color: "var(--text-muted)",
                marginBottom: 6,
                fontSize: 14,
              }}
            >
              {t.auth.confirmPassword}
            </label>
            <input
              id="setup-confirm"
              className="date-input"
              type="password"
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              style={{ width: "100%", boxSizing: "border-box" }}
            />
          </div>

          {error && (
            <p
              style={{
                color: "var(--status-red, #ef4444)",
                fontSize: 14,
                marginBottom: 16,
                textAlign: "center",
              }}
            >
              {error}
            </p>
          )}

          <button
            type="submit"
            disabled={loading}
            style={{
              width: "100%",
              padding: "10px 16px",
              backgroundColor: "var(--accent-blue)",
              color: "#fff",
              border: "none",
              borderRadius: 6,
              fontSize: 15,
              fontWeight: 600,
              cursor: loading ? "not-allowed" : "pointer",
              opacity: loading ? 0.7 : 1,
            }}
          >
            {loading ? "..." : t.auth.setupButton}
          </button>
        </form>
      </div>
    </div>
  );
}
