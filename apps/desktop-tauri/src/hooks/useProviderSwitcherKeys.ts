import { useEffect, useMemo, useRef } from "react";
import {
  matchSwitcherAction,
  resolveSwitcherShortcuts,
  resolveSwitcherTarget,
} from "../lib/switcherShortcuts";

const EDITABLE_SELECTOR =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="slider"]';
const DRAGGING_SELECTOR = ".provider-grid__item--dragging";

/**
 * Provider-switcher keyboard navigation for the tray flyout (the retired
 * pop-out layout no longer has a surface). `providerIds` must be the list the
 * grid displays, in display order. Keys are ignored while focus is in a text field, select or
 * slider (the zoom slider uses the arrow keys) or while a grid drag is active.
 */
export function useProviderSwitcherKeys({
  providerIds,
  selectedProviderId,
  onSelect,
  shortcuts,
}: {
  providerIds: readonly string[];
  selectedProviderId: string | null;
  onSelect: (providerId: string | null) => void;
  /** `settings.switcherShortcuts`; defaults apply when omitted or invalid. */
  shortcuts?: Readonly<Record<string, string>>;
}) {
  const mapping = useMemo(() => resolveSwitcherShortcuts(shortcuts), [shortcuts]);
  const latest = useRef({ providerIds, selectedProviderId, onSelect, mapping });
  latest.current = { providerIds, selectedProviderId, onSelect, mapping };

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const target = event.target;
      if (target instanceof Element && target.closest(EDITABLE_SELECTOR)) return;
      if (document.querySelector(DRAGGING_SELECTOR)) return;
      const current = latest.current;
      const action = matchSwitcherAction(event, current.mapping);
      if (action === null) return;
      const next = resolveSwitcherTarget(
        action,
        current.providerIds,
        current.selectedProviderId,
      );
      if (next === null) return;
      event.preventDefault();
      current.onSelect(next.providerId);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
}
