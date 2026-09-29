import { useState } from "react";
import type { LocaleKey } from "../../../../i18n/keys";
import { setProviderOptionalDetails } from "../../../../lib/tauri";

interface Props {
  providerId: string;
  enabled: boolean;
  available: boolean;
  disabled: boolean;
  t: (key: LocaleKey) => string;
  onChanged: () => void;
}

interface Copy {
  label: LocaleKey;
  helper: LocaleKey;
}

/** Copy for the one optional breakdown each supporting provider offers. */
const OPTIONAL_DETAILS_COPY: Record<string, Copy> = {
  litellm: {
    label: "ProviderLiteLLMModelActivity",
    helper: "ProviderLiteLLMModelActivityHelper",
  },
  claude: {
    label: "ProviderClaudeWorkspaceSpend",
    helper: "ProviderClaudeWorkspaceSpendHelper",
  },
};

/** Opt-in for a provider's extra breakdown (LiteLLM model activity, Claude workspace spend). */
export function OptionalDetailsSection({
  providerId,
  enabled,
  available,
  disabled,
  t,
  onChanged,
}: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const copy = OPTIONAL_DETAILS_COPY[providerId];
  if (!available || !copy) return null;

  const handleChange = async (next: boolean) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await setProviderOptionalDetails(providerId, next);
      onChanged();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="provider-detail-section">
      <h4>{t("ProviderOptionalDetailsTitle")}</h4>
      <label className="provider-detail-toggle">
        <input
          type="checkbox"
          checked={enabled}
          disabled={disabled || busy}
          onChange={(event) => void handleChange(event.target.checked)}
        />
        <span>
          <span className="provider-detail-toggle__label">{t(copy.label)}</span>
          <span className="provider-detail-toggle__helper">{t(copy.helper)}</span>
        </span>
      </label>
      {error && <div className="provider-detail-error">{error}</div>}
    </section>
  );
}
