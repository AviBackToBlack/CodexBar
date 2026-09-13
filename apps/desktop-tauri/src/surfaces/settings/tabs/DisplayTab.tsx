import { useEffect, useState } from "react";
import { useLocale } from "../../../hooks/useLocale";
import { Field, Select, Toggle } from "../../../components/FormControls";
import type {
  MenuBarDisplayMode,
  OverviewLayout,
  TrayIconMode,
  TrayVisibilityStatusDto,
} from "../../../types/bridge";
import type { TabProps } from "../settingsTabs";
import FloatBarSettingsSection from "../../../floatbar/SettingsSection";
import { getTrayVisibilityStatus } from "../../../lib/tauri";

export default function DisplayTab({
  mode = "menu",
  settings,
  set,
  saving,
}: TabProps & { mode?: "menuBar" | "menu" }) {
  const { t } = useLocale();
  const [trayVisibility, setTrayVisibility] = useState<TrayVisibilityStatusDto | null>(null);

  useEffect(() => {
    getTrayVisibilityStatus()
      .then(setTrayVisibility)
      .catch(() => setTrayVisibility(null));
  }, []);

  return (
    <>
      {/* ── Menu bar ─────────────────────────────────────────────── */}
      {mode === "menuBar" && <section className="settings-section">
        <h3 className="settings-section__title">{t("MenuBar")}</h3>
        <div className="settings-section__group">
          <Field
            label={t("TrayIconModeLabel")}
            description={t("TrayIconModeHelper")}
          >
            <Select
              value={settings.trayIconMode}
              disabled={saving}
              options={[
                { value: "single", label: t("TrayIconModeSingle") },
                { value: "perProvider", label: t("TrayIconModePerProvider") },
              ]}
              onChange={(v) => set({ trayIconMode: v as TrayIconMode })}
            />
          </Field>
          <Field
            label={t("ShowProviderIcons")}
            description={t("ShowProviderIconsHelper")}
            leading
          >
            <Toggle
              checked={settings.switcherShowsIcons}
              disabled={saving}
              onChange={(v) => set({ switcherShowsIcons: v })}
            />
          </Field>
          <Field
            label={t("PreferHighestUsage")}
            description={t("PreferHighestUsageHelper")}
            leading
          >
            <Toggle
              checked={settings.menuBarShowsHighestUsage}
              disabled={saving}
              onChange={(v) => set({ menuBarShowsHighestUsage: v })}
            />
          </Field>
          <Field
            label={t("ShowPercentInTray")}
            description={t("ShowPercentInTrayHelper")}
            leading
          >
            <Toggle
              checked={settings.menuBarShowsPercent}
              disabled={saving}
              onChange={(v) => set({ menuBarShowsPercent: v })}
            />
          </Field>
          <Field
            label={t("ColorPaceInTray")}
            description={t("ColorPaceInTrayHelper")}
            leading
          >
            <Toggle
              checked={settings.menuBarColorPace ?? false}
              ariaLabel={t("ColorPaceInTray")}
              disabled={saving}
              onChange={(v) => set({ menuBarColorPace: v })}
            />
          </Field>
          <Field
            label={t("DisplayModeLabel")}
            description={t("DisplayModeHelper")}
          >
            <Select
              value={settings.menuBarDisplayMode}
              disabled={saving}
              options={[
                { value: "detailed", label: t("DisplayModeDetailed") },
                { value: "compact", label: t("DisplayModeCompact") },
                { value: "minimal", label: t("DisplayModeMinimal") },
              ]}
              onChange={(v) =>
                set({ menuBarDisplayMode: v as MenuBarDisplayMode })
              }
            />
          </Field>
          <Field
            label={t("PromoteTrayIconLabel")}
            description={
              trayVisibility?.support === "supported"
                ? t("PromoteTrayIconHelper")
                : t("PromoteTrayIconUnsupportedHint")
            }
            leading
          >
            <Toggle
              checked={settings.promoteTrayIcon ?? false}
              disabled={saving || trayVisibility?.support !== "supported"}
              onChange={(v) => set({ promoteTrayIcon: v })}
            />
          </Field>
        </div>
      </section>}

      {/* ── Menu content ─────────────────────────────────────────── */}
      {mode === "menu" && <section className="settings-section">
        <h3 className="settings-section__title">{t("TabMenu")}</h3>
        <div className="settings-section__group">
          <Field
            label={t("TrayPanelAlwaysOnTopLabel")}
            description={t("TrayPanelAlwaysOnTopHelper")}
            leading
          >
            <Toggle
              checked={settings.trayPanelAlwaysOnTop}
              ariaLabel={t("TrayPanelAlwaysOnTopLabel")}
              disabled={saving}
              onChange={(v) => set({ trayPanelAlwaysOnTop: v })}
            />
          </Field>
          <Field
            label={t("ShowAsUsedLabel")}
            description={t("ShowAsUsedHelper")}
            leading
          >
            <Toggle
              checked={settings.showAsUsed}
              disabled={saving}
              onChange={(v) => set({ showAsUsed: v })}
            />
          </Field>
          <Field
            label={t("OverviewLayoutLabel")}
            description={t("OverviewLayoutHelper")}
          >
            <Select
              value={settings.overviewLayout}
              disabled={saving}
              options={[
                { value: "detailed", label: t("OverviewLayoutDetailed") },
                { value: "compact", label: t("OverviewLayoutCompact") },
              ]}
              onChange={(v) => set({ overviewLayout: v as OverviewLayout })}
            />
          </Field>
          <Field
            label={t("ShowAllTokenAccountsLabel")}
            description={t("ShowAllTokenAccountsHelper")}
            leading
          >
            <Toggle
              checked={settings.showAllTokenAccountsInMenu}
              disabled={saving}
              onChange={(v) => set({ showAllTokenAccountsInMenu: v })}
            />
          </Field>
          <Field
            label={t("ResetTimeRelative")}
            description={t("ResetTimeRelativeHelper")}
            leading
          >
            <Toggle
              checked={settings.resetTimeRelative}
              disabled={saving}
              onChange={(v) => set({ resetTimeRelative: v })}
            />
          </Field>
          <Field
            label={t("ShowResetWhenExhausted")}
            description={t("ShowResetWhenExhaustedHelper")}
            leading
          >
            <Toggle
              checked={settings.showResetWhenExhausted}
              ariaLabel={t("ShowResetWhenExhausted")}
              disabled={saving}
              onChange={(v) => set({ showResetWhenExhausted: v })}
            />
          </Field>
          <Field label={t("ShowPace")} description={t("ShowPaceHelper")} leading>
            <Toggle
              checked={settings.showPace ?? true}
              ariaLabel={t("ShowPace")}
              disabled={saving}
              onChange={(v) => set({ showPace: v })}
            />
          </Field>
        </div>
      </section>}

      {mode === "menu" && (
        <FloatBarSettingsSection settings={settings} saving={saving} set={set} />
      )}
    </>
  );
}
