import type { ProviderDetail } from "../../../../types/bridge";
import type { LocaleKey } from "../../../../i18n/keys";
import { ProviderIcon } from "../../../../components/providers/ProviderIcon";
import { hideOpenAiApiProjectId } from "../../../../lib/openAiApiIdentity";

interface Props {
  provider: Pick<
    ProviderDetail,
    "id" | "displayName" | "email" | "organization" | "plan" | "authType" | "sourceLabel"
  >;
  subtitle: string;
  t: (key: LocaleKey) => string;
  hidePersonalInfo?: boolean;
}

/**
 * Header block: provider icon + display name + identity rows
 * (account, plan, auth type, data source).
 *
 * Port of the identity portion of
 * `rust/src/native_ui/preferences.rs::render_provider_detail_panel` (~4301).
 */
export function IdentitySection({
  provider,
  subtitle,
  t,
  hidePersonalInfo = false,
}: Props) {
  const hideProjectId = provider.id === "openaiapi" && hidePersonalInfo;
  const rows: { label: string; value: string | null }[] = [
    {
      label: t("Account"),
      value: hideOpenAiApiProjectId(provider.email ?? provider.organization, hideProjectId),
    },
    {
      label: t("Plan"),
      value: hideOpenAiApiProjectId(displayIdentityValue(provider.plan, t), hideProjectId),
    },
    { label: t("AuthType"), value: provider.authType },
    { label: t("DataSource"), value: provider.sourceLabel },
  ];
  const visible = rows.filter(
    (r): r is { label: string; value: string } =>
      !!r.value && r.value.length > 0,
  );

  return (
    <header className="provider-detail-header-block">
      <div className="provider-detail-header">
        <ProviderIcon providerId={provider.id} size={28} />
        <div className="provider-detail-title-group">
          <div className="provider-detail-title">{provider.displayName}</div>
          <div className="provider-detail-subtitle">{subtitle}</div>
        </div>
      </div>
      {visible.length > 0 && (
        <dl className="provider-detail-grid">
          {visible.map((r) => (
            <div key={r.label} style={{ display: "contents" }}>
              <dt>{r.label}</dt>
              <dd>{r.value}</dd>
            </div>
          ))}
        </dl>
      )}
    </header>
  );
}

function displayIdentityValue(
  value: string | null,
  t: (key: LocaleKey) => string,
): string | null {
  if (!value) return null;
  if (value.trim().toLowerCase() === "default_claude_ai")
    return t("ProviderPlanClaudeAi");
  return value;
}
