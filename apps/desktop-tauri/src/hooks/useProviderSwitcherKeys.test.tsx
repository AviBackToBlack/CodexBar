import { fireEvent, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useProviderSwitcherKeys } from "./useProviderSwitcherKeys";

const IDS = ["codex", "claude", "gemini"];

function setup(selectedProviderId: string | null = null) {
  const onSelect = vi.fn();
  const view = renderHook(
    (props: { selected: string | null }) =>
      useProviderSwitcherKeys({
        providerIds: IDS,
        selectedProviderId: props.selected,
        onSelect,
      }),
    { initialProps: { selected: selectedProviderId } },
  );
  return { onSelect, ...view };
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("useProviderSwitcherKeys", () => {
  it("moves with Right and Left, wrapping through Overview", () => {
    const { onSelect, rerender } = setup(null);
    fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" });
    expect(onSelect).toHaveBeenLastCalledWith("codex");

    rerender({ selected: "gemini" });
    fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" });
    expect(onSelect).toHaveBeenLastCalledWith(null);

    rerender({ selected: null });
    fireEvent.keyDown(window, { key: "ArrowLeft", code: "ArrowLeft" });
    expect(onSelect).toHaveBeenLastCalledWith("gemini");
  });

  it("selects the Nth segment with Ctrl+digit, Overview first", () => {
    const { onSelect } = setup("codex");
    fireEvent.keyDown(window, { key: "1", code: "Digit1", ctrlKey: true });
    expect(onSelect).toHaveBeenLastCalledWith(null);
    fireEvent.keyDown(window, { key: "3", code: "Digit3", ctrlKey: true });
    expect(onSelect).toHaveBeenLastCalledWith("claude");
  });

  it("prevents default only when it handles the key", () => {
    setup(null);
    expect(fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" })).toBe(false);
    expect(fireEvent.keyDown(window, { key: "9", code: "Digit9", ctrlKey: true })).toBe(true);
    expect(fireEvent.keyDown(window, { key: "x", code: "KeyX" })).toBe(true);
  });

  it("ignores keys typed into inputs and sliders", () => {
    const { onSelect } = setup(null);
    const input = document.createElement("input");
    input.type = "range";
    document.body.append(input);
    fireEvent.keyDown(input, { key: "ArrowRight", code: "ArrowRight" });
    const field = document.createElement("input");
    document.body.append(field);
    fireEvent.keyDown(field, { key: "ArrowLeft", code: "ArrowLeft" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("ignores keys while a grid drag is active", () => {
    const { onSelect } = setup(null);
    const dragging = document.createElement("button");
    dragging.className = "provider-grid__item provider-grid__item--dragging";
    document.body.append(dragging);
    fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("ignores events another handler already consumed", () => {
    const { onSelect } = setup(null);
    const consume = (event: KeyboardEvent) => event.preventDefault();
    document.addEventListener("keydown", consume);
    fireEvent.keyDown(document.body, { key: "ArrowRight", code: "ArrowRight" });
    document.removeEventListener("keydown", consume);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("stops listening after unmount", () => {
    const { onSelect, unmount } = setup(null);
    unmount();
    fireEvent.keyDown(window, { key: "ArrowRight", code: "ArrowRight" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("scrolls the active grid item into view when the selection changes", () => {
    const item = document.createElement("button");
    item.className = "provider-grid__item provider-grid__item--active";
    const scrollIntoView = vi.fn();
    item.scrollIntoView = scrollIntoView;
    document.body.append(item);
    const { rerender } = setup(null);
    scrollIntoView.mockClear();
    rerender({ selected: "claude" });
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest", inline: "nearest" });
  });
});
