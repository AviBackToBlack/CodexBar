import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../../../i18n/LocaleProvider";
import { buildBundle } from "../../../../test/localeHarness";
import type { ProviderDetail } from "../../../../types/bridge";
import { UsageSection } from "./UsageSection";

const tauriMocks = vi.hoisted(() => ({
  getLocaleStrings: vi.fn(),
  setUiLanguage: vi.fn(),
}));

const eventMocks = vi.hoisted(() => ({
  listen: vi.fn(),
}));

vi.mock("../../../../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../lib/tauri")>()),
  ...tauriMocks,
}));
vi.mock("@tauri-apps/api/event", () => eventMocks);

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

function provider(): ProviderDetail {
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
    session: rateWindow(20),
    weekly: null,
    modelSpecific: null,
    tertiary: null,
    extraRateWindows: [
      { id: "additional_budget", title: "Additional Budget", window: rateWindow(42) },
    ],
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

describe("UsageSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.getLocaleStrings.mockResolvedValue(
      buildBundle({ InventoryAvailableCount: "{} available" }),
    );
    eventMocks.listen.mockResolvedValue(() => {});
  });

  it("renders extra Copilot budget windows in settings", async () => {
    render(
      <LocaleProvider>
        <UsageSection provider={provider()} resetTimeRelative={true} t={(key) => key} />
      </LocaleProvider>,
    );

    expect(await screen.findByText("Additional Budget")).toBeInTheDocument();
    expect(screen.getByText("42%")).toBeInTheDocument();
  });

  it("filters only hidden metric and extra rows", async () => {
    const detail = provider();
    detail.weekly = rateWindow(30);
    detail.hiddenUsageItemIds = [
      "metric:primary",
      "metric:extra-additional_budget",
    ];

    render(
      <LocaleProvider>
        <UsageSection provider={detail} resetTimeRelative={true} t={(key) => key} />
      </LocaleProvider>,
    );

    await screen.findByText("ProviderUsage");
    expect(screen.queryByText("ProviderSessionLabel")).not.toBeInTheDocument();
    expect(screen.queryByText("Additional Budget")).not.toBeInTheDocument();
  });

  it("marks an unavailable session without rendering a quota bar", async () => {
    const detail = provider();
    detail.session = {
      ...rateWindow(0),
      isInformational: true,
      resetDescription: "No active 5h session",
    };

    render(
      <LocaleProvider>
        <UsageSection provider={detail} resetTimeRelative={true} t={(key) => key} />
      </LocaleProvider>,
    );

    const label = await screen.findByText("ProviderSessionLabel");
    expect(label.parentElement).toHaveTextContent("No active 5h session");
    expect(label.parentElement?.querySelector(".provider-usage-bar__track")).toBeNull();
  });

  function kimiDetailBlockedUntil(blockResetsAt: string) {
    const detail = provider();
    detail.id = "kimi";
    detail.displayName = "Kimi";
    detail.session = {
      ...rateWindow(0),
      resetsAt: new Date(Date.now() + 3 * 60 * 60 * 1000).toISOString(),
      monthlyLimitBlock: { resetsAt: blockResetsAt },
    };
    detail.extraRateWindows = [
      {
        id: "kimi-monthly",
        title: "Total usage",
        window: { ...rateWindow(100), isExhausted: true, resetsAt: blockResetsAt },
      },
    ];
    return detail;
  }

  it("shows a window blocked by the monthly pool as title and status only", async () => {
    const poolReset = new Date(Date.now() + 20 * 24 * 60 * 60 * 1000).toISOString();

    render(
      <LocaleProvider>
        <UsageSection
          provider={kimiDetailBlockedUntil(poolReset)}
          resetTimeRelative={true}
          t={(key) => key}
        />
      </LocaleProvider>,
    );

    const label = await screen.findByText("ProviderSessionLabel");
    const row = label.closest(".provider-usage-bar");
    expect(row).toHaveTextContent(/^ProviderSessionLabelPanelBlockedByMonthlyLimit$/);
    expect(row?.querySelector(".provider-usage-bar__track")).toBeNull();
    expect(row?.querySelector(".provider-usage-bar__reset")).toBeNull();
    // The pool's own row keeps its bar, exhausted state and reset.
    const pool = screen.getByText("Total usage").closest(".provider-usage-bar");
    expect(pool?.querySelector(".provider-usage-bar__track")).not.toBeNull();
    expect(pool).toHaveTextContent("DetailWindowExhausted");
    expect(pool?.querySelector(".provider-usage-bar__reset")).not.toBeNull();
  });

  it("shows the raw window again once the monthly pool reset has passed", async () => {
    const past = new Date(Date.now() - 60 * 1000).toISOString();

    render(
      <LocaleProvider>
        <UsageSection
          provider={kimiDetailBlockedUntil(past)}
          resetTimeRelative={true}
          t={(key) => key}
        />
      </LocaleProvider>,
    );

    const label = await screen.findByText("ProviderSessionLabel");
    const row = label.closest(".provider-usage-bar");
    expect(row).toHaveTextContent("0%");
    expect(row).not.toHaveTextContent("PanelBlockedByMonthlyLimit");
    expect(row?.querySelector(".provider-usage-bar__track")).not.toBeNull();
  });

  it("renders discrete inventory without turning it into a quota bar", async () => {
    const detail = provider();
    detail.session = null;
    detail.extraRateWindows = [];
    detail.inventory = [
      {
        id: "reset-credits",
        title: "Limit Reset Credits",
        availableCount: 2,
        nextExpiresAt: "2099-01-01T00:00:00Z",
      },
    ];

    const { container } = render(
      <LocaleProvider>
        <UsageSection provider={detail} resetTimeRelative={true} t={(key) => key} />
      </LocaleProvider>,
    );

    expect(await screen.findByText(/Limit Reset Credits: 2 available/)).toBeInTheDocument();
    expect(container.querySelector(".provider-usage-bar__track")).toBeNull();
  });

  it("shows provider-declared lane labels in settings bars", async () => {
    const detail = provider();
    detail.weekly = rateWindow(30);
    detail.primaryLabel = "Personal budget";
    detail.secondaryLabel = "Team budget";

    render(
      <LocaleProvider>
        <UsageSection provider={detail} resetTimeRelative={true} t={(key) => key} />
      </LocaleProvider>,
    );

    expect(await screen.findByText("Personal budget")).toBeInTheDocument();
    expect(screen.getByText("Team budget")).toBeInTheDocument();
    expect(screen.queryByText("ProviderSessionLabel")).not.toBeInTheDocument();
    expect(screen.queryByText("ProviderWeeklyLabel")).not.toBeInTheDocument();
  });

  it("keeps the generic labels when the provider declares none", async () => {
    const detail = provider();
    detail.weekly = rateWindow(30);

    render(
      <LocaleProvider>
        <UsageSection provider={detail} resetTimeRelative={true} t={(key) => key} />
      </LocaleProvider>,
    );

    expect(await screen.findByText("ProviderSessionLabel")).toBeInTheDocument();
    expect(screen.getByText("ProviderWeeklyLabel")).toBeInTheDocument();
  });
});
