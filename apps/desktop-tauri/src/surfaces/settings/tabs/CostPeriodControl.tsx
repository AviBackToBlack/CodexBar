import { useEffect, useRef, useState } from "react";
import type { LocaleKey } from "../../../i18n/keys";
import {
  MAX_ROLLING_DAYS,
  PRESET_COST_PERIODS,
  costPeriodLabel,
  customPeriodRaw,
  isPresetCostPeriod,
  rollingDays,
} from "../../../lib/costPeriod";

const CUSTOM_OPTION = "custom";

/**
 * History window picker: month to date, all, 1/7/30/90/365 days, or a custom
 * day count in 1..=365. `value` is the persisted form (`rolling:N`,
 * `month-to-date`, `all`). A custom count is only reported once it is valid.
 */
export default function CostPeriodControl({
  value,
  onChange,
  disabled = false,
  t,
}: {
  value: string;
  onChange: (raw: string) => void;
  disabled?: boolean;
  t: (key: LocaleKey) => string;
}) {
  const [customMode, setCustomMode] = useState(() => !isPresetCostPeriod(value));
  const [customText, setCustomText] = useState(() => String(rollingDays(value) ?? ""));
  const typedRaw = useRef<string | null>(null);

  // Follow external changes (settings loaded after mount, another window).
  // Values this control just emitted while typing a custom count are skipped so
  // typing "1" on the way to "14" does not collapse the custom input.
  useEffect(() => {
    if (value === typedRaw.current) return;
    if (isPresetCostPeriod(value)) {
      setCustomMode(false);
    } else {
      setCustomMode(true);
      setCustomText(String(rollingDays(value) ?? ""));
    }
  }, [value]);

  const customInvalid = customMode && customText !== "" && customPeriodRaw(customText) === null;

  return (
    <div className="settings-section__group" style={{ marginBottom: 16 }}>
      <label className="settings-section__label" htmlFor="cost-history-window">
        {t("CostPeriodHistoryWindow")}
      </label>
      <p className="settings-section__caption">{t("CostPeriodHistoryWindowHelper")}</p>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <select
          id="cost-history-window"
          className="settings-select"
          value={customMode ? CUSTOM_OPTION : value}
          disabled={disabled}
          onChange={(event) => {
            const next = event.target.value;
            if (next === CUSTOM_OPTION) {
              // Nothing is saved until a valid count is typed.
              setCustomMode(true);
              setCustomText(String(rollingDays(value) ?? ""));
              return;
            }
            typedRaw.current = null;
            setCustomMode(false);
            onChange(next);
          }}
        >
          {PRESET_COST_PERIODS.map((raw) => (
            <option key={raw} value={raw}>
              {costPeriodLabel(raw, t)}
            </option>
          ))}
          <option value={CUSTOM_OPTION}>{t("CostPeriodCustom")}</option>
        </select>
        {customMode && (
          <input
            type="number"
            className="number-input"
            inputMode="numeric"
            min={1}
            max={MAX_ROLLING_DAYS}
            step={1}
            value={customText}
            disabled={disabled}
            aria-label={t("CostPeriodCustomDays")}
            aria-invalid={customInvalid}
            onChange={(event) => {
              const text = event.target.value;
              setCustomText(text);
              const raw = customPeriodRaw(text);
              if (raw && raw !== value) {
                typedRaw.current = raw;
                onChange(raw);
              }
            }}
          />
        )}
      </div>
    </div>
  );
}
