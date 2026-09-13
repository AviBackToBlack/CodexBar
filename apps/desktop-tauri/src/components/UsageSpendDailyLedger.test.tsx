import { render, screen, within } from "@testing-library/react";
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
      })[key] ?? key,
  }),
}));

import { UsageSpendDailyLedger } from "./UsageSpendDailyLedger";

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
  });

  it("shows an explicit empty state when no daily points are available", () => {
    render(<UsageSpendDailyLedger daily={[]} />);

    expect(screen.getByText("No daily history yet.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });
});
