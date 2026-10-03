import { BarChart } from "../../../../../components/charts/BarChart";
import { providerCostColor } from "../../../../../components/charts/chartPalette";
import { incompleteRequestsTooltip } from "../../../../../lib/incompleteRequests";
import type { LocaleKey } from "../../../../../i18n/keys";
import type { DailyCostPoint } from "../../../../../types/bridge";

interface Props {
  data: DailyCostPoint[];
  title: string;
  ariaLabel: string;
  providerId: string;
  animations: boolean;
  emptyMessage: string;
  t: (key: LocaleKey) => string;
}

/**
 * Port target: the cost_history bar cluster in
 * `rust/src/native_ui/preferences.rs::render_provider_detail_panel`.
 * Phase 10 wires through per-provider palette tokens + animation flags.
 */
export function CostHistoryChart({
  data,
  title,
  ariaLabel,
  providerId,
  animations,
  emptyMessage,
  t,
}: Props) {
  const recent = data.slice(-30);
  const points = recent.map((p) => ({
    label: p.date,
    value: p.value,
    incompleteNote: incompleteRequestsTooltip(t, p.incompleteRequestCount) ?? undefined,
  }));
  return (
    <div className="provider-detail-chart">
      <div className="provider-detail-chart__title">{title}</div>
      <BarChart
        data={points}
        color={providerCostColor(providerId)}
        ariaLabel={ariaLabel}
        valueFormatter={(v) => `$${v.toFixed(2)}`}
        animations={animations}
        emptyMessage={emptyMessage}
      />
    </div>
  );
}
