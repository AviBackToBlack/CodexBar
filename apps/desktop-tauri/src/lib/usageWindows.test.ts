import { describe, expect, it } from "vitest";
import type { RateWindowSnapshot } from "../types/bridge";
import {
  resetDescriptionFallback,
  selectSingleMetricUsageWindow,
  windowDetailText,
} from "./usageWindows";

function rateWindow(
  overrides: Partial<RateWindowSnapshot> = {},
): RateWindowSnapshot {
  return {
    usedPercent: 0,
    remainingPercent: 100,
    windowMinutes: null,
    resetsAt: null,
    resetDescription: null,
    isExhausted: false,
    reservePercent: null,
    reserveDescription: null,
    ...overrides,
  };
}

describe("selectSingleMetricUsageWindow", () => {
  it("returns a normal primary when a secondary exists", () => {
    const primary = rateWindow();
    const secondary = rateWindow({ isInformational: true });

    expect(selectSingleMetricUsageWindow({ primary, secondary })).toBe(primary);
  });

  it("returns a real secondary when the primary is informational", () => {
    const primary = rateWindow({ isInformational: true });
    const secondary = rateWindow();

    expect(selectSingleMetricUsageWindow({ primary, secondary })).toBe(secondary);
  });

  it("returns an informational primary when the secondary is null", () => {
    const primary = rateWindow({ isInformational: true });

    expect(selectSingleMetricUsageWindow({ primary, secondary: null })).toBe(primary);
  });

  it("returns an informational primary when the secondary is informational", () => {
    const primary = rateWindow({ isInformational: true });
    const secondary = rateWindow({ isInformational: true });

    expect(selectSingleMetricUsageWindow({ primary, secondary })).toBe(primary);
  });
});

describe("detail-backed descriptions", () => {
  const amounts = "34.07 EUR / 255.00 EUR · 220.93 EUR remaining";

  it("keeps ordinary descriptions as the reset fallback and shows no detail line", () => {
    const window = rateWindow({ resetDescription: "Resets in 2h" });

    expect(resetDescriptionFallback(window)).toBe("Resets in 2h");
    expect(windowDetailText(window)).toBeNull();
  });

  it("moves a detail-backed description out of the reset fallback", () => {
    const window = rateWindow({ resetDescription: ` ${amounts} `, descriptionIsDetail: true });

    expect(resetDescriptionFallback(window)).toBeNull();
    expect(windowDetailText(window)).toBe(amounts);
  });

  it("ignores blank and informational detail-backed descriptions", () => {
    expect(
      windowDetailText(rateWindow({ resetDescription: "  ", descriptionIsDetail: true })),
    ).toBeNull();
    expect(
      windowDetailText(
        rateWindow({ resetDescription: amounts, descriptionIsDetail: true, isInformational: true }),
      ),
    ).toBeNull();
  });
});
