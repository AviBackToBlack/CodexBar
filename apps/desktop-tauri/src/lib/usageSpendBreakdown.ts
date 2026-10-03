import type { SpendContract, SpendDailyPoint, SpendModelRow } from "../types/bridge";

/** Upstream 0.64.0 (#3353) `spendProviderModelDisplayLimit`. */
export const PROVIDER_MODEL_DISPLAY_LIMIT = 6;

/** One provider group in the Usage & Spend breakdown: its spend plus its models. */
export interface SpendProviderGroup {
  providerId: string;
  displayName: string;
  costUsd: number | null;
  totalTokens: number | null;
  /** True when the cost is a lower bound (unpriced models) and renders with "~". */
  costIsPartial: boolean;
  /** True when model history is incomplete, so the model list is a partial view. */
  hasPartialModelHistory: boolean;
  models: SpendModelRow[];
}

/**
 * Upstream `spendDashboardProviderBreakdowns` ordering: highest known cost
 * first, then highest tokens, unknown values last, then display name.
 */
export function compareSpendProviderGroups(a: SpendProviderGroup, b: SpendProviderGroup): number {
  const byValue = (left: number | null, right: number | null): number => {
    if (left != null && right != null) return left === right ? 0 : right - left;
    if (left != null) return -1;
    if (right != null) return 1;
    return 0;
  };
  return (
    byValue(a.costUsd, b.costUsd) ||
    byValue(a.totalTokens, b.totalTokens) ||
    a.displayName.localeCompare(b.displayName, undefined, { numeric: true, sensitivity: "base" })
  );
}

/**
 * Group the contract's model rows by the provider that produced them.
 *
 * The desktop shell builds model-level history for one provider contract
 * (`contract.providerId`), so that is the only group with models; providers
 * without model-level history stay in the provider table.
 */
export function buildSpendProviderGroups(
  contract: SpendContract,
  providerName: string,
): SpendProviderGroup[] {
  if (contract.models.length === 0) return [];
  const totalTokens = contract.models.reduce((sum, model) => sum + model.totalTokens, 0);
  const group: SpendProviderGroup = {
    providerId: contract.providerId,
    displayName: providerName,
    costUsd: contract.knownCostUsd,
    totalTokens: Number.isSafeInteger(totalTokens) ? totalTokens : null,
    costIsPartial: contract.priceCoverage.unpriced > 0,
    hasPartialModelHistory: !contract.historyCoverageEstablished,
    models: contract.models,
  };
  return [group].sort(compareSpendProviderGroups);
}

/** The daily point for `day`, or null when that day is not in the current range. */
export function findSpendDay(daily: SpendDailyPoint[], day: string | null): SpendDailyPoint | null {
  if (!day) return null;
  return daily.find((point) => point.day === day) ?? null;
}

/** Medium local date for a `YYYY-MM-DD` dashboard day; the raw value when it does not parse. */
export function formatSpendDay(day: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!match) return day;
  const [year, month, date] = [Number(match[1]), Number(match[2]), Number(match[3])];
  const parsed = new Date(year, month - 1, date);
  if (parsed.getFullYear() !== year || parsed.getMonth() !== month - 1 || parsed.getDate() !== date) {
    return day;
  }
  return parsed.toLocaleDateString(undefined, { dateStyle: "medium" });
}
