import type { PaceSnapshot } from "../../../../types/bridge";
import type { LocaleKey } from "../../../../i18n/keys";
import { formatEta } from "../../../../lib/formatEta";
import { useMonthlyLimitBlockNow } from "../../../../hooks/useMonthlyLimitBlockNow";
import { isMonthlyLimitBlockActive } from "../../../../lib/monthlyLimitBlock";

interface Props {
  pace: PaceSnapshot | null;
  t: (key: LocaleKey) => string;
}

const STAGE_TO_KEY: Record<PaceSnapshot["stage"], LocaleKey> = {
  on_track: "DetailPaceOnTrack",
  slightly_ahead: "DetailPaceSlightlyAhead",
  ahead: "DetailPaceAhead",
  far_ahead: "DetailPaceFarAhead",
  slightly_behind: "DetailPaceSlightlyBehind",
  behind: "DetailPaceBehind",
  far_behind: "DetailPaceFarBehind",
};

/** 展示配额节奏状态及其辅助说明。 */
export function PaceSection({ pace, t }: Props) {
  const monthlyLimitBlockNow = useMonthlyLimitBlockNow([pace?.monthlyLimitBlock]);
  // Upstream 0.69.0 #4091 drops the pace of a window blocked by a longer
  // exhausted pool; its own resets cannot restore access.
  if (!pace || isMonthlyLimitBlockActive(pace.monthlyLimitBlock, monthlyLimitBlockNow)) {
    return null;
  }

  const stageLabel = t(STAGE_TO_KEY[pace.stage]);
  const aux = pace.willLastToReset
    ? t("DetailPaceWillLastToReset")
    : pace.etaSeconds !== null
      ? `${t("DetailPaceRunsOutIn")} ${formatEta(pace.etaSeconds)}`
      : null;

  return (
    <section className="provider-detail-section provider-detail-pace">
      <h4>{t("DetailPaceTitle")}</h4>
      <div className="provider-detail-pace__stage" data-stage={pace.stage}>
        {stageLabel}
      </div>
      {aux && <div className="provider-detail-pace__aux">{aux}</div>}
    </section>
  );
}
