import { useState } from "react";
import { formatSpendMetric } from "../../../lib/usageSpendSharing";
import { findSpendDay, formatSpendDay } from "../../../lib/usageSpendBreakdown";
import type { SpendContract } from "../../../types/bridge";
import type { LocaleKey } from "../../../i18n/keys";

type TrendSection = "daily" | "hourly";

interface Props {
  contract: SpendContract;
  selectedDay: string | null;
  onSelectDay: (day: string | null) => void;
  t: (key: LocaleKey) => string;
}

/**
 * Upstream 0.64.0 (#3353): one panel with a compact Day/Hour selector in place
 * of separate chart sections, plus a control that clears the selected day.
 */
export function UsageSpendTrendPanel({ contract, selectedDay, onSelectDay, t }: Props) {
  const [section, setSection] = useState<TrendSection>("daily");
  const hasDaily = contract.daily.length > 0;
  const hasHourly = contract.hourlyActivity.length > 0;
  const sections: TrendSection[] = [...(hasDaily ? (["daily"] as const) : []), ...(hasHourly ? (["hourly"] as const) : [])];
  if (sections.length === 0) return null;
  const active = sections.includes(section) ? section : sections[0];
  const day = findSpendDay(contract.daily, selectedDay);

  return (
    <div className="settings-section__group" style={{ marginTop: 14 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12, flexWrap: "wrap" }}>
        <h4 style={{ margin: 0 }}>
          {active === "daily" ? t("UsageSpendDailySpend") : t("UsageSpendHourlyActivity")}
        </h4>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {day && (
            <span
              data-testid="usage-spend-selected-day"
              style={{ display: "inline-flex", alignItems: "center", gap: 4 }}
              className="settings-section__caption"
            >
              {formatSpendDay(day.day)}
              <button
                type="button"
                className="credential-btn credential-btn--secondary"
                aria-label={t("UsageSpendClearSelectedDay")}
                title={t("UsageSpendClearSelectedDay")}
                onClick={() => onSelectDay(null)}
              >
                ×
              </button>
            </span>
          )}
          {sections.length > 1 && (
            <div role="group" aria-label={t("UsageSpendTitle")} style={{ display: "flex", gap: 4 }}>
              {sections.map((value) => (
                <button
                  key={value}
                  type="button"
                  className="credential-btn credential-btn--secondary"
                  aria-pressed={active === value}
                  onClick={() => setSection(value)}
                >
                  {value === "daily" ? t("UsageSpendTrendDaily") : t("UsageSpendTrendHourly")}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
      {active === "daily" ? (
        <DailySpendBars contract={contract} selectedDay={day?.day ?? null} onSelectDay={onSelectDay} t={t} />
      ) : (
        <ActivityHeatmap cells={contract.hourlyActivity} t={t} />
      )}
    </div>
  );
}

function DailySpendBars({
  contract,
  selectedDay,
  onSelectDay,
  t,
}: {
  contract: SpendContract;
  selectedDay: string | null;
  onSelectDay: (day: string | null) => void;
  t: (key: LocaleKey) => string;
}) {
  const max = Math.max(0, ...contract.daily.map((point) => point.costUsd ?? 0));
  return (
    <div style={{ marginTop: 10 }}>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 2, height: 72, overflowX: "auto" }}>
        {contract.daily.map((point) => {
          const selected = point.day === selectedDay;
          const height = point.costUsd == null || max === 0 ? 2 : Math.max(2, (point.costUsd / max) * 72);
          return (
            <button
              key={point.day}
              type="button"
              aria-pressed={selected}
              aria-label={`${formatSpendDay(point.day)} ${formatSpendMetric(point.costUsd, point.totalTokens, "USD", t("UsageSpendTokens"))}`}
              title={`${formatSpendDay(point.day)} · ${formatSpendMetric(point.costUsd, point.totalTokens, "USD", t("UsageSpendTokens"))}`}
              onClick={() => onSelectDay(selected ? null : point.day)}
              style={{
                flex: "1 0 6px",
                minWidth: 6,
                maxWidth: 24,
                height,
                padding: 0,
                border: 0,
                borderRadius: 2,
                cursor: "pointer",
                background: selected ? "var(--accent)" : "var(--accent-muted)",
              }}
            />
          );
        })}
      </div>
      {selectedDay && (
        <p className="settings-section__caption" style={{ marginTop: 6 }}>
          {t("UsageSpendSelectedDayScope")}
        </p>
      )}
    </div>
  );
}

export function ActivityHeatmap({ cells, t }: { cells: SpendContract["hourlyActivity"]; t: (key: LocaleKey) => string }) {
  const lookup = new Map(cells.map((cell) => [`${cell.weekday}:${cell.hour}`, cell.conversations]));
  const max = Math.max(1, ...cells.map((cell) => cell.conversations));
  return (
    <div style={{ marginTop: 10 }}>
      <div
        aria-label={t("UsageSpendHourlyActivity")}
        style={{ display: "grid", gridTemplateColumns: "repeat(24, minmax(7px, 1fr))", gap: 2 }}
      >
        {Array.from({ length: 7 * 24 }, (_, index) => {
          const weekday = Math.floor(index / 24);
          const hour = index % 24;
          const value = lookup.get(`${weekday}:${hour}`) ?? 0;
          const alpha = value === 0 ? 0.08 : 0.18 + (value / max) * 0.72;
          return (
            <span
              key={`${weekday}:${hour}`}
              title={`Day ${weekday + 1}, ${hour}:00 · ${value} conversations`}
              style={{ aspectRatio: "1", borderRadius: 2, background: `rgb(90 160 255 / ${alpha})` }}
            />
          );
        })}
      </div>
    </div>
  );
}
