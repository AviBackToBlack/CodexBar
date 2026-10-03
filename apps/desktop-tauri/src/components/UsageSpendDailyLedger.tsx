import { useState } from "react";
import { useLocale } from "../hooks/useLocale";
import type { SpendContract } from "../types/bridge";
import {
  formatSpendTokens,
  formatUsd,
} from "../lib/usageSpendSharing";

type DailyPoints = SpendContract["daily"];

/** Upstream `SpendDailyLedger.collapsedRowCount`: long ranges start with the newest 30 days. */
export const DAILY_LEDGER_COLLAPSED_ROWS = 30;

/**
 * Newest-first ledger rows. Collapsed shows the newest `collapsedRowCount` days; expanded shows the
 * complete history (upstream `spendDailyLedgerVisibleSummaries`).
 */
export function dailyLedgerVisibleRows(
  daily: DailyPoints,
  showsAllRows: boolean,
  collapsedRowCount: number = DAILY_LEDGER_COLLAPSED_ROWS,
): DailyPoints {
  const newestFirst = [...daily].sort((a, b) => b.day.localeCompare(a.day));
  return showsAllRows ? newestFirst : newestFirst.slice(0, collapsedRowCount);
}

export function UsageSpendDailyLedger({
  daily,
}: {
  daily: DailyPoints;
}) {
  const { t } = useLocale();
  const [showsAllRows, setShowsAllRows] = useState(false);
  const rows = dailyLedgerVisibleRows(daily, showsAllRows);

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
      {daily.length > DAILY_LEDGER_COLLAPSED_ROWS && (
        <button
          type="button"
          className="credential-btn credential-btn--secondary usage-spend-daily__toggle"
          aria-expanded={showsAllRows}
          onClick={() => setShowsAllRows((value) => !value)}
        >
          {showsAllRows ? t("UsageSpendShowLess") : `${t("UsageSpendShowAll")} (${daily.length})`}
        </button>
      )}
    </section>
  );
}
