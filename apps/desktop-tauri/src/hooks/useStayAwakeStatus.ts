import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getStayAwakeStatus } from "../lib/tauri";

const STAY_AWAKE_CHANGED = "stay-awake-changed";

export function useStayAwakeStatus(): boolean {
  const [held, setHeld] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let updates = 0;
    let unlisten = () => {};

    const initialize = async () => {
      try {
        unlisten = await listen<boolean>(STAY_AWAKE_CHANGED, (event) => {
          updates += 1;
          if (!cancelled) setHeld(event.payload);
        });
      } catch {
        // The status query below still provides a useful initial value.
      }
      if (cancelled) {
        unlisten();
        return;
      }

      const observedUpdates = updates;
      try {
        const current = await getStayAwakeStatus();
        if (!cancelled && updates === observedUpdates) setHeld(current);
      } catch {
        // Keep the last event value if the status query is unavailable.
      }
    };

    void initialize();
    return () => {
      cancelled = true;
      unlisten();
    };
  }, []);

  return held;
}
