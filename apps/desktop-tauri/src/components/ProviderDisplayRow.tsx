import type { ProviderDisplayDetail } from "../types/bridge";

const MAX_ROWS_PER_SECTION = 24;

export interface ProviderDisplayDetailGroup {
  id: number;
  title: string | null;
  rows: ProviderDisplayDetail[];
}

/** Group consecutive provider details and cap each rendered section. */
export function groupProviderDisplayDetails(
  details: ProviderDisplayDetail[],
): ProviderDisplayDetailGroup[] {
  const groups: ProviderDisplayDetailGroup[] = [];
  for (const detail of details) {
    const title = detail.sectionTitle ?? null;
    const current = groups[groups.length - 1];
    if (
      current &&
      current.title === title &&
      current.rows.length < MAX_ROWS_PER_SECTION
    ) {
      current.rows.push(detail);
    } else {
      groups.push({ id: groups.length, title, rows: [detail] });
    }
  }
  return groups;
}

/**
 * One transient provider detail line: "{title}: {value} [secondary]"
 * plus an optional clamped progress bar.
 *
 * Shared by the tray menu card (`MenuCardDetails`) and the settings provider
 * detail (`UsageSection`); each surface passes its own layout classes.
 */
export function ProviderDisplayRow({
  detail,
  lineClassName,
  secondaryClassName,
  trackClassName,
  fillClassName,
}: {
  detail: ProviderDisplayDetail;
  lineClassName: string;
  secondaryClassName?: string;
  trackClassName: string;
  fillClassName: string;
}) {
  const progress = detail.progress;
  const progressPercent =
    progress &&
    Number.isFinite(progress.used) &&
    Number.isFinite(progress.total) &&
    progress.total > 0
      ? Math.max(0, Math.min(100, (progress.used / progress.total) * 100))
      : null;

  return (
    <div>
      <div className={lineClassName}>
        <span>{detail.title}: {detail.value}</span>
        {detail.secondaryValue && secondaryClassName && (
          <>
            {" "}
            <span className={secondaryClassName}>{detail.secondaryValue}</span>
          </>
        )}
      </div>
      {progressPercent != null && (
        <div
          className={trackClassName}
          role="progressbar"
          aria-label={`${detail.title} progress`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={progressPercent}
        >
          <div className={fillClassName} style={{ width: `${progressPercent}%` }} />
        </div>
      )}
    </div>
  );
}
