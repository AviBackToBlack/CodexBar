import { describe, expect, it } from "vitest";
import type { ProviderDisplayDetail } from "../types/bridge";
import { displaySectionHeading } from "./ProviderDisplayRow";

function row(id: string, section?: string | null): ProviderDisplayDetail {
  return { id, title: id, value: "1", secondaryValue: null, progress: null, section };
}

describe("displaySectionHeading", () => {
  it("prints a heading only where the section changes", () => {
    const rows = [
      row("plain"),
      row("a", "Model activity"),
      row("b", "Model activity"),
      row("c", "Workspace spend"),
      row("d", null),
    ];

    expect(rows.map((_, index) => displaySectionHeading(rows, index))).toEqual([
      null,
      "Model activity",
      null,
      "Workspace spend",
      null,
    ]);
  });
});
