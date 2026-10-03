import { describe, expect, it } from "vitest";
import { incompleteRequestsTooltip } from "./incompleteRequests";

const t = (key: string) =>
  key === "IncompleteRequestsDetail" ? "{} requests excluded" : "Incomplete";

describe("incompleteRequestsTooltip", () => {
  it("returns null when nothing was excluded", () => {
    expect(incompleteRequestsTooltip(t, undefined)).toBeNull();
    expect(incompleteRequestsTooltip(t, 0)).toBeNull();
  });

  it("names the marker and the excluded request count", () => {
    expect(incompleteRequestsTooltip(t, 3)).toBe("Incomplete · 3 requests excluded");
  });
});
