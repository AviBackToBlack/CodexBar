import { useEffect, useRef } from "react";
import { matchSwitcherAction, resolveSwitcherTarget } from "../lib/switcherShortcuts";

const EDITABLE_SELECTOR =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="slider"]';
const DRAGGING_SELECTOR = ".provider-grid__item--dragging";

/**
 * Provider-switcher keyboard navigation shared by the tray flyout and the
 * pop-out window. `providerIds` must be the list the grid displays, in
 * display order. Keys are ignored while focus is in a text field, select or
 * slider (the zoom slider uses the arrow keys) or while a grid drag is active.
 */
export function useProviderSwitcherKeys({
  providerIds,
  selectedProviderId,
  onSelect,
}: {
  providerIds: readonly string[];
  selectedProviderId: string | null;
  onSelect: (providerId: string | null) => void;
}) {
  const latest = useRef({ providerIds, selectedProviderId, onSelect });
  latest.current = { providerIds, selectedProviderId, onSelect };

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const target = event.target;
      if (target instanceof Element && target.closest(EDITABLE_SELECTOR)) return;
      if (document.querySelector(DRAGGING_SELECTOR)) return;
      const current = latest.current;
      const action = matchSwitcherAction(event);
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
