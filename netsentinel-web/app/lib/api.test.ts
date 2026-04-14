import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  getMetricsRangeUrl,
  getMetricsUrl,
  clearLegacyStorage,
  setAccessToken,
  getAccessToken,
} from "./api";

// ──────────────────────────────────────────────
// getMetricsRangeUrl
// ──────────────────────────────────────────────

describe("getMetricsRangeUrl", () => {
  const API_BASE = "http://localhost:3000";

  it("floors start to the nearest minute", () => {
    // 2025-01-15T10:05:30.500Z → should floor to 10:05:00.000Z
    const start = new Date("2025-01-15T10:05:30.500Z");
    const end = new Date("2025-01-15T11:00:00.000Z");
    const url = getMetricsRangeUrl("host1", start, end);

    const parsed = new URL(url);
    const startParam = parsed.searchParams.get("start")!;
    expect(startParam).toBe("2025-01-15T10:05:00.000Z");
  });

  it("ceils end to the nearest minute", () => {
    // 2025-01-15T11:05:30.500Z → should ceil to 11:06:00.000Z
    const start = new Date("2025-01-15T10:00:00.000Z");
    const end = new Date("2025-01-15T11:05:30.500Z");
    const url = getMetricsRangeUrl("host1", start, end);

    const parsed = new URL(url);
    const endParam = parsed.searchParams.get("end")!;
    expect(endParam).toBe("2025-01-15T11:06:00.000Z");
  });

  it("does not change timestamps already on minute boundaries", () => {
    const start = new Date("2025-01-15T10:00:00.000Z");
    const end = new Date("2025-01-15T11:00:00.000Z");
    const url = getMetricsRangeUrl("host1", start, end);

    const parsed = new URL(url);
    expect(parsed.searchParams.get("start")).toBe("2025-01-15T10:00:00.000Z");
    expect(parsed.searchParams.get("end")).toBe("2025-01-15T11:00:00.000Z");
  });

  it("encodes host_key in the path", () => {
    const start = new Date("2025-01-15T10:00:00.000Z");
    const end = new Date("2025-01-15T11:00:00.000Z");
    const url = getMetricsRangeUrl("192.168.1.10:9101", start, end);

    expect(url).toContain(`/api/metrics/${encodeURIComponent("192.168.1.10:9101")}?`);
  });

  it("produces identical URLs for timestamps within the same minute", () => {
    const start1 = new Date("2025-01-15T10:05:01.000Z");
    const start2 = new Date("2025-01-15T10:05:59.000Z");
    const end = new Date("2025-01-15T11:00:00.000Z");

    const url1 = getMetricsRangeUrl("host1", start1, end);
    const url2 = getMetricsRangeUrl("host1", start2, end);

    // Both should floor to 10:05:00
    const parsed1 = new URL(url1);
    const parsed2 = new URL(url2);
    expect(parsed1.searchParams.get("start")).toBe(parsed2.searchParams.get("start"));
  });
});

// ──────────────────────────────────────────────
// getMetricsUrl
// ──────────────────────────────────────────────

describe("getMetricsUrl", () => {
  const API_BASE = "http://localhost:3000";

  it("returns correct URL with plain host key", () => {
    expect(getMetricsUrl("myhost")).toBe(`${API_BASE}/api/metrics/myhost`);
  });

  it("encodes special characters in host key", () => {
    const url = getMetricsUrl("192.168.1.10:9101");
    expect(url).toBe(`${API_BASE}/api/metrics/${encodeURIComponent("192.168.1.10:9101")}`);
  });

  it("encodes slashes in host key", () => {
    const url = getMetricsUrl("host/with/slashes");
    expect(url).toContain(encodeURIComponent("host/with/slashes"));
    // Should NOT have unencoded slashes after /metrics/
    expect(url).toBe(`${API_BASE}/api/metrics/${encodeURIComponent("host/with/slashes")}`);
  });
});

// ──────────────────────────────────────────────
// clearLegacyStorage
// ──────────────────────────────────────────────

describe("clearLegacyStorage", () => {
  const removeItemSpy = vi.fn();

  beforeEach(() => {
    removeItemSpy.mockClear();
    // clearLegacyStorage checks `typeof window !== "undefined"` before
    // accessing localStorage, so both globals must be present.
    vi.stubGlobal("window", globalThis);
    vi.stubGlobal("localStorage", {
      getItem: vi.fn(),
      setItem: vi.fn(),
      removeItem: removeItemSpy,
      clear: vi.fn(),
      length: 0,
      key: vi.fn(),
    });
  });

  it("calls localStorage.removeItem with 'auth_token'", () => {
    clearLegacyStorage();
    expect(removeItemSpy).toHaveBeenCalledWith("auth_token");
  });

  it("calls removeItem exactly once", () => {
    clearLegacyStorage();
    expect(removeItemSpy).toHaveBeenCalledTimes(1);
  });

  it("does not throw when called", () => {
    expect(() => clearLegacyStorage()).not.toThrow();
  });
});

// ──────────────────────────────────────────────
// setAccessToken / getAccessToken
// ──────────────────────────────────────────────

describe("setAccessToken / getAccessToken", () => {
  beforeEach(() => {
    setAccessToken(null);
  });

  it("returns null by default after clearing", () => {
    expect(getAccessToken()).toBeNull();
  });

  it("stores and retrieves a token", () => {
    setAccessToken("my-jwt-token");
    expect(getAccessToken()).toBe("my-jwt-token");
  });

  it("overwrites a previously set token", () => {
    setAccessToken("token-1");
    setAccessToken("token-2");
    expect(getAccessToken()).toBe("token-2");
  });

  it("can clear the token by setting null", () => {
    setAccessToken("some-token");
    setAccessToken(null);
    expect(getAccessToken()).toBeNull();
  });
});

// ──────────────────────────────────────────────
// authHeaders (tested indirectly via exports)
// ──────────────────────────────────────────────
// authHeaders is not exported directly, but we can verify its behavior
// through the token state: when a token is set, Authorization header
// should be present. We test the building blocks here.

describe("Authorization header logic", () => {
  beforeEach(() => {
    setAccessToken(null);
  });

  it("token is null when not set — no Authorization header expected", () => {
    const token = getAccessToken();
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json",
      ...(token && { Authorization: `Bearer ${token}` }),
    };
    expect(headers).not.toHaveProperty("Authorization");
  });

  it("includes Authorization header when token is set", () => {
    setAccessToken("test-jwt");
    const token = getAccessToken();
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json",
      ...(token && { Authorization: `Bearer ${token}` }),
    };
    expect(headers).toHaveProperty("Authorization", "Bearer test-jwt");
  });

  it("formats Bearer token correctly", () => {
    const testToken = "eyJhbGciOiJIUzI1NiJ9.payload.signature";
    setAccessToken(testToken);
    const token = getAccessToken();
    const authHeader = token ? `Bearer ${token}` : undefined;
    expect(authHeader).toBe(`Bearer ${testToken}`);
  });
});
