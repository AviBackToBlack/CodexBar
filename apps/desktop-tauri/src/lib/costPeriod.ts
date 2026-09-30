import type { LocaleKey } from "../i18n/keys";

/**
 * History window ("cost reporting period") helpers. The persisted form mirrors
 * `codexbar::cost_reporting_period::CostReportingPeriod::raw`:
 * `rolling:N` (1..=365), `month-to-date`, or `all`.
 */

export const DEFAULT_COST_PERIOD = "rolling:30";
export const MAX_ROLLING_DAYS = 365;

export type ParsedCostPeriod =
  | { kind: "rolling"; days: number }
  | { kind: "monthToDate" }
  | { kind: "all" };

type Translate = (key: LocaleKey) => string;

/** Presets offered by the picker, in display order. Custom day counts are separate. */
export const PRESET_COST_PERIODS = [
  "month-to-date",
  "rolling:1",
  "rolling:7",
  "rolling:30",
  "rolling:90",
  "rolling:365",
  "all",
] as const;

/** Parse a persisted period. Unknown forms and `rolling:0` return null; counts above 365 clamp. */
export function parseCostPeriod(raw: string | null | undefined): ParsedCostPeriod | null {
  const value = raw?.trim();
  if (!value) return null;
  if (value === "month-to-date") return { kind: "monthToDate" };
  if (value === "all") return { kind: "all" };
  const match = /^rolling:(\d+)$/.exec(value);
  if (!match) return null;
  const days = Number.parseInt(match[1], 10);
  if (!Number.isSafeInteger(days) || days <= 0) return null;
  return { kind: "rolling", days: Math.min(days, MAX_ROLLING_DAYS) };
}

function toRaw(parsed: ParsedCostPeriod): string {
  switch (parsed.kind) {
    case "monthToDate":
      return "month-to-date";
    case "all":
      return "all";
    case "rolling":
      return `rolling:${parsed.days}`;
  }
}

/** Canonical persisted form, or the 30-day default when the value is missing or unreadable. */
export function normalizeCostPeriod(raw: string | null | undefined): string {
  const parsed = parseCostPeriod(raw);
  return parsed ? toRaw(parsed) : DEFAULT_COST_PERIOD;
}

/** Persisted form for a typed custom day count; null unless it is an integer in 1..=365. */
export function customPeriodRaw(input: string): string | null {
  const text = input.trim();
  if (!/^\d+$/.test(text)) return null;
  const days = Number.parseInt(text, 10);
  if (!Number.isSafeInteger(days) || days < 1 || days > MAX_ROLLING_DAYS) return null;
  return `rolling:${days}`;
}

export function isPresetCostPeriod(raw: string): boolean {
  return (PRESET_COST_PERIODS as readonly string[]).includes(raw);
}

/** Day count of a rolling period, or null for month-to-date / all. */
export function rollingDays(raw: string | null | undefined): number | null {
  const parsed = parseCostPeriod(raw);
  return parsed?.kind === "rolling" ? parsed.days : null;
}

function fill(template: string, value: string): string {
  return template.replace("{}", value);
}

/** Long label, for headings and captions ("Month to date", "Last 7 days"). */
export function costPeriodLabel(raw: string | null | undefined, t: Translate): string {
  const parsed = parseCostPeriod(raw ?? DEFAULT_COST_PERIOD) ?? { kind: "rolling", days: 30 };
  switch (parsed.kind) {
    case "monthToDate":
      return t("CostPeriodMonthToDate");
    case "all":
      return t("CostPeriodAll");
    case "rolling":
      return parsed.days === 1
        ? t("CostPeriodToday")
        : fill(t("CostPeriodLastDays"), String(parsed.days));
  }
}

/** Compact label, for menu rows and the float bar ("MTD", "7d", "All"). */
export function costPeriodShortLabel(raw: string | null | undefined, t: Translate): string {
  const parsed = parseCostPeriod(raw ?? DEFAULT_COST_PERIOD) ?? { kind: "rolling", days: 30 };
  switch (parsed.kind) {
    case "monthToDate":
      return t("CostPeriodShortMonthToDate");
    case "all":
      return t("CostPeriodShortAll");
    case "rolling":
      return fill(t("CostPeriodShortDays"), String(parsed.days));
  }
}

/** Menu row label for the selected window ("MTD cost"). */
export function periodCostLabel(raw: string | null | undefined, t: Translate): string {
  return fill(t("PanelPeriodCost"), costPeriodShortLabel(raw, t));
}

export function periodTokensLabel(raw: string | null | undefined, t: Translate): string {
  return fill(t("PanelPeriodTokens"), costPeriodShortLabel(raw, t));
}
