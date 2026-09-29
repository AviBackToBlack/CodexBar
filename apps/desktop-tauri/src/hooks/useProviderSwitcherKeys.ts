import { useEffect, useRef } from "react";
import {
  DEFAULT_SWITCHER_SHORTCUTS,
  matchSwitcherAction,
  resolveSwitcherTarget,
  type SwitcherShortcutMap,
} from "../lib/switcherShortcuts";

const EDITABLE_SELECTOR =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="slider"]';
const DRAGGING_SELECTOR = ".provider-grid__item--dragging";
const ACTIVE_ITEM_SELECTOR = ".provider-grid__item--active";

/**
 * Provider-switcher keyboard navigation shared by the tray flyout and the
 * pop-out window. Keys are ignored while focus is in a text field, select or
 * slider (the zoom slider uses the arrow keys) or while a grid drag is active.
 */
export function useProviderSwitcherKeys({
  providerIds,
  selectedProviderId,
  onSelect,
  mapping = DEFAULT_SWITCHER_SHORTCUTS,
}: {
  providerIds: readonly string[];
  selectedProviderId: string | null;
  onSelect: (providerId: string | null) => void;
  mapping?: Readonly<SwitcherShortcutMap>;
}) {
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

  // Keep the selected grid item visible (the dense grid can scroll); this
  // never expands a collapsed grid.
  useEffect(() => {
    document
      .querySelector(ACTIVE_ITEM_SELECTOR)
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [selectedProviderId]);
}
