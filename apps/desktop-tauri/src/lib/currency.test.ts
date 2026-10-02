import { describe, expect, it } from "vitest";
import { formatDisplayCurrency, sumDisplayCurrencyAmounts } from "./currency";

// Rust owns the currency model (`codexbar::currency`); these rates mirror the
// merged snapshot the backend returns so the format tests stay deterministic.
const RATES: Record<string, number> = {
  USD: 1,
  GBP: 0.79,
  EUR: 0.92,
  TRY: 48.5,
};

describe("preferred currency display", () => {
  it("converts through the USD pivot and formats with Intl", () => {
    const display = formatDisplayCurrency(10, "USD", "TRY", RATES);
    expect(display).not.toContain("10.00");
    expect(display).toMatch(/485/);
  });

  it("keeps AUTO, credits, unknown units, and missing-rate values in source units", () => {
    expect(formatDisplayCurrency(4.25, "USD", "AUTO", RATES)).toMatch(/4\.25/);
    expect(formatDisplayCurrency(4.25, "Credits", "TRY", RATES)).toBe("4.25 Credits");
    expect(formatDisplayCurrency(10, "USD", "TRY", {}, "$")).toBe("$10.00");
  });

  it("prefers the source symbol when both codes match", () => {
    expect(formatDisplayCurrency(4.25, "USD", "USD", RATES, "$")).toBe("$4.25");
  });

  it("sums only converted overview rows and reports incomplete coverage", () => {
    const result = sumDisplayCurrencyAmounts(
      [
        { amount: 10, currency: "USD" },
        { amount: 10, currency: "EUR" },
        { amount: 5, currency: "Credits" },
      ],
      "TRY",
      RATES,
    );
    expect(result.included).toBe(2);
    expect(result.considered).toBe(3);
    expect(result.total).toBeCloseTo(10 * 48.5 + (10 / 0.92) * 48.5, 8);
  });

  it("sums USD-only rows unchanged under AUTO", () => {
    const result = sumDisplayCurrencyAmounts(
      [
        { amount: 10, currency: "USD" },
        { amount: 10, currency: "EUR" },
      ],
      "AUTO",
      RATES,
    );
    expect(result.included).toBe(1);
    expect(result.total).toBe(10);
  });
});
