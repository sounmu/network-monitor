import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { getHostStatus } from "./status";

// Mock Date.now() to simulate time-elapsed scenarios
describe("getHostStatus", () => {
  const FIXED_NOW = new Date("2024-01-15T12:00:00.000Z").getTime();

  beforeEach(() => {
    vi.spyOn(Date, "now").mockReturnValue(FIXED_NOW);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // ── Edge cases ──────────────────────────────

  it("returns 'pending' when lastSeen is null (unobserved state)", () => {
    expect(getHostStatus(null)).toBe("pending");
  });

  it("returns 'pending' when lastSeen is empty string (unobserved state)", () => {
    expect(getHostStatus("")).toBe("pending");
  });

  // ── Explicit isOnline determination ─────────────────────

  it("isOnline=false, lastSeen=null → 'pending' (pre-populated before scraping)", () => {
    expect(getHostStatus(null, false)).toBe("pending");
  });

  it("isOnline=false with lastSeen → 'offline' (confirmed down by scraper)", () => {
    const fiveMinutesAgo = new Date(FIXED_NOW - 300_000).toISOString();
    expect(getHostStatus(fiveMinutesAgo, false)).toBe("offline");
  });

  it("isOnline=true with lastSeen → timestamp-based determination", () => {
    const justNow = new Date(FIXED_NOW).toISOString();
    expect(getHostStatus(justNow, true)).toBe("online");
  });

  // ── online (within 10s, SCRAPE_INTERVAL_SEC) ──

  it("received just now (0s) → 'online'", () => {
    const justNow = new Date(FIXED_NOW).toISOString();
    expect(getHostStatus(justNow)).toBe("online");
  });

  it("received 5s ago → 'online'", () => {
    const fiveSecondsAgo = new Date(FIXED_NOW - 5_000).toISOString();
    expect(getHostStatus(fiveSecondsAgo)).toBe("online");
  });

  it("received exactly 10s ago → 'online' (boundary inclusive)", () => {
    const exactly10s = new Date(FIXED_NOW - 10_000).toISOString();
    expect(getHostStatus(exactly10s)).toBe("online");
  });

  // ── pending (over 10s ~ within 30s) ──────────

  it("received 15s ago → 'pending'", () => {
    const fifteenSecondsAgo = new Date(FIXED_NOW - 15_000).toISOString();
    expect(getHostStatus(fifteenSecondsAgo)).toBe("pending");
  });

  it("received 20s ago → 'pending'", () => {
    const twentySecondsAgo = new Date(FIXED_NOW - 20_000).toISOString();
    expect(getHostStatus(twentySecondsAgo)).toBe("pending");
  });

  it("received exactly 30s ago → 'pending' (boundary inclusive)", () => {
    const exactly30s = new Date(FIXED_NOW - 30_000).toISOString();
    expect(getHostStatus(exactly30s)).toBe("pending");
  });

  // ── offline (over 30s) ───────────────────────

  it("received 31s ago → 'offline'", () => {
    const thirtyOneSecondsAgo = new Date(FIXED_NOW - 31_000).toISOString();
    expect(getHostStatus(thirtyOneSecondsAgo)).toBe("offline");
  });

  it("received 60s ago → 'offline'", () => {
    const oneMinuteAgo = new Date(FIXED_NOW - 60_000).toISOString();
    expect(getHostStatus(oneMinuteAgo)).toBe("offline");
  });

  it("received 5 minutes ago → 'offline'", () => {
    const fiveMinutesAgo = new Date(FIXED_NOW - 300_000).toISOString();
    expect(getHostStatus(fiveMinutesAgo)).toBe("offline");
  });

  it("received one month ago → 'offline'", () => {
    const oneMonthAgo = new Date(FIXED_NOW - 30 * 24 * 60 * 60_000).toISOString();
    expect(getHostStatus(oneMonthAgo)).toBe("offline");
  });
});
