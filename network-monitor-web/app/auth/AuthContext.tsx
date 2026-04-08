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
import { UserInfo, setUserToken, getUserToken, getMe } from "@/app/lib/api";

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

  useEffect(() => {
    let cancelled = false;
    const token = getUserToken();
    if (token) {
      getMe()
        .then((u) => {
          if (!cancelled) setUser(u);
        })
        .catch(() => {
          setUserToken(null);
        })
        .finally(() => {
          if (!cancelled) setIsLoading(false);
        });
    } else {
      // No token — use microtask to avoid synchronous setState in effect body
      queueMicrotask(() => {
        if (!cancelled) setIsLoading(false);
      });
    }
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
    setUserToken(token);
    setUser(userInfo);
  }, []);

  const logout = useCallback(() => {
    setUserToken(null);
    setUser(null);
    router.replace("/login");
  }, [router]);

  const value = useMemo(
    () => ({ user, isLoading, login, logout }),
    [user, isLoading, login, logout],
  );

  if (isLoading) {
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
