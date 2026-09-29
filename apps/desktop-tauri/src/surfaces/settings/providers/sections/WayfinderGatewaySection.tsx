import type { LocaleKey } from "../../../../i18n/keys";

export type GatewayProviderId = "wayfinder" | "bifrost" | "aixy";

interface GatewayCopy {
  title: LocaleKey;
  label: LocaleKey;
  help: LocaleKey;
}

const GATEWAY_COPY: Record<GatewayProviderId, GatewayCopy> = {
  wayfinder: {
    title: "WayfinderGatewayTitle",
    label: "WayfinderGatewayLabel",
    help: "WayfinderGatewayHelp",
  },
  bifrost: {
    title: "BifrostGatewayTitle",
    label: "WayfinderGatewayLabel",
    help: "BifrostGatewayHelp",
  },
  aixy: {
    title: "AixyGatewayTitle",
    label: "AixyGatewayLabel",
    help: "AixyGatewayHelp",
  },
};

export function isGatewayProviderId(id: string): id is GatewayProviderId {
  return Object.prototype.hasOwnProperty.call(GATEWAY_COPY, id);
}

interface Props {
  providerId: GatewayProviderId;
  draft: string;
  error: string | null;
  busy: boolean;
  disabled: boolean;
  onDraftChange: (draft: string) => void;
  onSave: () => void;
  t: (key: LocaleKey) => string;
}

export function WayfinderGatewaySection({
  providerId,
  draft,
  error,
  busy,
  disabled,
  onDraftChange,
  onSave,
  t,
}: Props) {
  const copy = GATEWAY_COPY[providerId];
  return (
    <section className="provider-detail__section">
      <h3>{t(copy.title)}</h3>
      <label>
        <span>{t(copy.label)}</span>
        <input
          type="url"
          value={draft}
          disabled={disabled || busy}
          onChange={(event) => onDraftChange(event.target.value)}
          aria-describedby="wayfinder-gateway-help"
        />
      </label>
      <p id="wayfinder-gateway-help">{t(copy.help)}</p>
      {error && <p role="alert">{error}</p>}
      <button
        type="button"
        disabled={disabled || busy}
        onClick={onSave}
      >
        {t("Save")}
      </button>
    </section>
  );
}
