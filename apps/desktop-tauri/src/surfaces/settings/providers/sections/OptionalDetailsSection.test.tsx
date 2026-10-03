import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setProviderOptionalDetails } from "../../../../lib/tauri";
import { OptionalDetailsSection } from "./OptionalDetailsSection";

vi.mock("../../../../lib/tauri", () => ({
  setProviderOptionalDetails: vi.fn(),
}));

function renderSection(
  overrides: Partial<Parameters<typeof OptionalDetailsSection>[0]> = {},
) {
  const onChanged = vi.fn();
  const view = render(
    <OptionalDetailsSection
      providerId="litellm"
      enabled={false}
      available={true}
      disabled={false}
      t={(key) => key}
      onChanged={onChanged}
      {...overrides}
    />,
  );
  return { ...view, onChanged };
}

describe("OptionalDetailsSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(setProviderOptionalDetails).mockResolvedValue(undefined);
  });

  it("shows the LiteLLM model activity opt-in and persists a change", async () => {
    const { onChanged } = renderSection();

    expect(screen.getByText("ProviderLiteLLMModelActivity")).toBeInTheDocument();
    const checkbox = screen.getByRole("checkbox");
    expect(checkbox).not.toBeChecked();
    await act(async () => {
      fireEvent.click(checkbox);
    });

    await vi.waitFor(() =>
      expect(setProviderOptionalDetails).toHaveBeenCalledWith("litellm", true),
    );
    expect(onChanged).toHaveBeenCalledOnce();
  });

  it("uses the workspace spend copy for Claude", () => {
    renderSection({ providerId: "claude", enabled: true });

    expect(screen.getByText("ProviderClaudeWorkspaceSpend")).toBeInTheDocument();
    expect(screen.getByRole("checkbox")).toBeChecked();
  });

  it("does not render for providers without an optional breakdown", () => {
    const { container } = renderSection({ providerId: "codex", available: false });

    expect(container).toBeEmptyDOMElement();
  });

  it("surfaces a failed save and keeps the section usable", async () => {
    vi.mocked(setProviderOptionalDetails).mockRejectedValue("save failed");
    const { onChanged } = renderSection();

    await act(async () => {
      fireEvent.click(screen.getByRole("checkbox"));
    });

    expect(await screen.findByText("save failed")).toBeInTheDocument();
    expect(onChanged).not.toHaveBeenCalled();
    expect(screen.getByRole("checkbox")).not.toBeDisabled();
  });
});
