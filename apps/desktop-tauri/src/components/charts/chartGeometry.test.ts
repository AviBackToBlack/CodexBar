import { describe, expect, it } from "vitest";
import {
  DATE_EDGE_PADDING,
  WIDTH,
  getBarCenter,
  getBarWidth,
  getScrollableChartWidth,
  shouldRenderCenterMax,
} from "./chartGeometry";

describe("chart axis geometry", () => {
  it("keeps two bar centers symmetric around the chart center without a max label", () => {
    const first = getBarCenter(0, 2);
    const last = getBarCenter(1, 2);

    expect((first + last) / 2).toBe(WIDTH / 2);
    expect(first).toBeGreaterThanOrEqual(DATE_EDGE_PADDING);
    expect(first).toBeLessThanOrEqual(WIDTH - DATE_EDGE_PADDING);
    expect(last).toBeGreaterThanOrEqual(DATE_EDGE_PADDING);
    expect(last).toBeLessThanOrEqual(WIDTH - DATE_EDGE_PADDING);
    expect(shouldRenderCenterMax(2)).toBe(false);
  });

  it("renders the center max label for three or more points", () => {
    expect(shouldRenderCenterMax(3)).toBe(true);
  });

  it("allocates enough scrollable width to keep long-series bars visible", () => {
    const width = getScrollableChartWidth(90);
    const lastBarRight = getBarCenter(89, 90, width) + getBarWidth(90, width) / 2;

    expect(width).toBeGreaterThan(WIDTH);
    expect(getBarWidth(90, width)).toBeCloseTo(1.5);
    expect(lastBarRight).toBeLessThanOrEqual(width - DATE_EDGE_PADDING);
  });
});
