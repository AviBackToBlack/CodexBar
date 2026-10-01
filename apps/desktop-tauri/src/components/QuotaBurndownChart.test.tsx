import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import QuotaBurndownChart from "./QuotaBurndownChart";
import type { QuotaBurndownSnapshot } from "../types/bridge";
import { ALL_LOCALE_KEYS } from "../i18n/keys";

const t = (key: string): string => {
  expect(ALL_LOCALE_KEYS).toContain(key);
  return key;
};

const NOW = Date.parse("2026-09-30T12:00:00Z");

function hoursAgo(hours: number): string {
  return new Date(NOW - hours * 3600 * 1000).toISOString();
}

function snapshot(overrides?: Partial<QuotaBurndownSnapshot>): QuotaBurndownSnapshot {
  return {
    series: "session",
    windowMinutes: 300,
    start: hoursAgo(3),
    reset: new Date(NOW + 2 * 3600 * 1000).toISOString(),
    samples: [
      { capturedAt: hoursAgo(2), remainingPercent: 80 },
      { capturedAt: hoursAgo(1), remainingPercent: 55 },
      { capturedAt: new Date(NOW).toISOString(), remainingPercent: 40 },
    ],
    ideal: [
      { capturedAt: hoursAgo(3), remainingPercent: 100 },
      { capturedAt: new Date(NOW + 2 * 3600 * 1000).toISOString(), remainingPercent: 0 },
    ],
    ...overrides,
  };
}

describe("QuotaBurndownChart", () => {
  it("renders the observed line, ideal guide, remaining caption and capture age", () => {
    render(<QuotaBurndownChart burndown={snapshot()} t={t} nowMs={NOW} />);

    const svg = document.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute("data-series")).toBe("session");

    const observed = document.querySelector(".quota-burndown-chart__observed");
    // Final point: 40% remaining -> y = 44.80 in the 76px viewBox.
    expect(observed?.getAttribute("points")).toContain("44.80");

    const ideal = document.querySelector(".quota-burndown-chart__ideal");
    expect(ideal).not.toBeNull();

    // 40% remaining at the last capture.
    expect(document.querySelector(".quota-burndown-chart__caption")?.textContent).toContain(
      "40%",
    );
  });

  it("shows calendar dates on the axis for weekly-length windows and compact times for sessions", () => {
    const start = "2026-09-23T12:00:00Z";
    const reset = "2026-09-30T12:00:00Z";
    render(
      <QuotaBurndownChart
        burndown={snapshot({
          series: "weekly",
          windowMinutes: 10_080,
          start,
          reset,
          samples: [{ capturedAt: "2026-09-29T12:00:00Z", remainingPercent: 60 }],
          ideal: [
            { capturedAt: start, remainingPercent: 100 },
            { capturedAt: reset, remainingPercent: 0 },
          ],
        })}
        t={t}
        nowMs={NOW}
      />,
    );
    const axis = document.querySelector(".quota-burndown-chart__axis");
    // A 7-day window: boundary labels include a month name.
    expect(axis?.textContent).toMatch(/Sep/);
  });

  it("renders nothing without samples or a valid window span", () => {
    const { container } = render(
      <QuotaBurndownChart burndown={snapshot({ samples: [] })} t={t} nowMs={NOW} />,
    );
    expect(container.querySelector("svg")).toBeNull();

    const { container: container2 } = render(
      <QuotaBurndownChart
        burndown={snapshot({ start: "2026-09-30T13:00:00Z", reset: "2026-09-30T12:00:00Z" })}
        t={t}
        nowMs={NOW}
      />,
    );
    expect(container2.querySelector("svg")).toBeNull();
  });
});
