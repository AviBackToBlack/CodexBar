import { describe, expect, it } from "vitest";
import type { LocaleKey } from "../i18n/keys";
import {
  DEFAULT_COST_PERIOD,
  PRESET_COST_PERIODS,
  costPeriodLabel,
  costPeriodShortLabel,
  customPeriodRaw,
  isPresetCostPeriod,
  normalizeCostPeriod,
  parseCostPeriod,
  periodCostLabel,
  periodTokensLabel,
  rollingDays,
} from "./costPeriod";

const EN: Partial<Record<LocaleKey, string>> = {
  CostPeriodMonthToDate: "Month to date",
  CostPeriodAll: "All available history",
  CostPeriodToday: "Today",
  CostPeriodLastDays: "Last {} days",
  CostPeriodShortMonthToDate: "MTD",
  CostPeriodShortAll: "All",
  CostPeriodShortDays: "{}d",
  PanelPeriodCost: "{} cost",
  PanelPeriodTokens: "{} tokens",
};
const t = (key: LocaleKey) => EN[key] ?? key;

describe("parseCostPeriod", () => {
  it("reads the persisted forms", () => {
    expect(parseCostPeriod("month-to-date")).toEqual({ kind: "monthToDate" });
    expect(parseCostPeriod("all")).toEqual({ kind: "all" });
    expect(parseCostPeriod("rolling:7")).toEqual({ kind: "rolling", days: 7 });
    expect(parseCostPeriod(" rolling:90 ")).toEqual({ kind: "rolling", days: 90 });
  });

  it("clamps counts above 365 and rejects zero and unreadable values", () => {
    expect(parseCostPeriod("rolling:4000")).toEqual({ kind: "rolling", days: 365 });
    expect(parseCostPeriod("rolling:0")).toBeNull();
    expect(parseCostPeriod("rolling:-3")).toBeNull();
    expect(parseCostPeriod("rolling:seven")).toBeNull();
    expect(parseCostPeriod("weekly")).toBeNull();
    expect(parseCostPeriod("")).toBeNull();
    expect(parseCostPeriod(undefined)).toBeNull();
  });
});

describe("normalizeCostPeriod", () => {
  it("keeps valid periods and falls back to 30 days otherwise", () => {
    expect(normalizeCostPeriod("month-to-date")).toBe("month-to-date");
    expect(normalizeCostPeriod("rolling:9999")).toBe("rolling:365");
    expect(normalizeCostPeriod("rolling:0")).toBe(DEFAULT_COST_PERIOD);
    expect(normalizeCostPeriod(undefined)).toBe("rolling:30");
  });
});

describe("customPeriodRaw", () => {
  it("accepts integers in 1..=365 only", () => {
    expect(customPeriodRaw("1")).toBe("rolling:1");
    expect(customPeriodRaw(" 14 ")).toBe("rolling:14");
    expect(customPeriodRaw("365")).toBe("rolling:365");
    expect(customPeriodRaw("0")).toBeNull();
    expect(customPeriodRaw("366")).toBeNull();
    expect(customPeriodRaw("1.5")).toBeNull();
    expect(customPeriodRaw("-2")).toBeNull();
    expect(customPeriodRaw("")).toBeNull();
    expect(customPeriodRaw("abc")).toBeNull();
  });
});

describe("presets", () => {
  it("lists month to date, 1/7/30/90/365 days, and all", () => {
    expect([...PRESET_COST_PERIODS]).toEqual([
      "month-to-date",
      "rolling:1",
      "rolling:7",
      "rolling:30",
      "rolling:90",
      "rolling:365",
      "all",
    ]);
    expect(isPresetCostPeriod("rolling:7")).toBe(true);
    expect(isPresetCostPeriod("rolling:14")).toBe(false);
    expect(rollingDays("rolling:14")).toBe(14);
    expect(rollingDays("all")).toBeNull();
  });
});

describe("labels", () => {
  it("builds long labels", () => {
    expect(costPeriodLabel("month-to-date", t)).toBe("Month to date");
    expect(costPeriodLabel("all", t)).toBe("All available history");
    expect(costPeriodLabel("rolling:1", t)).toBe("Today");
    expect(costPeriodLabel("rolling:14", t)).toBe("Last 14 days");
    expect(costPeriodLabel(undefined, t)).toBe("Last 30 days");
  });

  it("builds short and menu labels", () => {
    expect(costPeriodShortLabel("month-to-date", t)).toBe("MTD");
    expect(costPeriodShortLabel("all", t)).toBe("All");
    expect(costPeriodShortLabel("rolling:7", t)).toBe("7d");
    expect(periodCostLabel("month-to-date", t)).toBe("MTD cost");
    expect(periodTokensLabel("rolling:90", t)).toBe("90d tokens");
  });
});
