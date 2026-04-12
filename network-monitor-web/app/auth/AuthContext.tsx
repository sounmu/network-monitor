"use client";

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  useEffect,
  type ReactNode,
} from "react";
import { usePathname, useRouter } from "next/navigation";
import {
  UserInfo,
  setAccessToken,
  getAccessToken,
  clearLegacyStorage,
  serverLogout,
  tryRefreshSession,
  getMe,
} from "@/app/lib/api";

interface AuthContextValue {
  user: UserInfo | null;
  isLoading: boolean;
  login: (token: string, user: UserInfo) => void;
  logout: () => void;
}

const AuthContext = createContext<AuthContextValue | null>(null);

const PUBLIC_PATHS = ["/login", "/setup", "/status"];

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<UserInfo | null>(null);
  // Start as true on both server and client to avoid hydration mismatch.
  // Resolved to false either synchronously (no token) or after getMe() completes.
  const [isLoading, setIsLoading] = useState(true);
  const pathname = usePathname();
  const router = useRouter();

  // On mount: attempt session restoration. Priority order:
  //   1. In-memory access token (tab-reuse — already present if we didn't
  //      unload). Validate it with /api/auth/me.
  //   2. httpOnly refresh cookie (page reload / new tab). Call
  //      /api/auth/refresh to mint a fresh access token.
  //   3. Neither → unauthenticated.
  useEffect(() => {
    let cancelled = false;
    clearLegacyStorage();

    const init = async () => {
      // Path 1: memory token exists — validate server-side.
      if (getAccessToken()) {
        try {
          const u = await getMe();
          if (!cancelled) setUser(u);
        } catch {
          setAccessToken(null);
        }
        if (!cancelled) setIsLoading(false);
        return;
      }

      // Path 2: no memory token — try to refresh via httpOnly cookie.
      const result = await tryRefreshSession();
      if (!cancelled) {
        if (result) {
          setUser(result.user);
        }
        setIsLoading(false);
      }
    };

    void init();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (!isLoading && !user) {
      const isPublic = PUBLIC_PATHS.some(
        (p) => pathname === p || pathname.startsWith(p + "/"),
      );
      if (!isPublic) {
        router.replace("/login");
      }
    }
  }, [isLoading, user, pathname, router]);

  const login = useCallback((token: string, userInfo: UserInfo) => {
    setAccessToken(token);
    setUser(userInfo);
  }, []);

  const logout = useCallback(() => {
    // Fire-and-forget server revoke so the token + refresh cookie are
    // killed on the backend. `serverLogout` never throws.
    void serverLogout().finally(() => {
      setAccessToken(null);
      setUser(null);
      router.replace("/login");
    });
  }, [router]);

  const value = useMemo(
    () => ({ user, isLoading, login, logout }),
    [user, isLoading, login, logout],
  );

  if (isLoading) {
    return null;
  }

  // Don't render protected children when unauthenticated. Without this
  // guard, SWR hooks in the child tree mount for a single frame before
  // the redirect useEffect fires, fire unauthenticated fetches that
  // return 401, trigger handleUnauthorized → hard reload, and loop.
  const isPublic = PUBLIC_PATHS.some(
    (p) => pathname === p || pathname.startsWith(p + "/"),
  );
  if (!user && !isPublic) {
    return null;
  }

  return (
    <AuthContext.Provider value={value}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return ctx;
}
