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

  it.each([false, true])(
    "renders detail-backed amounts as a detail line, not reset text (resetsAt: %s)",
    async (hasReset) => {
      const detail = provider();
      const resetsAt = hasReset
        ? new Date(Date.now() + 3 * 60 * 60 * 1000 + 30_000).toISOString()
        : null;
      detail.session = {
        ...rateWindow(75),
        resetsAt,
        resetDescription: "19.17 EUR / 25.50 EUR · 6.33 EUR remaining",
        descriptionIsDetail: true,
      };
      detail.extraRateWindows = [
        {
          id: "mistral-monthly-plan",
          title: "Monthly Plan",
          window: {
            ...rateWindow(13),
            resetsAt,
            resetDescription: "34.07 EUR / 255.00 EUR · 220.93 EUR remaining",
            descriptionIsDetail: true,
          },
        },
      ];

      const { container } = render(
        <LocaleProvider>
          <UsageSection provider={detail} resetTimeRelative={true} t={(key) => key} />
        </LocaleProvider>,
      );

      const detailLines = await screen.findAllByText(/EUR remaining/);
      expect(detailLines.map((line) => line.textContent)).toEqual([
        "19.17 EUR / 25.50 EUR · 6.33 EUR remaining",
        "34.07 EUR / 255.00 EUR · 220.93 EUR remaining",
      ]);
      detailLines.forEach((line) => expect(line).toHaveClass("provider-usage-bar__detail"));
      expect(screen.queryByText(/Resets .*EUR/)).not.toBeInTheDocument();
      expect(container.querySelectorAll(".provider-usage-bar__reset")).toHaveLength(
        hasReset ? 2 : 0,
      );
    },
  );

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
});
