import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ProviderDetail } from "../../../../types/bridge";
import { MenuBarMetricSection } from "./MenuBarMetricSection";

function provider(extra = true): ProviderDetail {
  return {
    id: "copilot",
    displayName: "GitHub Copilot",
    enabled: true,
    autoResumeAfterQuotaReset: false,
    autoResumeSupported: false,
    email: null,
    plan: null,
    authType: null,
    sourceLabel: null,
    organization: null,
    lastUpdated: null,
    session: null,
    weekly: null,
    modelSpecific: null,
    tertiary: null,
    extraRateWindows: extra
      ? [{ id: "additional_budget", title: "Additional Budget", window: rateWindow(42) }]
      : [],
    cost: null,
    pace: null,
    lastError: null,
    errorState: null,
    dashboardUrl: null,
    statusPageUrl: null,
    buyCreditsUrl: null,
    hasSnapshot: true,
    cookieSource: null,
    region: null,
  };
}

function rateWindow(usedPercent: number) {
  return {
    usedPercent,
    remainingPercent: 100 - usedPercent,
    windowMinutes: null,
    resetsAt: null,
    resetDescription: null,
    isExhausted: false,
    reservePercent: null,
    reserveDescription: null,
  };
}

describe("MenuBarMetricSection", () => {
  it("renders the provider-declared tertiary label key before observation", () => {
    const base = provider(false);
    base.id = "opencodego";
    base.displayName = "OpenCode Go";
    base.tertiary = null;
    base.tertiaryLabelKey = "ProviderMonthly";
    const onChange = vi.fn();
    const { rerender } = render(
      <MenuBarMetricSection
        provider={base}
        providerMetrics={{}}
        disabled={false}
        t={(key) => key}
        onChange={onChange}
      />,
    );

    expect(screen.getByRole("option", { name: "ProviderMonthly" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "tertiary" } });
    expect(onChange).toHaveBeenCalledWith({
      providerMetrics: { opencodego: "tertiary" },
    });

    const observed = { ...base, tertiary: rateWindow(37) };
    rerender(
      <MenuBarMetricSection
        provider={observed}
        providerMetrics={{}}
        disabled={false}
        t={(key) => key}
        onChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("option", { name: "ProviderMonthly" })).toBeInTheDocument();
  });

  it("keeps the generic tertiary label when no provider key is declared", () => {
    const base = provider(false);
    base.tertiary = rateWindow(37);

    render(
      <MenuBarMetricSection
        provider={base}
        providerMetrics={{}}
        disabled={false}
        t={(key) => key}
        onChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("option", { name: "DetailWindowTertiary" })).toBeInTheDocument();
  });

  it("offers extra usage when a provider has extra rate windows", () => {
    const onChange = vi.fn();
    render(
      <MenuBarMetricSection
        provider={provider()}
        providerMetrics={{}}
        disabled={false}
        t={(key) => key}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "extraUsage" } });

    expect(screen.getByRole("option", { name: "ExtraUsage" })).toBeInTheDocument();
    expect(onChange).toHaveBeenCalledWith({
      providerMetrics: { copilot: "extraUsage" },
    });
  });

  it("offers the provider-declared lane labels in the metric picker", () => {
    const base = provider(false);
    base.id = "litellm";
    base.displayName = "LiteLLM";
    base.weekly = rateWindow(30);
    base.primaryLabel = "Personal budget";
    base.secondaryLabel = "Team budget";

    render(
      <MenuBarMetricSection
        provider={base}
        providerMetrics={{}}
        disabled={false}
        t={(key) => key}
        onChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("option", { name: "Personal budget" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Team budget" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "ProviderSessionLabel" })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "ProviderWeeklyLabel" })).not.toBeInTheDocument();
  });

  it("keeps the generic metric labels when the provider declares none", () => {
    const base = provider(false);
    base.weekly = rateWindow(30);

    render(
      <MenuBarMetricSection
        provider={base}
        providerMetrics={{}}
        disabled={false}
        t={(key) => key}
        onChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("option", { name: "Automatic" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "ProviderSessionLabel" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "ProviderWeeklyLabel" })).toBeInTheDocument();
  });
});
