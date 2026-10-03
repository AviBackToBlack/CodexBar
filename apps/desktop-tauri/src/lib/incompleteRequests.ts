import type { LocaleKey } from "../i18n/keys";

type Translate = (key: LocaleKey) => string;

/**
 * Tooltip text for Claude requests that only produced a preliminary proxy
 * usage row (upstream 0.60.5 #3688). Returns null when there is nothing to
 * flag so callers can skip the marker.
 */
export function incompleteRequestsTooltip(
  t: Translate,
  count: number | null | undefined,
): string | null {
  if (count == null || count <= 0) return null;
  return `${t("IncompleteRequestsLabel")} \u00b7 ${t("IncompleteRequestsDetail").replace("{}", String(count))}`;
}
