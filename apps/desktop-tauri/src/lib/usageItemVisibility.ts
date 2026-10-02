export const USAGE_ITEM_METRIC_PREFIX = "metric:";
export const USAGE_ITEM_DETAIL_SECTION_PREFIX = "detailSection:";

/** Stable bridge ID for a metric or extra usage row. */
export function usageItemId(rawMetricId: string): string {
  return `${USAGE_ITEM_METRIC_PREFIX}${rawMetricId}`;
}

/** Stable bridge ID for a provider detail section (upstream 0.62.0 #3638). */
export function detailSectionItemId(rawSectionTitle: string): string {
  return `${USAGE_ITEM_DETAIL_SECTION_PREFIX}${rawSectionTitle}`;
}

export function isUsageItemVisible(
  hiddenUsageItemIds: readonly string[] | undefined,
  rawMetricId: string,
): boolean {
  return !hiddenUsageItemIds?.includes(usageItemId(rawMetricId));
}

/** A detail section is visible unless its title-based ID is hidden. */
export function isDetailSectionVisible(
  hiddenUsageItemIds: readonly string[] | undefined,
  rawSectionTitle: string,
): boolean {
  return !hiddenUsageItemIds?.includes(detailSectionItemId(rawSectionTitle));
}
