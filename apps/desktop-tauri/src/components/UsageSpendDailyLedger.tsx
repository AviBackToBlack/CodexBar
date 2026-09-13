import { useLocale } from "../hooks/useLocale";
import type { SpendContract } from "../types/bridge";

const currencyFormatters = new Map<string, Intl.NumberFormat>();

function formatUsd(value: number | null, currency: string): string {
  if (value == null || !Number.isFinite(value)) return "—";
  const code = currency || "USD";
  try {
    let formatter = currencyFormatters.get(code);
    if (!formatter) {
      formatter = new Intl.NumberFormat(undefined, {
        style: "currency",
        currency: code,
        maximumFractionDigits: 2,
      });
      currencyFormatters.set(code, formatter);
    }
    return formatter.format(value);
  } catch {
    return `$${value.toFixed(2)}`;
  }
}

function formatTokens(value: number | null, tokenLabel: string, unknownLabel: string): string {
  if (value == null || !Number.isFinite(value) || value < 0) {
    return unknownLabel;
  }
  return `${value.toLocaleString()} ${tokenLabel}`;
}

export function UsageSpendDailyLedger({
  daily,
}: {
  daily: SpendContract["daily"];
}) {
  const { t } = useLocale();
  const rows = [...daily].sort((a, b) => b.day.localeCompare(a.day));

  return (
    <section className="settings-section__group usage-spend-daily" aria-labelledby="usage-spend-daily-title">
      <h4 id="usage-spend-daily-title">{t("UsageSpendDailyLedger")}</h4>
      <p className="settings-section__caption">{t("UsageSpendDailyLedgerHelper")}</p>
      {rows.length === 0 ? (
        <p className="settings-section__caption">{t("UsageSpendNoDailyData")}</p>
      ) : (
        <div className="usage-spend-daily__table">
          <table className="usage-spend-table" aria-label={t("UsageSpendDailyLedger")}>
            <thead>
              <tr>
                <th>{t("UsageSpendColDate")}</th>
                <th>{t("UsageSpendColCost")}</th>
                <th>{t("UsageSpendColTokens")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((point) => (
                <tr key={point.day}>
                  <td>{point.day}</td>
                  <td>{point.costUsd == null ? t("UsageSpendUnknown") : formatUsd(point.costUsd, "USD")}</td>
                  <td>{formatTokens(point.totalTokens, t("UsageSpendTokens"), t("UsageSpendUnknown"))}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
