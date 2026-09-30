import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useChartAnimation } from "./useChartAnimation";

describe("useChartAnimation long series", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("finishes long-series entrance animation after the bounded stagger", () => {
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrame = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      const id = ++nextFrame;
      frames.set(id, callback);
      return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));

    const { result, unmount } = renderHook(() => useChartAnimation(366, true));
    act(() => {
      const first = frames.get(1);
      if (!first) throw new Error("expected initial animation frame");
      frames.delete(1);
      first(0);
    });
    act(() => {
      const next = frames.get(2);
      if (!next) throw new Error("expected follow-up animation frame");
      frames.delete(2);
      next(1_200);
    });

    expect(result.current.running).toBe(false);
    expect(result.current.barProgress(365)).toBe(1);
    unmount();
  });
});
