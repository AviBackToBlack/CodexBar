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

const AUTOMATIC_ONLY_PROVIDERS: ReadonlySet<string> = new Set(["aixy"]);

export function MenuBarMetricSection({
  provider,
  providerMetrics,
  disabled,
  t,
  onChange,
}: Props) {
  const [error, setError] = useState<string | null>(null);
  const selected = AUTOMATIC_ONLY_PROVIDERS.has(provider.id)
    ? "automatic"
    : providerMetrics[provider.id] ?? "automatic";
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
  // Aixy's primary budget depends on which limits currently apply to the key,
  // so a fixed session/weekly lane would be misleading. Offer Automatic only.
  if (AUTOMATIC_ONLY_PROVIDERS.has(provider.id)) {
    return [{ value: "automatic", label: t("Automatic") }];
  }

  const options: MetricOption[] = [
    { value: "automatic", label: t("Automatic") },
    { value: "session", label: provider.primaryLabel || t("ProviderSessionLabel") },
  ];

  if (provider.weekly) {
    options.push({ value: "weekly", label: provider.secondaryLabel || t("ProviderWeeklyLabel") });
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
  if (provider.id === "cursor" || provider.extraRateWindows.length > 0) {
    options.push({ value: "extraUsage", label: t("ExtraUsage") });
  }
  if (provider.id === "mistral") {
    options.push({ value: "monthlyPlan", label: t("MistralMonthlySpend") });
  }
  if (provider.id === "gemini" && provider.weekly) {
    options.push({ value: "average", label: t("Average") });
  }
  return withSelected(options, selected, t);
}

/** Keep a previously saved preference visible even if it is no longer offered. */
function withSelected(
  options: MetricOption[],
  selected: MetricPreference,
  t: (key: LocaleKey) => string,
): MetricOption[] {
  if (options.some((option) => option.value === selected)) return options;
  return [
    ...options,
    {
      value: selected,
      label: selected === "credits" ? t("CreditsLabel") : selected,
    },
  ];
}
