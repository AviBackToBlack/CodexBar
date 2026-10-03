import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  isGatewayProviderId,
  WayfinderGatewaySection,
  type GatewayProviderId,
} from "./WayfinderGatewaySection";

function renderSection(providerId: GatewayProviderId, onSave = vi.fn()) {
  render(
    <WayfinderGatewaySection
      providerId={providerId}
      draft="https://gateway.example.com"
      error={null}
      busy={false}
      disabled={false}
      onDraftChange={vi.fn()}
      onSave={onSave}
      t={(key) => key}
    />,
  );
  return onSave;
}

describe("WayfinderGatewaySection", () => {
  it.each([
    ["wayfinder", "WayfinderGatewayTitle", "WayfinderGatewayLabel", "WayfinderGatewayHelp"],
    ["bifrost", "BifrostGatewayTitle", "WayfinderGatewayLabel", "BifrostGatewayHelp"],
    ["aixy", "AixyGatewayTitle", "AixyGatewayLabel", "AixyGatewayHelp"],
  ] as const)("uses localized %s copy", (providerId, title, label, help) => {
    renderSection(providerId);

    expect(screen.getByRole("heading", { name: title })).toBeInTheDocument();
    expect(screen.getByLabelText(label)).toHaveValue("https://gateway.example.com");
    expect(screen.getByText(help)).toBeInTheDocument();
  });

  it("saves through the shared button", () => {
    const onSave = renderSection("aixy");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it("recognizes only gateway providers", () => {
    expect(isGatewayProviderId("aixy")).toBe(true);
    expect(isGatewayProviderId("bifrost")).toBe(true);
    expect(isGatewayProviderId("wayfinder")).toBe(true);
    expect(isGatewayProviderId("codex")).toBe(false);
    expect(isGatewayProviderId("toString")).toBe(false);
  });
});
