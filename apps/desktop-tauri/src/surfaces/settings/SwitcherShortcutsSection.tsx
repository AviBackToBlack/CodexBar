import { useState } from "react";
import { Field } from "../../components/FormControls";
import { ShortcutCapture } from "../../components/ShortcutCapture";
import { useLocale } from "../../hooks/useLocale";
import type { LocaleKey } from "../../i18n/keys";
import {
  SWITCHER_ACTIONS,
  SWITCHER_SHORTCUT_NONE,
  SwitcherShortcutError,
  resolveSwitcherShortcuts,
  shortcutFromEvent,
  switcherShortcutOverrides,
  validateSwitcherShortcuts,
  type SwitcherAction,
  type SwitcherShortcutErrorCode,
} from "../../lib/switcherShortcuts";
import type { TabProps } from "./settingsTabs";

const ERROR_KEYS: Record<SwitcherShortcutErrorCode, LocaleKey> = {
  unknown: "SwitcherShortcutErrorUnknown",
  duplicate: "SwitcherShortcutErrorDuplicate",
  reserved: "SwitcherShortcutErrorReserved",
  invalid: "SwitcherShortcutErrorInvalid",
};

/** Editor for the provider-switcher keys (Settings > Menu). */
export default function SwitcherShortcutsSection({
  settings,
  set,
  saving,
}: TabProps) {
  const { t } = useLocale();
  const [error, setError] = useState<{
    action: SwitcherAction;
    code: SwitcherShortcutErrorCode;
  } | null>(null);
  const mapping = resolveSwitcherShortcuts(settings.switcherShortcuts);

  const assign = (action: SwitcherAction, shortcut: string) => {
    try {
      const next = validateSwitcherShortcuts({ ...mapping, [action]: shortcut });
      setError(null);
      set({ switcherShortcuts: switcherShortcutOverrides(next) });
    } catch (caught) {
      if (!(caught instanceof SwitcherShortcutError)) throw caught;
      setError({ action, code: caught.code });
    }
  };

  const reset = () => {
    setError(null);
    set({ switcherShortcuts: {} });
  };

  const label = (action: SwitcherAction) => {
    if (action === "previous") return t("SwitcherShortcutPrevious");
    if (action === "next") return t("SwitcherShortcutNext");
    return `${t("SwitcherShortcutSelect")} ${action.slice("select".length)}`;
  };

  return (
    <section className="settings-section">
      <h3 className="settings-section__title">{t("SwitcherShortcutsTitle")}</h3>
      <p className="settings-section__caption">{t("SwitcherShortcutsHelper")}</p>
      <div className="settings-section__group">
        {SWITCHER_ACTIONS.map((action) => {
          const value = mapping[action];
          return (
            <Field key={action} label={label(action)}>
              <ShortcutCapture
                value={value === SWITCHER_SHORTCUT_NONE ? "" : value}
                disabled={saving}
                compose={shortcutFromEvent}
                recordingHint={t("SwitcherShortcutRecordingHint")}
                emptyLabel={t("SwitcherShortcutNone")}
                accessibleLabel={label(action)}
                onCommit={(shortcut) => assign(action, shortcut)}
                onClear={() => assign(action, SWITCHER_SHORTCUT_NONE)}
              />
            </Field>
          );
        })}
      </div>
      {error && (
        <p className="settings-section__error" role="alert">
          {label(error.action)}: {t(ERROR_KEYS[error.code])}
        </p>
      )}
      <button
        type="button"
        className="credential-btn"
        disabled={saving}
        onClick={reset}
      >
        {t("SwitcherShortcutReset")}
      </button>
    </section>
  );
}
