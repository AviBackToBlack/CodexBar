import { useState } from "react";
import type {
  MetricPreference,
  ProviderDetail,
  SettingsSnapshot,
  SettingsUpdate,
} from "../../../../types/bridge";
import type { LocaleKey } from "../../../../i18n/keys";

interface Props {
  provider: ProviderDetail;
  providerMetrics: SettingsSnapshot["providerMetrics"];
  disabled: boolean;
  t: (key: LocaleKey) => string;
  onChange: (patch: SettingsUpdate) => void;
}

interface MetricOption {
  value: MetricPreference;
  label: string;
}

export function MenuBarMetricSection({
  provider,
  providerMetrics,
  disabled,
  t,
  onChange,
}: Props) {
  const [error, setError] = useState<string | null>(null);
  const selected = providerMetrics[provider.id] ?? "automatic";
  const options = metricOptions(provider, selected, t);

  const handleChange = (value: MetricPreference) => {
    setError(null);
    try {
      onChange({
        providerMetrics: {
          ...providerMetrics,
          [provider.id]: value,
        },
      });
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="provider-detail-section provider-detail-menu-metric">
      <h4>{t("TrayDisplayTitle")}</h4>
      <label className="provider-detail-field">
        <span className="provider-detail-field__label">
          {t("MenuBarMetric")}
        </span>
        <select
          className="provider-detail-select"
          value={selected}
          disabled={disabled}
          onChange={(e) => handleChange(e.target.value as MetricPreference)}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <p className="provider-detail-helper">{t("MenuBarMetricHelper")}</p>
      {error && <p className="provider-detail-error">{error}</p>}
    </section>
  );
}

function metricOptions(
  provider: ProviderDetail,
  selected: MetricPreference,
  t: (key: LocaleKey) => string,
): MetricOption[] {
  const options: MetricOption[] = [
    { value: "automatic", label: t("Automatic") },
    {
      value: "session",
      // Upstream 0.70.0: a primary lane that is not a session window keeps
      // its provider name in the picker (Mistral's "Included API").
      label: provider.primaryMetricLabel ?? t("ProviderSessionLabel"),
    },
  ];

  if (provider.weekly) {
    options.push({ value: "weekly", label: t("ProviderWeeklyLabel") });
  }
  if (provider.modelSpecific) {
    options.push({ value: "model", label: t("DetailWindowModelSpecific") });
  }
  if (provider.tertiary || provider.tertiaryLabelKey) {
    options.push({
      value: "tertiary",
      label: t(
        (provider.tertiaryLabelKey ?? "DetailWindowTertiary") as LocaleKey,
      ),
    });
  }
  // The monthly plan window has its own choice below, so it does not make
  // Extra usage available on its own.
  const extraWindows = provider.extraRateWindows.filter(
    (extra) => extra.id !== provider.monthlyPlanWindowId,
  );
  if (provider.id === "cursor" || extraWindows.length > 0) {
    options.push({ value: "extraUsage", label: t("ExtraUsage") });
  }
  // Upstream 0.70.0 (#4072): providers that publish a monthly plan
  // allowance window offer it for the menu bar and widgets.
  if (provider.monthlyPlanWindowId) {
    options.push({ value: "monthlyPlan", label: t("MetricMonthlyPlan") });
  }
  if (provider.id === "gemini" && provider.weekly) {
    options.push({ value: "average", label: t("Average") });
  }
  if (!options.some((option) => option.value === selected)) {
    const labelKey = SAVED_ONLY_LABEL_KEYS[selected];
    options.push({
      value: selected,
      label: labelKey ? t(labelKey) : selected,
    });
  }

  return options;
}

/** Labels for saved choices the provider no longer offers. */
const SAVED_ONLY_LABEL_KEYS: Partial<Record<MetricPreference, LocaleKey>> = {
  credits: "CreditsLabel",
  extraUsage: "ExtraUsage",
  monthlyPlan: "MetricMonthlyPlan",
};
