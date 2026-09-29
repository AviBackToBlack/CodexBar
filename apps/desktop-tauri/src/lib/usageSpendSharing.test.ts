import { describe, expect, it } from "vitest";

import {
  formatUsageSpendReportingDay,
  filterUsageSpendSummaryForOverview,
  renderUsageSpendSharePng,
  usageSpendShareColumns,
  usageSpendShareFooter,
  usageSpendSubscriptionCaption,
} from "./usageSpendSharing";
import type { SpendContract, UsageSpendRow, UsageSpendSummary } from "../types/bridge";

describe("usage spend sharing", () => {
  it.each([
    [0, "0 subscriptions"],
    [1, "1 subscription"],
    [2, "2 subscriptions"],
    [12, "12 subscriptions"],
  ])("uses the correct subscription caption for %i", (count, expected) => {
    expect(usageSpendSubscriptionCaption(count)).toBe(expected);
  });

  it("preserves the reporting day across dashboard timezones", () => {
    expect(formatUsageSpendReportingDay("2026-09-19", "Pacific/Kiritimati")).toBe("Sep 19, 2026");
    expect(formatUsageSpendReportingDay("2026-09-19", "America/Los_Angeles")).toBe("Sep 19, 2026");
    expect(formatUsageSpendReportingDay("2026-10-01", "Pacific/Norfolk")).toBe("Oct 1, 2026");
  });

  it("falls back for an invalid dashboard timezone or date", () => {
    expect(formatUsageSpendReportingDay("2026-10-01", "Not/AZone")).toBe("2026-10-01");
    expect(formatUsageSpendReportingDay("2026-02-31", "UTC")).toBe("2026-02-31");
  });

  it("builds the report footer from the included day and row count", () => {
    expect(
      usageSpendShareFooter({
        contract: {} as SpendContract,
        rows: [{
          providerId: "codex",
          displayName: "Codex",
          sevenDay: null,
          thirtyDay: null,
          currency: "USD",
          source: "local",
          includedInOverview: true,
        }],
        reportingDay: "2026-09-19",
        dashboardTimezone: "UTC",
      }),
    ).toBe("Data through Sep 19, 2026 · 1 subscription");
  });

  it("keeps hidden sources out of the Overview share summary", () => {
    const summary: UsageSpendSummary = {
      contract: {} as SpendContract,
      reportingDay: "2026-09-19",
      dashboardTimezone: "UTC",
      rows: [
        {
          providerId: "codex",
          displayName: "Codex",
          sevenDay: 1,
          thirtyDay: 2,
          currency: "USD",
          source: "local",
          includedInOverview: true,
        },
        {
          providerId: "claude",
          displayName: "Claude",
          sevenDay: 3,
          thirtyDay: 4,
          currency: "USD",
          source: "hidden",
          includedInOverview: false,
        },
      ],
    };

    expect(filterUsageSpendSummaryForOverview(summary).rows.map((row) => row.providerId)).toEqual([
      "codex",
    ]);
  });

  it("renders only the redaction-safe row fields in the share PNG", () => {
    // Redaction invariant (upstream #2112): the share image must never carry
    // account emails or provider-identity secrets. The renderer draws exactly
    // these cells; keep the invariant comment on renderUsageSpendSharePng in
    // sync if this list ever grows.
    const renderedCells = [
      "displayName",
      "periodCost",
      "periodTokens",
      "sevenDay",
      "thirtyDay",
      "currency",
      "source",
    ] as const;
    const row: UsageSpendRow = {
      providerId: "codex",
      displayName: "Codex",
      sevenDay: 1,
      thirtyDay: 2,
      currency: "USD",
      source: "local",
      includedInOverview: true,
    };
    const unsafeKeys = Object.keys(row).filter(
      (key) => !renderedCells.includes(key as (typeof renderedCells)[number]),
    );
    // Nothing outside the drawn cells may be read by the renderer, and no
    // UsageSpendRow field may carry account-identity data (no email/org).
    expect(Object.keys(row).filter((key) => /email|org|token|account/i.test(key))).toEqual([]);
    expect(unsafeKeys).toEqual(["providerId", "includedInOverview"]);
  });

  it("adds a labelled History window column while keeping the fixed 7 and 30 day columns", () => {
    const row: UsageSpendRow = {
      providerId: "codex",
      displayName: "Codex",
      sevenDay: 1,
      thirtyDay: 2,
      periodCost: 9,
      periodTokens: 900,
      currency: "USD",
      source: "local",
      includedInOverview: true,
    };
    const plain = usageSpendShareColumns();
    expect(plain.headers).toEqual(["Provider", "7 days", "30 days", "Currency", "Source"]);
    expect(plain.cellsFor(row)).toHaveLength(plain.headers.length);

    const withPeriod = usageSpendShareColumns("Month to date");
    expect(withPeriod.headers).toEqual([
      "Provider",
      "Month to date",
      "7 days",
      "30 days",
      "Currency",
      "Source",
    ]);
    const cells = withPeriod.cellsFor(row);
    expect(cells).toHaveLength(withPeriod.headers.length);
    expect(cells).toHaveLength(withPeriod.colW.length);
    expect(cells[1]).toContain("900");
    expect(cells[2]).not.toContain("900");
  });
});
