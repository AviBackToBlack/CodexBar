import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("../hooks/useLocale", () => ({
  useLocale: () => ({
    t: (key: string) =>
      ({
        UsageSpendDailyLedger: "Daily ledger",
        UsageSpendDailyLedgerHelper: "Recorded daily cost and token totals",
        UsageSpendNoDailyData: "No daily history yet.",
        UsageSpendColDate: "Date",
        UsageSpendColCost: "Cost",
        UsageSpendColTokens: "Tokens",
        UsageSpendUnknown: "Unknown",
        UsageSpendTokens: "tokens",
        UsageSpendShowAll: "Show all",
        UsageSpendShowLess: "Show less",
      })[key] ?? key,
  }),
}));

import {
  DAILY_LEDGER_COLLAPSED_ROWS,
  UsageSpendDailyLedger,
  dailyLedgerVisibleRows,
} from "./UsageSpendDailyLedger";

/** `days` consecutive UTC days in oldest-first order, like the contract's daily history. */
function dailyHistory(days: number) {
  return Array.from({ length: days }, (_, index) => ({
    day: new Date(Date.UTC(2026, 0, 1) + index * 86_400_000).toISOString().slice(0, 10),
    costUsd: index,
    totalTokens: index,
  }));
}

describe("dailyLedgerVisibleRows", () => {
  // Translated from upstream v0.67.0 SpendDashboardDailyLedgerTests
  // `ledger expansion preserves every day in newest first order` (#3998).
  it.each([0, 7, 30, 31, 365])(
    "ledger expansion preserves every day in newest first order (%i days)",
    (days) => {
      const daily = dailyHistory(days);
      const collapsed = dailyLedgerVisibleRows(daily, false, 30);
      expect(collapsed).toHaveLength(Math.min(days, 30));
      expect(collapsed).toEqual(daily.slice(-30).reverse());
      const expanded = dailyLedgerVisibleRows(daily, true, 30);
      expect(expanded).toEqual([...daily].reverse());
    },
  );

  it("collapses to the upstream default of 30 rows", () => {
    expect(DAILY_LEDGER_COLLAPSED_ROWS).toBe(30);
    expect(dailyLedgerVisibleRows(dailyHistory(45), false)).toHaveLength(30);
  });
});

describe("UsageSpendDailyLedger", () => {
  it("renders newest daily rows and keeps zero distinct from unknown", () => {
    render(
      <UsageSpendDailyLedger
        daily={[
          { day: "2026-09-11", costUsd: null, totalTokens: 8_100 },
          { day: "2026-09-13", costUsd: 0, totalTokens: 0 },
          { day: "2026-09-12", costUsd: 0.12, totalTokens: null },
        ]}
      />,
    );

    const table = screen.getByRole("table", { name: "Daily ledger" });
    const rows = within(table).getAllByRole("row");
    expect(rows[1]).toHaveTextContent("2026-09-13");
    expect(rows[1]).toHaveTextContent("$0.00");
    expect(rows[1]).toHaveTextContent("0 tokens");
    expect(rows[2]).toHaveTextContent("2026-09-12");
    expect(rows[2]).toHaveTextContent("$0.12");
    expect(rows[2]).toHaveTextContent("Unknown");
    expect(rows[3]).toHaveTextContent("2026-09-11");
    expect(rows[3]).toHaveTextContent("Unknown");
    expect(rows[3]).toHaveTextContent("8,100 tokens");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("starts long ledgers with the newest 30 rows and a Show all control", () => {
    render(<UsageSpendDailyLedger daily={dailyHistory(31)} />);

    const table = screen.getByRole("table", { name: "Daily ledger" });
    let bodyRows = within(table).getAllByRole("row").slice(1);
    expect(bodyRows).toHaveLength(30);
    expect(bodyRows[0]).toHaveTextContent("2026-01-31");
    expect(bodyRows[29]).toHaveTextContent("2026-01-02");
    expect(table).not.toHaveTextContent("2026-01-01");

    const toggle = screen.getByRole("button", { name: "Show all (31)" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(toggle);

    bodyRows = within(table).getAllByRole("row").slice(1);
    expect(bodyRows).toHaveLength(31);
    expect(bodyRows[30]).toHaveTextContent("2026-01-01");
    const collapse = screen.getByRole("button", { name: "Show less" });
    expect(collapse).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(collapse);
    expect(within(table).getAllByRole("row").slice(1)).toHaveLength(30);
  });

  it("shows an explicit empty state when no daily points are available", () => {
    render(<UsageSpendDailyLedger daily={[]} />);

    expect(screen.getByText("No daily history yet.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
