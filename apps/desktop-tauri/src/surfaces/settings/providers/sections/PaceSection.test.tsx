import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PaceSnapshot } from "../../../../types/bridge";
import { PaceSection } from "./PaceSection";

const pace: PaceSnapshot = {
  stage: "behind",
  deltaPercent: -10,
  willLastToReset: true,
  etaSeconds: null,
  expectedUsedPercent: 40,
  actualUsedPercent: 30,
};

describe("PaceSection", () => {
  it("renders the pace of an unblocked window", () => {
    render(<PaceSection pace={pace} t={(key) => key} />);

    expect(screen.getByText("DetailPaceTitle")).toBeInTheDocument();
    expect(screen.getByText("DetailPaceBehind")).toBeInTheDocument();
  });

  it("hides the pace of a window blocked by an exhausted monthly pool", () => {
    const future = new Date(Date.now() + 20 * 24 * 60 * 60 * 1000).toISOString();
    const { container } = render(
      <PaceSection pace={{ ...pace, monthlyLimitBlock: { resetsAt: future } }} t={(key) => key} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("shows the pace again once the monthly pool reset has passed", () => {
    const past = new Date(Date.now() - 60 * 1000).toISOString();
    render(
      <PaceSection pace={{ ...pace, monthlyLimitBlock: { resetsAt: past } }} t={(key) => key} />,
    );

    expect(screen.getByText("DetailPaceTitle")).toBeInTheDocument();
  });
});
