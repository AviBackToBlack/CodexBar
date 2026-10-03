import type {
  ProviderUsageSnapshot,
  RateWindowSnapshot,
} from "../types/bridge";

/** Selects the automatic window for surfaces rendering exactly one primary/secondary usage metric. */
export function selectSingleMetricUsageWindow(
  provider: Pick<ProviderUsageSnapshot, "primary" | "secondary">,
): RateWindowSnapshot {
  const { primary, secondary } = provider;
  return primary.isInformational && secondary && !secondary.isInformational
    ? secondary
    : primary;
}

/**
 * Reset-wording fallback for a window. A detail-backed description (for
 * example spend amounts) is never reset text, so it must not reach
 * `normalizeResetDescription` through the reset formatter.
 */
export function resetDescriptionFallback(
  window: Pick<RateWindowSnapshot, "resetDescription" | "descriptionIsDetail">,
): string | null {
  return window.descriptionIsDetail ? null : window.resetDescription;
}

/** Secondary detail line for a detail-backed window, or null. */
export function windowDetailText(
  window: Pick<RateWindowSnapshot, "resetDescription" | "descriptionIsDetail" | "isInformational">,
): string | null {
  if (!window.descriptionIsDetail || window.isInformational) return null;
  return window.resetDescription?.trim() || null;
}
