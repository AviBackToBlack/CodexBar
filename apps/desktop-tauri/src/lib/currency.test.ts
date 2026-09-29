import { describe, expect, it } from "vitest";
import {
  CURRENCY_PICKER_OPTIONS,
  FALLBACK_CURRENCY_RATES,
  SUPPORTED_CURRENCIES,
  convertCurrencyAmount,
  formatDisplayCurrency,
  mergeValidCurrencyRates,
  normalizePreferredCurrency,
  sumDisplayCurrencyAmounts,
} from "./currency";

describe("preferred currency display", () => {
  it("normalizes supported preferences and falls back safely for unknown codes", () => {
    expect(normalizePreferredCurrency(undefined)).toBe("AUTO");
    expect(normalizePreferredCurrency(" try ")).toBe("TRY");
    expect(normalizePreferredCurrency("BTC")).toBe("AUTO");
  });

  it("converts both currencies through the USD pivot and rounds for display", () => {
    expect(convertCurrencyAmount(10, "USD", "TRY", FALLBACK_CURRENCY_RATES)).toBe(485);
    expect(convertCurrencyAmount(10, "GBP", "TRY", FALLBACK_CURRENCY_RATES)).toBeCloseTo(613.92405, 4);
    const display = formatDisplayCurrency(10, "USD", "TRY", FALLBACK_CURRENCY_RATES);
    expect(display).not.toContain("10.00");
    expect(display).toMatch(/485/);
  });

  it("lists the twelve added currencies after TRY with upstream fallback rates", () => {
    const added = ["NZD", "SEK", "NOK", "DKK", "PLN", "BRL", "MXN", "ZAR", "THB", "IDR", "VND", "UAH"];
    const tryIndex = SUPPORTED_CURRENCIES.indexOf("TRY");
    expect(SUPPORTED_CURRENCIES.slice(tryIndex + 1)).toEqual(added);
    expect(CURRENCY_PICKER_OPTIONS.slice(tryIndex + 1).map((option) => option.label)).toEqual([
      "NZD ($)", "SEK (kr)", "NOK (kr)", "DKK (kr)", "PLN (zł)", "BRL (R$)",
      "MXN ($)", "ZAR (R)", "THB (฿)", "IDR (Rp)", "VND (₫)", "UAH (₴)",
    ]);
    expect(FALLBACK_CURRENCY_RATES).toMatchObject({
      NZD: 1.761, SEK: 9.908, NOK: 9.48, DKK: 6.554, PLN: 3.838, BRL: 5.117,
      MXN: 17.47, ZAR: 16.36, THB: 33.37, IDR: 17836, VND: 25962, UAH: 44.86,
    });
    expect(Object.keys(FALLBACK_CURRENCY_RATES)).toEqual([...SUPPORTED_CURRENCIES]);
    for (const code of added) expect(normalizePreferredCurrency(code.toLowerCase())).toBe(code);
  });

  it("converts each added currency through the USD pivot", () => {
    for (const code of ["NZD", "SEK", "NOK", "DKK", "PLN", "BRL", "MXN", "ZAR", "THB", "IDR", "VND", "UAH"]) {
      const rate = FALLBACK_CURRENCY_RATES[code];
      expect(convertCurrencyAmount(10, "USD", code, FALLBACK_CURRENCY_RATES)).toBeCloseTo(10 * rate, 8);
      expect(convertCurrencyAmount(10 * rate, code, "USD", FALLBACK_CURRENCY_RATES)).toBeCloseTo(10, 8);
    }
    expect(convertCurrencyAmount(10, "EUR", "SEK", FALLBACK_CURRENCY_RATES)).toBeCloseTo((10 / 0.92) * 9.908, 8);
  });

  it("formats zero-decimal currencies without fractional units", () => {
    const vnd = formatDisplayCurrency(0.01, "USD", "VND", FALLBACK_CURRENCY_RATES);
    expect(vnd).toMatch(/260/);
    expect(vnd).not.toMatch(/[.,]\d{1,2}\D*$/);
    expect(formatDisplayCurrency(1, "USD", "JPY", FALLBACK_CURRENCY_RATES)).toMatch(/154(?![.,]\d)/);
  });

  it("keeps AUTO, credits, unknown units, and missing-rate values in source units", () => {
    expect(formatDisplayCurrency(4.25, "USD", "AUTO", FALLBACK_CURRENCY_RATES)).toMatch(/4\.25/);
    expect(formatDisplayCurrency(4.25, "Credits", "TRY", FALLBACK_CURRENCY_RATES)).toBe("4.25 Credits");
    expect(formatDisplayCurrency(10, "USD", "TRY", {} , "$" )).toBe("$10.00");
    expect(convertCurrencyAmount(8, "Quota", "TRY", FALLBACK_CURRENCY_RATES)).toBeNull();
  });

  it("rejects malformed exchange rates and preserves offline fallbacks", () => {
    const rates = mergeValidCurrencyRates({ USD: 1, TRY: Number.NaN, EUR: -2, BTC: 90 });
    expect(rates.TRY).toBe(48.5);
    expect(rates.EUR).toBe(0.92);
    expect(rates.BTC).toBeUndefined();
  });

  it("sums only converted overview rows and reports incomplete coverage", () => {
    const result = sumDisplayCurrencyAmounts([
      { amount: 10, currency: "USD" },
      { amount: 10, currency: "EUR" },
      { amount: 5, currency: "Credits" },
    ], "TRY", FALLBACK_CURRENCY_RATES);
    expect(result.included).toBe(2);
    expect(result.considered).toBe(3);
    expect(result.total).toBeCloseTo(10 * 48.5 + (10 / 0.92) * 48.5, 8);
  });
});
