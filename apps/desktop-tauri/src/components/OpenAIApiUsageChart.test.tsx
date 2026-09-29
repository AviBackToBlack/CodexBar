import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { LocaleKey } from "../i18n/keys";
import type {
  OpenAiApiDailyUsageSnapshot,
  OpenAiApiUsageSnapshot,
} from "../types/bridge";
import { OpenAIApiUsageChart } from "./OpenAIApiUsageChart";

const DAY = 86_400;
// 2026-09-01T00:00:00Z
const START = 1_788_220_800;

const strings: Partial<Record<LocaleKey, string>> = {
  OpenAIChartMore: "+{} more",
  OpenAIChartWindowToday: "Today",
  OpenAIChartWindowDays: "Last {} days",
  UsageSpendRequests: "requests",
};
const t = (key: LocaleKey) => strings[key] ?? key;

function day(
  index: number,
  overrides: Partial<OpenAiApiDailyUsageSnapshot> = {},
): OpenAiApiDailyUsageSnapshot {
  return {
    startTime: START + index * DAY,
    endTime: START + (index + 1) * DAY,
    costUsd: 1 + index,
    requests: 10 * (index + 1),
    inputTokens: 1_000 * (index + 1),
    cachedInputTokens: 100 * (index + 1),
    outputTokens: 500 * (index + 1),
    totalTokens: 1_500 * (index + 1),
    lineItems: [{ name: "Text tokens", costUsd: 1 + index }],
    models: [
      {
        name: `gpt-model-${index}`,
        requests: 10 * (index + 1),
        inputTokens: 1_000 * (index + 1),
        cachedInputTokens: 100 * (index + 1),
        outputTokens: 500 * (index + 1),
        totalTokens: 1_500 * (index + 1),
      },
    ],
    ...overrides,
  };
}

function usage(days: OpenAiApiDailyUsageSnapshot[], historyDays = 30): OpenAiApiUsageSnapshot {
  return { historyDays, projectId: null, daily: days };
}

function renderChart(snapshot: OpenAiApiUsageSnapshot, onLayoutChange?: () => void) {
  return render(
    <OpenAIApiUsageChart
      usage={snapshot}
      animations={false}
      t={t}
      onLayoutChange={onLayoutChange}
    />,
  );
}

function bars() {
  return screen.getAllByRole("option");
}

describe("OpenAIApiUsageChart", () => {
  it("selects the latest day by default and shows its detail", () => {
    renderChart(usage([day(0), day(1), day(2)]));
    expect(bars()).toHaveLength(3);
    expect(bars()[2]).toHaveAttribute("aria-selected", "true");
    expect(bars()[2]).toHaveAttribute("tabindex", "0");
    expect(bars()[0]).toHaveAttribute("tabindex", "-1");
    const detail = document.querySelector(".openai-usage__detail") as HTMLElement;
    expect(within(detail).getByText("2026-09-03")).toBeInTheDocument();
    // Day cost in the stat grid plus the single line item carrying the same cost.
    expect(within(detail).getAllByText("$3.00")).toHaveLength(2);
    expect(within(detail).getByText("30")).toBeInTheDocument();
    expect(within(detail).getByText("3,000")).toBeInTheDocument();
    expect(within(detail).getByText("300")).toBeInTheDocument();
    expect(within(detail).getByText("1,500")).toBeInTheDocument();
    expect(within(detail).getByText("gpt-model-2")).toBeInTheDocument();
  });

  it("shows the window total in the footer", () => {
    renderChart(usage([day(0), day(1), day(2)]));
    expect(screen.getByText("Last 30 days").nextElementSibling).toHaveTextContent("$6.00");
  });

  it("labels a one-day window Today", () => {
    renderChart(usage([day(0)], 1));
    expect(screen.getByText("Today")).toBeInTheDocument();
  });

  it("selects a day on hover", () => {
    renderChart(usage([day(0), day(1), day(2)]));
    fireEvent.mouseEnter(bars()[0]);
    expect(bars()[0]).toHaveAttribute("aria-selected", "true");
    expect(document.querySelector(".openai-usage__day")).toHaveTextContent("2026-09-01");
    expect(screen.getByText("gpt-model-0")).toBeInTheDocument();
    expect(screen.queryByText("gpt-model-2")).toBeNull();
  });

  it("moves the selection with Left/Right/Home/End and keeps focus on the bar", () => {
    renderChart(usage([day(0), day(1), day(2), day(3)]));
    const list = screen.getByRole("listbox");
    fireEvent.keyDown(list, { key: "ArrowLeft" });
    expect(bars()[2]).toHaveAttribute("aria-selected", "true");
    expect(bars()[2]).toHaveFocus();
    fireEvent.keyDown(list, { key: "Home" });
    expect(bars()[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(list, { key: "ArrowLeft" });
    expect(bars()[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(list, { key: "End" });
    expect(bars()[3]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(list, { key: "ArrowRight" });
    expect(bars()[3]).toHaveAttribute("aria-selected", "true");
    expect(bars()[3]).toHaveAttribute("tabindex", "0");
  });

  it("keeps the selected day when a refresh delivers new data", () => {
    const { rerender } = renderChart(usage([day(0), day(1), day(2)]));
    fireEvent.mouseEnter(bars()[1]);
    rerender(
      <OpenAIApiUsageChart
        usage={usage([day(0), day(1), day(2), day(3)])}
        animations={false}
        t={t}
      />,
    );
    expect(bars()).toHaveLength(4);
    expect(bars()[1]).toHaveAttribute("aria-selected", "true");
  });

  it("toggles between Cost and Tokens with the tab strip", () => {
    renderChart(usage([day(0), day(1)]));
    const cost = screen.getByRole("tab", { name: "OpenAIChartMetricCost" });
    const tokens = screen.getByRole("tab", { name: "OpenAIChartMetricTokens" });
    expect(cost).toHaveAttribute("aria-selected", "true");
    expect(tokens).toHaveAttribute("tabindex", "-1");
    expect(bars()[1]).toHaveAttribute("aria-label", "2026-09-02: $2.00");

    fireEvent.click(tokens);
    expect(tokens).toHaveAttribute("aria-selected", "true");
    expect(bars()[1]).toHaveAttribute("aria-label", "2026-09-02: 3K");
    expect(screen.getByText("Last 30 days").nextElementSibling).toHaveTextContent("4.5K");
    expect(screen.getByRole("tabpanel")).toHaveAttribute("aria-labelledby", tokens.id);
  });

  it("moves between the metric tabs with the arrow keys", () => {
    renderChart(usage([day(0), day(1)]));
    const cost = screen.getByRole("tab", { name: "OpenAIChartMetricCost" });
    const tokens = screen.getByRole("tab", { name: "OpenAIChartMetricTokens" });
    fireEvent.keyDown(cost, { key: "ArrowRight" });
    expect(tokens).toHaveAttribute("aria-selected", "true");
    expect(tokens).toHaveFocus();
    fireEvent.keyDown(tokens, { key: "ArrowLeft" });
    expect(cost).toHaveAttribute("aria-selected", "true");
    expect(cost).toHaveFocus();
  });

  it("caps line items and models at five rows plus a more row", () => {
    const many = day(0, {
      lineItems: Array.from({ length: 7 }, (_, i) => ({ name: `item-${i}`, costUsd: 7 - i })),
      models: Array.from({ length: 6 }, (_, i) => ({
        name: `model-${i}`,
        requests: 1,
        inputTokens: 1,
        cachedInputTokens: 0,
        outputTokens: 1,
        totalTokens: 2,
      })),
    });
    renderChart(usage([many]));
    expect(screen.getByText("item-4")).toBeInTheDocument();
    expect(screen.queryByText("item-5")).toBeNull();
    expect(screen.getByText("+2 more")).toBeInTheDocument();
    expect(screen.getByText("model-4")).toBeInTheDocument();
    expect(screen.queryByText("model-5")).toBeNull();
    expect(screen.getByText("+1 more")).toBeInTheDocument();
  });

  it("formats sub-cent costs and zero-token days", () => {
    renderChart(
      usage([
        day(0, {
          costUsd: 0.004,
          totalTokens: 0,
          inputTokens: 0,
          cachedInputTokens: 0,
          outputTokens: 0,
          requests: 0,
          lineItems: [],
          models: [],
        }),
      ]),
    );
    const detail = document.querySelector(".openai-usage__detail") as HTMLElement;
    expect(within(detail).getByText("<$0.01")).toBeInTheDocument();
    expect(within(detail).queryByText("OpenAIChartLineItems")).toBeNull();
    expect(within(detail).queryByText("OpenAIChartModels")).toBeNull();
  });

  it("draws only the latest 60 days and relabels the window to match", () => {
    const days = Array.from({ length: 90 }, (_, i) => day(i));
    renderChart(usage(days, 90));
    expect(bars()).toHaveLength(60);
    expect(screen.getByText("Last 60 days")).toBeInTheDocument();
  });

  it("shows the empty message when the window has no days", () => {
    renderChart(usage([]));
    expect(screen.getByText("DetailChartEmpty")).toBeInTheDocument();
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("asks the host to re-measure when the detail panel changes", async () => {
    const onLayoutChange = vi.fn();
    renderChart(usage([day(0), day(1)]), onLayoutChange);
    await vi.waitFor(() => expect(onLayoutChange).toHaveBeenCalledTimes(1));
    fireEvent.mouseEnter(bars()[0]);
    await vi.waitFor(() => expect(onLayoutChange).toHaveBeenCalledTimes(2));
  });
});
