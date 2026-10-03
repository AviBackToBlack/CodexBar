import { useCallback, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useLocale } from "../../../hooks/useLocale";
import { exportPreferences, importPreferences } from "../../../lib/tauri";

const JSON_FILTERS = [{ name: "JSON", extensions: ["json"] }];

type TransferOutcome =
  | { kind: "success"; message: string }
  | { kind: "error"; message: string };

/**
 * Export or import the portable preferences document. The shell validates the
 * whole file before saving anything, so a rejected import changes nothing.
 */
export default function PreferencesTransferSection() {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<TransferOutcome | null>(null);

  const run = useCallback(
    async (action: () => Promise<string | null>) => {
      setBusy(true);
      setOutcome(null);
      try {
        const message = await action();
        if (message) setOutcome({ kind: "success", message });
      } catch (cause: unknown) {
        setOutcome({
          kind: "error",
          message: cause instanceof Error ? cause.message : String(cause),
        });
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const onExport = useCallback(
    () =>
      run(async () => {
        const path = await save({
          defaultPath: "codexbar-preferences.json",
          filters: JSON_FILTERS,
        });
        if (!path) return null;
        await exportPreferences(path);
        return t("PreferencesExportSuccess");
      }),
    [run, t],
  );

  const onImport = useCallback(
    () =>
      run(async () => {
        const path = await open({ multiple: false, filters: JSON_FILTERS });
        if (typeof path !== "string") return null;
        await importPreferences(path);
        return t("PreferencesImportSuccess");
      }),
    [run, t],
  );

  return (
    <section className="settings-section">
      <h3 className="settings-section__title settings-section__title--bold">
        {t("SectionPreferencesTransfer")}
      </h3>
      <p className="settings-section__caption">
        {t("PreferencesTransferCaption")}
      </p>
      <div
        className="settings-section__group"
        style={{ display: "flex", gap: 8 }}
      >
        <button
          type="button"
          className="credential-btn"
          disabled={busy}
          onClick={() => void onExport()}
        >
          {t("PreferencesExportButton")}
        </button>
        <button
          type="button"
          className="credential-btn credential-btn--secondary"
          disabled={busy}
          onClick={() => void onImport()}
        >
          {t("PreferencesImportButton")}
        </button>
      </div>
      {outcome?.kind === "success" && (
        <p className="settings-section__hint" role="status">
          {outcome.message}
        </p>
      )}
      {outcome?.kind === "error" && (
        <p className="settings-section__error" role="alert">
          {outcome.message}
        </p>
      )}
    </section>
  );
}
