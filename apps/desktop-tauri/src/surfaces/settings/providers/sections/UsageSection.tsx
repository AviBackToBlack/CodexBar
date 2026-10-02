import type {
  ProviderDisplayDetail,
  ProviderInventoryItem,
  ProviderDetail,
  RateWindowSnapshot,
} from "../../../../types/bridge";
import { InventoryItemRow } from "../../../../components/InventoryRows";
import { ProviderDisplayRow } from "../../../../components/ProviderDisplayRow";
import type { LocaleKey } from "../../../../i18n/keys";
import { useFormattedResetTime } from "../../../../hooks/useFormattedResetTime";
import { useMonthlyLimitBlockNow } from "../../../../hooks/useMonthlyLimitBlockNow";
import { isMonthlyLimitBlockActive } from "../../../../lib/monthlyLimitBlock";
import { isUsageItemVisible } from "../../../../lib/usageItemVisibility";

interface Props {
  provider: ProviderDetail;
  resetTimeRelative: boolean;
  t: (key: LocaleKey) => string;
}

interface BarSpec {
  key: string;
  label: string;
  rate: RateWindowSnapshot;
}

/**
 * Stacked usage bars — session / weekly / model-specific / tertiary.
 * Mirrors the bars in
 * `rust/src/native_ui/preferences.rs::render_provider_detail_panel`.
 */
export function UsageSection({ provider, resetTimeRelative, t }: Props) {
  const bars: BarSpec[] = [];
  if (provider.session && isUsageItemVisible(provider.hiddenUsageItemIds, "primary")) {
    bars.push({
      key: "session",
      label: provider.primaryLabel || t("ProviderSessionLabel"),
      rate: provider.session,
    });
  }
  if (provider.weekly && isUsageItemVisible(provider.hiddenUsageItemIds, "secondary")) {
    bars.push({
      key: "weekly",
      label: provider.secondaryLabel || t("ProviderWeeklyLabel"),
      rate: provider.weekly,
    });
  }
  if (provider.modelSpecific && isUsageItemVisible(provider.hiddenUsageItemIds, "model-specific")) {
    bars.push({
      key: "modelSpecific",
      label: t("DetailWindowModelSpecific"),
      rate: provider.modelSpecific,
    });
  }
  if (provider.tertiary && isUsageItemVisible(provider.hiddenUsageItemIds, "tertiary")) {
    bars.push({
      key: "tertiary",
      label: t("DetailWindowTertiary"),
      rate: provider.tertiary,
    });
  }
  for (const extra of provider.extraRateWindows ?? []) {
    if (!isUsageItemVisible(provider.hiddenUsageItemIds, `extra-${extra.id}`)) {
      continue;
    }
    bars.push({
      key: extra.id,
      label: extra.title,
      rate: extra.window,
    });
  }

  // Cached snapshots outlive the pool reset; re-check blocks on the clock.
  const monthlyLimitBlockNow = useMonthlyLimitBlockNow(
    bars.map((bar) => bar.rate.monthlyLimitBlock),
  );
  const inventory = provider.inventory ?? [];
  const displayDetails = provider.displayDetails ?? [];
  if (bars.length === 0 && inventory.length === 0 && displayDetails.length === 0) {
    return null;
  }

  return (
    <section className="provider-detail-section">
      <h4>{t("ProviderUsage")}</h4>
      {bars.map((b) => (
        <UsageBar
          key={b.key}
          label={b.label}
          rate={b.rate}
          blocked={isMonthlyLimitBlockActive(b.rate.monthlyLimitBlock, monthlyLimitBlockNow)}
          resetTimeRelative={resetTimeRelative}
          t={t}
        />
      ))}
      {inventory.map((item) => (
        <InventoryItemRow
          key={item.id}
          item={item}
          resetTimeRelative={resetTimeRelative}
          lineClassName="provider-usage-inventory"
        />
      ))}
      {displayDetails.map((detail) => (
        <ProviderDisplayRow
          key={detail.id}
          detail={detail}
          t={t}
          lineClassName="provider-usage-inventory"
          trackClassName="provider-usage-bar__track"
          fillClassName="provider-usage-bar__fill"
        />
      ))}
    </section>
  );
}

function UsageBar({
  label,
  rate,
  blocked,
  resetTimeRelative,
  t,
}: {
  label: string;
  rate: RateWindowSnapshot;
  blocked: boolean;
  resetTimeRelative: boolean;
  t: (key: LocaleKey) => string;
}) {
  const usedPct = Number.isFinite(rate.usedPercent) ? Math.max(0, rate.usedPercent) : 0;
  const pct = Math.min(100, usedPct);
  const isInformational = rate.isInformational === true;
  const formattedReset = useFormattedResetTime(
    blocked ? null : rate.resetsAt,
    blocked ? null : rate.resetDescription,
    resetTimeRelative,
  );
  if (blocked) {
    // Upstream 0.69.0 #4091 status row: title and status, no bar or reset.
    return (
      <div className="provider-usage-bar provider-usage-bar--blocked">
        <div className="provider-usage-bar__header">
          <span className="provider-usage-bar__label">{label}</span>
          <span className="provider-usage-bar__status">{t("PanelBlockedByMonthlyLimit")}</span>
        </div>
      </div>
    );
  }
  const resetHint = formattedReset
    ? resetTimeRelative
      ? formattedReset
      : `${t("MetricResetsIn")} ${formattedReset}`
    : null;

  return (
    <div className="provider-usage-bar">
      <div className="provider-usage-bar__header">
        <span className="provider-usage-bar__label">{label}</span>
        <span
          className="provider-usage-bar__pct"
          data-exhausted={rate.isExhausted || undefined}
        >
          {isInformational
            ? rate.resetDescription?.trim() || formattedReset || "—"
            : rate.isExhausted
            ? usedPct > 100
              ? `${usedPct.toFixed(0)}%`
              : t("DetailWindowExhausted")
            : `${usedPct.toFixed(0)}%`}
        </span>
      </div>
      {!isInformational && (
        <div className="provider-usage-bar__track">
          <div
            className="provider-usage-bar__fill"
            style={{ width: `${pct}%` }}
            data-exhausted={rate.isExhausted || undefined}
          />
        </div>
      )}
      {!isInformational && resetHint && (
        <span className="provider-usage-bar__reset">{resetHint}</span>
      )}
    </div>
  );
}
