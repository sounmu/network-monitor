"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import useSWR from "swr";
import { Shield } from "lucide-react";
import { toast } from "sonner";
import { useAuth } from "@/app/auth/AuthContext";
import { useI18n } from "@/app/i18n/I18nContext";
import {
  login as apiLogin,
  ApiError,
  AuthStatus,
  getAuthStatusUrl,
  fetcher,
} from "@/app/lib/api";

export default function LoginPage() {
  const auth = useAuth();
  const { t } = useI18n();
  const router = useRouter();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [loading, setLoading] = useState(false);

  const { data: authStatus } = useSWR<AuthStatus>(getAuthStatusUrl(), fetcher);

  // Redirect to setup if no users exist
  useEffect(() => {
    if (authStatus?.setup_required) {
      router.replace("/setup");
    }
  }, [authStatus, router]);

  // Already logged in
  useEffect(() => {
    if (auth.user) {
      router.replace("/");
    }
  }, [auth.user, router]);

  if (authStatus?.setup_required || auth.user) {
    return null;
  }

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();

    if (!username.trim()) {
      toast.error(t.auth.usernameRequired);
      return;
    }

    setLoading(true);
    try {
      const response = await apiLogin(username, password);
      auth.login(response.token, response.user);
      router.replace("/");
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 401) {
          toast.error(t.auth.loginError.invalid);
        } else if (err.status === 429) {
          toast.error(t.auth.loginError.rateLimit);
        } else {
          toast.error(t.auth.loginError.generic);
        }
      } else if (err instanceof TypeError) {
        // fetch throws TypeError on network failure (DNS, CORS, offline)
        toast.error(t.auth.loginError.network);
      } else {
        toast.error(t.auth.loginError.generic);
      }
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
            marginBottom: 24,
            justifyContent: "center",
          }}
        >
          <Shield size={28} style={{ color: "var(--accent-blue)" }} />
          <h1 style={{ color: "var(--text-primary)", fontSize: 24, margin: 0 }}>
            {t.auth.login}
          </h1>
        </div>

        <form onSubmit={handleSubmit}>
          <div style={{ marginBottom: 16 }}>
            <label
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
              className="date-input"
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              style={{ width: "100%", boxSizing: "border-box" }}
              autoFocus
            />
          </div>

          <div style={{ marginBottom: 24 }}>
            <label
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
              className="date-input"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              style={{ width: "100%", boxSizing: "border-box" }}
            />
          </div>

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
            {loading ? "..." : t.auth.loginButton}
          </button>
        </form>
      </div>
    </div>
  );
}
