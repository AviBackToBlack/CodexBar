import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrencyRates, getSettingsSnapshot } from "../lib/tauri";
import { formatDisplayCurrency } from "../lib/currency";

interface CurrencyContextValue {
  preferredCode: string;
  rates: Record<string, number>;
  format: (amount: number | null | undefined, sourceCode: string, sourceSymbol?: string | null) => string;
}

const EMPTY_RATES: Record<string, number> = {};

const CurrencyContext = createContext<CurrencyContextValue>({
  preferredCode: "AUTO",
  rates: EMPTY_RATES,
  format: (amount, sourceCode, sourceSymbol) =>
    formatDisplayCurrency(amount, sourceCode, "AUTO", EMPTY_RATES, sourceSymbol),
});

export function CurrencyProvider({ children }: { children: ReactNode }) {
  const [preferredCode, setPreferredCode] = useState("AUTO");
  const [rates, setRates] = useState<Record<string, number>>(EMPTY_RATES);
  const requestId = useRef(0);

  // Single subscription path: the backend's `settings-changed` broadcast
  // (the same event `useSettings` consumes). Re-fetch the settings snapshot
  // and reload rates only when the preference actually changed.
  const reloadForPreference = useCallback((preference: string) => {
    const selected = preference.trim().toUpperCase() || "AUTO";
    setPreferredCode(selected);
    if (selected === "AUTO") {
      requestId.current += 1;
      setRates(EMPTY_RATES);
      return;
    }
    const id = ++requestId.current;
    void getCurrencyRates(selected)
      .then((snapshot) => {
        if (requestId.current === id) setRates(snapshot.rates);
      })
      .catch(() => {
        // Keep the last known table; exchange-rate availability never blocks
        // app surfaces. The backend merged fallback rates into the snapshot.
      });
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;

    void getSettingsSnapshot()
      .then((settings) => {
        if (!cancelled) reloadForPreference(settings.preferredCurrencyCode ?? "AUTO");
      })
      .catch(() => {});

    listen("settings-changed", () => {
      void getSettingsSnapshot()
        .then((settings) => {
          if (!cancelled) reloadForPreference(settings.preferredCurrencyCode ?? "AUTO");
        })
        .catch(() => {});
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [reloadForPreference]);

  const format = useCallback(
    (amount: number | null | undefined, sourceCode: string, sourceSymbol?: string | null) =>
      formatDisplayCurrency(amount, sourceCode, preferredCode, rates, sourceSymbol),
    [preferredCode, rates],
  );

  return <CurrencyContext.Provider value={{ preferredCode, rates, format }}>{children}</CurrencyContext.Provider>;
}

export function useCurrency(): CurrencyContextValue {
  return useContext(CurrencyContext);
}
