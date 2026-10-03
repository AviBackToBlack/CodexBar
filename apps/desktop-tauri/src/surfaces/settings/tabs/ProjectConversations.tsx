import { useState } from "react";
import type { LocaleKey } from "../../../i18n/keys";
import type { CodexWorkspacesSessionUsage } from "../../../types/bridge";

/** Conversations shown before the expand control, and the most ever shown. */
export const CONVERSATION_INITIAL_ROWS = 8;
export const CONVERSATION_MAX_ROWS = 50;

interface ProjectConversationsProps {
  /** Already ranked by the backend: cost descending, unpriced last. */
  conversations: CodexWorkspacesSessionUsage[];
  t: (key: LocaleKey) => string;
}

export default function ProjectConversations({ conversations, t }: ProjectConversationsProps) {
  const [showAll, setShowAll] = useState(false);
  const capped = conversations.slice(0, CONVERSATION_MAX_ROWS);
  const visible = showAll ? capped : capped.slice(0, CONVERSATION_INITIAL_ROWS);

  return (
    <div style={{ display: "grid", gap: 5, marginTop: 9, paddingTop: 8, borderTop: "1px solid var(--border-subtle)" }}>
      {visible.map((session, index) => (
        <div
          key={session.id}
          style={{ display: "grid", gridTemplateColumns: "auto minmax(0, 1fr) auto", gap: 10 }}
        >
          <span className="settings-section__caption" style={{ minWidth: "2.5ch", textAlign: "right" }}>
            #{index + 1}
          </span>
          <span
            title={session.displayTitle}
            style={{ minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {session.displayTitle}
          </span>
          <span>
            {session.costEstimate.unknownTokens > 0 ? "~" : ""}
            ${session.costEstimate.knownUsd.toFixed(2)}
          </span>
        </div>
      ))}
      {capped.length > CONVERSATION_INITIAL_ROWS && (
        <button
          type="button"
          className="credential-btn credential-btn--secondary"
          style={{ justifySelf: "start" }}
          aria-expanded={showAll}
          onClick={() => setShowAll((value) => !value)}
        >
          {showAll ? t("UsageSpendShowLess") : `${t("UsageSpendShowAll")} (${capped.length})`}
        </button>
      )}
    </div>
  );
}
