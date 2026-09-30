import type { ProviderDisplayDetail } from "../types/bridge";
import type { LocaleKey } from "../i18n/keys";
import { providerDisplayDetailTitle } from "../lib/providerLabels";

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
  t,
}: {
  detail: ProviderDisplayDetail;
  lineClassName: string;
  secondaryClassName?: string;
  trackClassName: string;
  fillClassName: string;
  t: (key: LocaleKey) => string;
}) {
  const title = providerDisplayDetailTitle(detail, t);
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
        <span>{title}: {detail.value}</span>
        {detail.secondaryValue && secondaryClassName && (
          <span className={secondaryClassName}>{detail.secondaryValue}</span>
        )}
      </div>
      {progressPercent != null && (
        <div className={trackClassName} aria-label={`${title} progress`}>
          <div className={fillClassName} style={{ width: `${progressPercent}%` }} />
        </div>
      )}
    </div>
  );
}
