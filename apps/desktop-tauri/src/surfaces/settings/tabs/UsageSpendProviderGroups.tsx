import { useState } from "react";
import { formatUsd } from "../../../lib/usageSpendSharing";
import {
  PROVIDER_MODEL_DISPLAY_LIMIT,
  type SpendProviderGroup,
} from "../../../lib/usageSpendBreakdown";
import type { LocaleKey } from "../../../i18n/keys";

interface Props {
  groups: SpendProviderGroup[];
  /** Names the History window the model rows cover. */
  caption: string;
  t: (key: LocaleKey) => string;
}

/**
 * Upstream 0.64.0 (#3353): models grouped by provider, one collapsible group
 * per provider with its spend and its models.
 */
export function UsageSpendProviderGroups({ groups, caption, t }: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [expandedModels, setExpandedModels] = useState<Set<string>>(() => new Set());
  const toggle = (set: Set<string>, id: string) => {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  };

  return (
    <div className="settings-section__group" style={{ marginTop: 20 }}>
      <h4 style={{ margin: 0 }}>{t("UsageSpendModels")}</h4>
      <p className="settings-section__caption">{caption}</p>
      {groups.length === 0 ? (
        <p className="settings-section__caption">{t("UsageSpendNoModels")}</p>
      ) : (
        <div style={{ display: "grid", gap: 6, marginTop: 10 }}>
          {groups.map((group) => {
            const isCollapsed = collapsed.has(group.providerId);
            const showAll = expandedModels.has(group.providerId);
            const models = showAll ? group.models : group.models.slice(0, PROVIDER_MODEL_DISPLAY_LIMIT);
            return (
              <div key={group.providerId} className="provider-detail-section" style={{ padding: "10px 12px" }}>
                <button
                  type="button"
                  aria-expanded={!isCollapsed}
                  onClick={() => setCollapsed((current) => toggle(current, group.providerId))}
                  style={{
                    width: "100%",
                    display: "grid",
                    gridTemplateColumns: "minmax(0, 1fr) auto auto",
                    gap: 12,
                    alignItems: "center",
                    border: 0,
                    padding: 0,
                    background: "transparent",
                    color: "inherit",
                    textAlign: "left",
                    cursor: "pointer",
                  }}
                >
                  <strong>{group.displayName}</strong>
                  <span>{groupMetric(group, t("UsageSpendTokens"))}</span>
                  <span aria-hidden="true">{isCollapsed ? "▸" : "▾"}</span>
                </button>
                {!isCollapsed && (
                  <div style={{ display: "grid", gap: 6, marginTop: 8 }}>
                    {group.hasPartialModelHistory && (
                      <span className="settings-section__caption">{t("UsageSpendPartialModelBreakdown")}</span>
                    )}
                    {models.map((model) => (
                      <div key={model.model} style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) auto", gap: 12 }}>
                        <span style={{ minWidth: 0 }}>
                          {model.model}
                          <span className="settings-section__caption" style={{ display: "block", margin: 0 }}>
                            {model.totalTokens.toLocaleString()} {t("UsageSpendTokens")}
                            {model.customPricing ? " · " + t("UsageSpendCustomPricing") : ""}
                          </span>
                        </span>
                        <span>{model.costUsd == null ? t("UsageSpendUnpriced") : formatUsd(model.costUsd, "USD")}</span>
                      </div>
                    ))}
                    {group.models.length > PROVIDER_MODEL_DISPLAY_LIMIT && (
                      <button
                        type="button"
                        className="credential-btn credential-btn--secondary"
                        style={{ justifySelf: "start" }}
                        onClick={() => setExpandedModels((current) => toggle(current, group.providerId))}
                      >
                        {showAll ? t("UsageSpendShowLess") : `${t("UsageSpendShowAll")} (${group.models.length})`}
                      </button>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function groupMetric(group: SpendProviderGroup, tokenLabel: string): string {
  const parts: string[] = [];
  if (group.costUsd != null) parts.push(`${group.costIsPartial ? "~" : ""}${formatUsd(group.costUsd, "USD")}`);
  if (group.totalTokens != null) parts.push(`${group.totalTokens.toLocaleString()} ${tokenLabel}`);
  return parts.length > 0 ? parts.join(" · ") : "—";
}
