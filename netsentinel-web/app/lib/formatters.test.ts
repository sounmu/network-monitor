import { describe, expect, it } from "vitest";
import { formatNetworkSpeed, formatNetworkSpeedTick } from "./formatters";

// ──────────────────────────────────────────────
// formatNetworkSpeed
// ──────────────────────────────────────────────

describe("formatNetworkSpeed", () => {
  // ── null/undefined/invalid guard ──────────────

  it("returns '0 KB/s' for null", () => {
    expect(formatNetworkSpeed(null)).toBe("0 KB/s");
  });

  it("returns '0 KB/s' for undefined", () => {
    expect(formatNetworkSpeed(undefined)).toBe("0 KB/s");
  });

  it("returns '0 KB/s' for NaN", () => {
    expect(formatNetworkSpeed(NaN)).toBe("0 KB/s");
  });

  it("returns '0 KB/s' for negative numbers", () => {
    expect(formatNetworkSpeed(-100)).toBe("0 KB/s");
  });

  it("returns '0 KB/s' for Infinity", () => {
    expect(formatNetworkSpeed(Infinity)).toBe("0 KB/s");
  });

  // ── KB/s range ────────────────────────────────

  it("0 bytes/s → '0 KB/s'", () => {
    expect(formatNetworkSpeed(0)).toBe("0 KB/s");
  });

  it("1024 bytes/s (1 KB/s) → '1.0 KB/s'", () => {
    expect(formatNetworkSpeed(1024)).toBe("1.0 KB/s");
  });

  it("102400 bytes/s (100 KB/s) → '100.0 KB/s'", () => {
    expect(formatNetworkSpeed(102400)).toBe("100.0 KB/s");
  });

  // ── MB/s range ────────────────────────────────

  it("1048576 bytes/s (1 MB/s) → '1.00 MB/s'", () => {
    expect(formatNetworkSpeed(1048576)).toBe("1.00 MB/s");
  });

  it("10485760 bytes/s (10 MB/s) → '10.00 MB/s'", () => {
    expect(formatNetworkSpeed(10485760)).toBe("10.00 MB/s");
  });

  // ── GB/s range ────────────────────────────────

  it("1073741824 bytes/s (1 GB/s) → '1.00 GB/s'", () => {
    expect(formatNetworkSpeed(1073741824)).toBe("1.00 GB/s");
  });
});

// ──────────────────────────────────────────────
// formatNetworkSpeedTick (axis-only, with units)
// ──────────────────────────────────────────────

describe("formatNetworkSpeedTick", () => {
  it("returns '0' for null", () => {
    expect(formatNetworkSpeedTick(null)).toBe("0");
  });

  it("0 bytes/s → '0 KB/s' (0 passes the guard and is handled in the KB/s branch)", () => {
    // 0 < 0.05 so `${0 < 0.05 ? "0" : ...} KB/s` = "0 KB/s"
    expect(formatNetworkSpeedTick(0)).toBe("0 KB/s");
  });

  it("1024 bytes/s → '1.0 KB/s'", () => {
    expect(formatNetworkSpeedTick(1024)).toBe("1.0 KB/s");
  });

  it("1048576 bytes/s → '1.0 MB/s' (1 decimal place)", () => {
    // formatNetworkSpeed uses 2 decimal places, Tick uses 1
    expect(formatNetworkSpeedTick(1048576)).toBe("1.0 MB/s");
  });

  it("1073741824 bytes/s → '1.0 GB/s' (1 decimal place)", () => {
    expect(formatNetworkSpeedTick(1073741824)).toBe("1.0 GB/s");
  });
});
