import type { LocaleKey } from "../i18n/keys";
import type { ProviderDisplayDetail } from "../types/bridge";

type Translate = (key: LocaleKey) => string;

const DISPLAY_DETAIL_TITLE_KEYS: ReadonlyMap<string, LocaleKey> = new Map<string, LocaleKey>([
  ["atlascloud-available", "AtlasCloudAvailableBalance"],
]);

const COST_PERIOD_KEYS: ReadonlyMap<string, LocaleKey> = new Map<string, LocaleKey>([
  ["atlascloud:Atlas Cloud balance", "AtlasCloudBalance"],
]);

export function providerDisplayDetailTitle(
  detail: ProviderDisplayDetail,
  t: Translate,
): string {
  const key = DISPLAY_DETAIL_TITLE_KEYS.get(detail.id);
  return key ? t(key) : detail.title;
}

export function providerCostPeriodTitle(
  providerId: string,
  period: string,
  t: Translate,
): string {
  const key = COST_PERIOD_KEYS.get(`${providerId}:${period}`);
  return key ? t(key) : period;
}
