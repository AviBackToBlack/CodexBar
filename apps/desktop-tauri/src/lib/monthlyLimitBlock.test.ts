import { describe, expect, it } from "vitest";
import { isMonthlyLimitBlockActive, monthlyLimitBlockResetMs } from "./monthlyLimitBlock";

const NOW = Date.parse("2026-06-01T00:00:00Z");

describe("monthlyLimitBlock", () => {
  it("is inactive without a block", () => {
    expect(isMonthlyLimitBlockActive(undefined, NOW)).toBe(false);
    expect(isMonthlyLimitBlockActive(null, NOW)).toBe(false);
  });

  it("holds until the monthly pool reset passes", () => {
    const block = { resetsAt: "2026-06-01T00:00:01Z" };
    expect(isMonthlyLimitBlockActive(block, NOW)).toBe(true);
    expect(isMonthlyLimitBlockActive(block, NOW + 1000)).toBe(false);
    expect(isMonthlyLimitBlockActive(block, NOW + 60_000)).toBe(false);
  });

  it("holds while the pool reset is unknown or unreadable", () => {
    expect(isMonthlyLimitBlockActive({ resetsAt: null }, NOW)).toBe(true);
    expect(isMonthlyLimitBlockActive({ resetsAt: "not a date" }, NOW)).toBe(true);
    expect(monthlyLimitBlockResetMs({ resetsAt: null })).toBeNull();
    expect(monthlyLimitBlockResetMs({ resetsAt: "not a date" })).toBeNull();
  });

  it("parses the RFC 3339 reset the backend sends", () => {
    expect(monthlyLimitBlockResetMs({ resetsAt: "2026-06-30T12:00:00+00:00" })).toBe(
      Date.parse("2026-06-30T12:00:00Z"),
    );
  });
});
