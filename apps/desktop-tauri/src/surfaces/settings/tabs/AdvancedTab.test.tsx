import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  getSafeDiagnostics: vi.fn(),
  registerGlobalShortcut: vi.fn().mockResolvedValue(undefined),
  unregisterGlobalShortcut: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../../../lib/tauri", () => tauriMocks);
vi.mock("../../../hooks/useLocale", () => ({
  useLocale: () => ({ t: (key: string) => key }),
}));

import AdvancedTab from "./AdvancedTab";
import type { SettingsSnapshot } from "../../../types/bridge";

const settings: SettingsSnapshot = {
  enabledProviders: [],
  refreshIntervalSecs: 300,
  adaptiveRefresh: false,
  refreshAllProvidersOnMenuOpen: false,
  lowPowerMode: false,
  startAtLogin: false,
  startMinimized: false,
  showNotifications: true,
  soundEnabled: true,
  notificationSoundTheme: "windows",
  highUsageThreshold: 70,
  criticalUsageThreshold: 90,
  predictivePaceWarningEnabled: false,
  trayIconMode: "single",
  switcherShowsIcons: true,
  menuBarShowsHighestUsage: true,
  menuBarShowsPercent: true,
  showAsUsed: false,
  showAllTokenAccountsInMenu: true,
  enableAnimations: true,
  resetTimeRelative: true,
  showResetWhenExhausted: false,
  menuBarDisplayMode: "compact",
  notificationSoundPaths: {
    predictiveWarning: null,
    highUsage: null,
    criticalUsage: null,
    exhausted: null,
    statusIssue: null,
    sessionDepleted: null,
    sessionRestored: null,
  },
  hidePersonalInfo: false,
  autoDownloadUpdates: false,
  installUpdatesOnQuit: false,
  globalShortcut: "",
  codexCustomSessionsDirs: [],
  updateChannel: "stable",
  uiLanguage: "english",
  theme: "dark",
  windowScalePercent: 125,
  trayScalePercent: 100,
  powertoysStatusPipeEnabled: false,
  claudeAvoidKeychainPrompts: true,
  codexSparkUsageVisible: true,
  disableKeychainAccess: false,
  providerMetrics: {},
  floatBarEnabled: false,
  floatBarOpacity: 0.9,
  floatBarScale: 100,
  floatBarOrientation: "horizontal",
  floatBarStyle: "floating",
  floatBarClickThrough: false,
  floatBarProviderIds: [],
  floatBarDarkText: false,
  floatBarShowResetInline: false,
  floatBarShowCost: false,
  claudeDailyRoutinesUsageVisible: true,
  claudeAllowReadingClaudeCodeCredentials: false,
  alibabaTokenPlanRegion: "cn",
  weeklyProgressWorkDays: null,
  costSummaryDisplayStyle: "compact",
  providerAccentColors: {},
};

describe("AdvancedTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.getSafeDiagnostics.mockResolvedValue("diagnostics text");
  });

  it("copies safe diagnostics to the clipboard", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(<AdvancedTab settings={settings} set={vi.fn()} saving={false} />);

    const copyButtons = screen.getAllByRole("button", {
      name: "DiagnosticsCopyButton",
    });
    fireEvent.click(copyButtons[copyButtons.length - 1]);

    await waitFor(() => {
      expect(tauriMocks.getSafeDiagnostics).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith("diagnostics text");
    });
    await waitFor(() => {
      expect(screen.getAllByText("DiagnosticsCopied").length).toBeGreaterThan(0);
    });
  });

  it("shows an error when copying diagnostics fails", async () => {
    tauriMocks.getSafeDiagnostics.mockRejectedValue(new Error("invoke failed"));

    render(<AdvancedTab settings={settings} set={vi.fn()} saving={false} />);

    const copyButtons = screen.getAllByRole("button", {
      name: "DiagnosticsCopyButton",
    });
    fireEvent.click(copyButtons[copyButtons.length - 1]);

    await waitFor(() => {
      expect(
        screen.getAllByText(/DiagnosticsCopyFailed/).length,
      ).toBeGreaterThan(0);
    });
  });
});
