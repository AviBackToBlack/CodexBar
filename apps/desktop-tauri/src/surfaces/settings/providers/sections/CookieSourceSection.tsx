import { useState } from "react";
import type { CookieSourceOption } from "../../../../types/bridge";
import type { LocaleKey } from "../../../../i18n/keys";
import { setProviderCookieSource } from "../../../../lib/tauri";

interface Props {
  providerId: string;
  currentValue: string | null;
  options: CookieSourceOption[];
  /** Backend flag: Manual is selected but no cookie header is configured. */
  manualCookieMissing?: boolean;
  t: (key: LocaleKey) => string;
  onChanged: () => void;
}

/**
 * Cookie-source segmented picker (Automatic / Manual / Disabled).
 *
 * Port of the per-provider cookie-source ComboBox rows in
 * `rust/src/native_ui/preferences.rs::render_provider_detail_panel`.
 * Hidden when the provider has no options.
 */
export function CookieSourceSection({
  providerId,
  currentValue,
  options,
  manualCookieMissing = false,
  t,
  onChanged,
}: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (options.length === 0) return null;

  const selected = currentValue ?? options[0]?.value ?? "";
  const selectedOption =
    options.find((o) => o.value === selected) ?? options[0] ?? null;

  const handleSelect = async (value: string) => {
    if (value === selected || busy) return;
    setBusy(true);
    setError(null);
    try {
      await setProviderCookieSource(providerId, value);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const showUseAutomatic =
    manualCookieMissing &&
    selected === "manual" &&
    options.some((o) => o.value === "auto");

  return (
    <section className="provider-detail-section provider-detail-cookie-source">
      <h4>{t("ProviderCookieSource")}</h4>
      <div
        role="radiogroup"
        aria-label={t("ProviderCookieSource")}
        className="provider-detail-segmented"
      >
        {options.map((opt) => {
          const isActive = opt.value === selected;
          return (
            <button
              key={opt.value}
              type="button"
              role="radio"
              aria-checked={isActive}
              disabled={busy}
              className={`provider-detail-segmented__option${
                isActive ? " is-active" : ""
              }`}
              onClick={() => void handleSelect(opt.value)}
            >
              {opt.label}
            </button>
          );
        })}
      </div>
      {selectedOption?.description && (
        <p className="provider-detail-helper">{selectedOption.description}</p>
      )}
      {showUseAutomatic && (
        <div className="provider-detail-cookie-source__missing">
          <p className="provider-detail-helper">
            {t("ProviderManualCookieMissing")}
          </p>
          <button
            type="button"
            className="credential-btn"
            disabled={busy}
            onClick={() => void handleSelect("auto")}
          >
            {t("ProviderUseAutomaticCookies")}
          </button>
        </div>
      )}
      {error && <p className="provider-detail-error">{error}</p>}
    </section>
  );
}
