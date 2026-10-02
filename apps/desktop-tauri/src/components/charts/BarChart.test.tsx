import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { BarChart } from "./BarChart";

describe("BarChart calendar slots", () => {
  it("keeps unknown and known-zero slots distinct", () => {
    const { container } = render(
      <BarChart
        data={[
          { label: "unknown", value: null },
          { label: "zero", value: 0 },
          { label: "known", value: 2 },
        ]}
        ariaLabel="history"
        animations={false}
      />,
    );
    const bars = container.querySelectorAll(".chart__bar");
    expect(bars).toHaveLength(3);
    expect(bars[0]).toHaveAttribute("opacity", "0");
    expect(bars[1]).toHaveAttribute("opacity", "0.25");
    expect(container).toHaveTextContent("unknown");
    expect(container).toHaveTextContent("zero: 0.00");
  });

  it("keeps full endpoint dates in the axis", () => {
    const { container } = render(
      <BarChart
        data={[
          { label: "2026-09-01", value: 2 },
          { label: "2026-09-30", value: 3 },
        ]}
        ariaLabel="history"
        animations={false}
      />,
    );

    const labels = container.querySelectorAll(".chart__axis > span");
    expect(labels).toHaveLength(2);
    expect(labels[0]).toHaveTextContent("2026-09-01");
    expect(labels[1]).toHaveTextContent("2026-09-30");
    expect((labels[0] as HTMLElement).style.left).toBe("87.5px");
    expect((labels[1] as HTMLElement).style.left).toBe("192.5px");
    expect(container.querySelector(".chart__axis-max")).toBeNull();
    expect(labels[0]).toHaveClass("chart__axis-start");
    expect(labels[1]).toHaveClass("chart__axis-end");
    expect((labels[0] as HTMLElement).style.transform).toBe("");
    expect((labels[1] as HTMLElement).style.transform).toBe("");
  });
});

describe("BarChart controlled selection", () => {
  const data = [
    { label: "a", value: 1 },
    { label: "b", value: 2 },
    { label: "c", value: 3 },
  ];

  function setup(index = 2) {
    const onSelect = vi.fn();
    const utils = render(
      <BarChart
        data={data}
        ariaLabel="history"
        animations={false}
        selection={{ index, onSelect }}
      />,
    );
    return { onSelect, ...utils };
  }

  it("exposes a listbox of options with a roving tabindex", () => {
    setup(1);
    expect(screen.getByRole("listbox")).toHaveAttribute("aria-orientation", "horizontal");
    const options = screen.getAllByRole("option");
    expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
    expect(options.map((o) => o.getAttribute("tabindex"))).toEqual(["-1", "0", "-1"]);
    expect(options[1]).toHaveAttribute("data-selected", "true");
    expect(options[1]).toHaveAttribute("opacity", "1");
    expect(options[0]).toHaveAttribute("opacity", "0.6");
  });

  it("reports hover and focus as a selection change", () => {
    const { onSelect } = setup();
    const options = screen.getAllByRole("option");
    fireEvent.mouseEnter(options[0]);
    expect(onSelect).toHaveBeenLastCalledWith(0);
    fireEvent.focus(options[1]);
    expect(onSelect).toHaveBeenLastCalledWith(1);
  });

  it("navigates with the arrow, Home and End keys and clamps at the ends", () => {
    const { onSelect } = setup(1);
    const list = screen.getByRole("listbox");
    fireEvent.keyDown(list, { key: "ArrowLeft" });
    expect(onSelect).toHaveBeenLastCalledWith(0);
    fireEvent.keyDown(list, { key: "ArrowRight" });
    expect(onSelect).toHaveBeenLastCalledWith(2);
    fireEvent.keyDown(list, { key: "Home" });
    expect(onSelect).toHaveBeenLastCalledWith(0);
    fireEvent.keyDown(list, { key: "End" });
    expect(onSelect).toHaveBeenLastCalledWith(2);
    onSelect.mockClear();
    fireEvent.keyDown(list, { key: "Tab" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("does not open the hover tooltip in selection mode", () => {
    const { container } = setup();
    fireEvent.mouseMove(container.querySelector("svg") as SVGElement, { clientX: 10, clientY: 10 });
    expect(container.querySelector(".chart__tooltip")).toBeNull();
  });

  it("stays a plain image chart without a selection prop", () => {
    const { container } = render(<BarChart data={data} ariaLabel="history" animations={false} />);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(container.querySelector("[data-selected]")).toBeNull();
  });
});
