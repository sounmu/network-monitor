"use client";

import {
  createContext,
  useContext,
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
  const [isLoading, setIsLoading] = useState(() => {
    if (typeof window === "undefined") return true;
    return !!getUserToken();
  });
  const pathname = usePathname();
  const router = useRouter();

  useEffect(() => {
    const token = getUserToken();
    if (token) {
      getMe()
        .then((u) => {
          setUser(u);
        })
        .catch(() => {
          setUserToken(null);
        })
        .finally(() => {
          setIsLoading(false);
        });
    }
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

  const login = (token: string, userInfo: UserInfo) => {
    setUserToken(token);
    setUser(userInfo);
  };

  const logout = () => {
    setUserToken(null);
    setUser(null);
    router.replace("/login");
  };

  if (isLoading) {
    return null;
  }

  return (
    <AuthContext.Provider value={{ user, isLoading, login, logout }}>
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
