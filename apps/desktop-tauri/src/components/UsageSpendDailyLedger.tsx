import { useLocale } from "../hooks/useLocale";
import type { SpendContract } from "../types/bridge";
import {
  formatSpendTokens,
  formatUsd,
} from "../lib/usageSpendSharing";

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
                  <td>{formatSpendTokens(point.totalTokens, t("UsageSpendTokens"), t("UsageSpendUnknown"))}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
