import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { SpendContract, SpendModelRow } from "../../../types/bridge";
import type { LocaleKey } from "../../../i18n/keys";
import { buildSpendProviderGroups } from "../../../lib/usageSpendBreakdown";
import { UsageSpendProviderGroups } from "./UsageSpendProviderGroups";
import { UsageSpendTrendPanel } from "./UsageSpendTrendPanel";

const t = (key: LocaleKey) => key;

const models: SpendModelRow[] = Array.from({ length: 8 }, (_, index) => ({
  model: `model-${index + 1}`,
  costUsd: 8 - index,
  inputTokens: 10,
  outputTokens: 0,
  cacheReadTokens: 0,
  totalTokens: 10,
  customPricing: false,
}));

const contract = {
  providerId: "codex",
  historyDays: 30,
  knownCostUsd: 36,
  historyCoverageEstablished: false,
  priceCoverage: { priced: 8, unpriced: 0, unmetered: 0, estimated: 0 },
  models,
  daily: [
    { day: "2026-09-18", costUsd: 4, totalTokens: 40 },
    { day: "2026-09-19", costUsd: 2, totalTokens: 20 },
  ],
  hourlyActivity: [{ weekday: 0, hour: 9, conversations: 3 }],
} as unknown as SpendContract;

describe("UsageSpendProviderGroups", () => {
  const groups = buildSpendProviderGroups(contract, "Codex");

  it("shows six models per provider, expands, and collapses the group", () => {
    render(<UsageSpendProviderGroups groups={groups} historyDays={30} t={t} />);
    expect(screen.getByText("Codex")).toBeTruthy();
    expect(screen.getByText("UsageSpendPartialModelBreakdown")).toBeTruthy();
    expect(screen.queryByText("model-7")).toBeNull();
    expect(screen.getByText("model-6")).toBeTruthy();

    fireEvent.click(screen.getByText(/UsageSpendShowAll/));
    expect(screen.getByText("model-8")).toBeTruthy();
    fireEvent.click(screen.getByText("UsageSpendShowLess"));
    expect(screen.queryByText("model-8")).toBeNull();

    const header = screen.getByRole("button", { expanded: true });
    fireEvent.click(header);
    expect(screen.queryByText("model-1")).toBeNull();
    expect(header.getAttribute("aria-expanded")).toBe("false");
  });

  it("shows the empty state without groups", () => {
    render(<UsageSpendProviderGroups groups={[]} historyDays={30} t={t} />);
    expect(screen.getByText("UsageSpendNoModels")).toBeTruthy();
  });
});

describe("UsageSpendTrendPanel", () => {
  function Harness() {
    const [day, setDay] = useState<string | null>(null);
    return <UsageSpendTrendPanel contract={contract} selectedDay={day} onSelectDay={setDay} t={t} />;
  }

  it("selects a day from the daily chart and clears it with one control", () => {
    render(<Harness />);
    expect(screen.queryByTestId("usage-spend-selected-day")).toBeNull();
    const bars = screen.getAllByRole("button", { pressed: false }).filter((node) => node.getAttribute("title"));
    expect(bars).toHaveLength(2);

    fireEvent.click(bars[0]);
    expect(screen.getByTestId("usage-spend-selected-day")).toBeTruthy();
    expect(screen.getByText("UsageSpendSelectedDayScope")).toBeTruthy();

    fireEvent.click(screen.getByLabelText("UsageSpendClearSelectedDay"));
    expect(screen.queryByTestId("usage-spend-selected-day")).toBeNull();
  });

  it("switches between the daily and hourly views with one selector", () => {
    render(<Harness />);
    expect(screen.getByText("UsageSpendDailySpend")).toBeTruthy();
    fireEvent.click(screen.getByText("UsageSpendTrendHourly"));
    expect(screen.getByLabelText("UsageSpendHourlyActivity")).toBeTruthy();
    expect(screen.queryByText("UsageSpendDailySpend")).toBeNull();
  });

  it("drops the selector when only one view has data", () => {
    const hourlyOnly = { ...contract, daily: [] } as unknown as SpendContract;
    render(<UsageSpendTrendPanel contract={hourlyOnly} selectedDay={null} onSelectDay={() => {}} t={t} />);
    expect(screen.queryByText("UsageSpendTrendDaily")).toBeNull();
    expect(screen.getByLabelText("UsageSpendHourlyActivity")).toBeTruthy();
  });
});
