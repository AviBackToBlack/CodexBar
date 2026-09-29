/**
 * Provider-switcher keyboard shortcuts (upstream CodexBar 0.67.0,
 * `ProviderSwitcherShortcuts`). Shortcuts are menu-local strings such as
 * `left`, `ctrl+3` or `ctrl+shift+right`; `none` disables an action.
 *
 * Grammar is upstream's, with one Windows difference: `cmd` is accepted as an
 * alias for `ctrl` and normalized to `ctrl`, so documents exported from macOS
 * stay portable. Windows reserves `ctrl+r`, `ctrl+q`, `ctrl+,` and `ctrl+w`
 * (the tray/pop-out commands). `alt+f4` and `alt+tab` need no entry: F4 and
 * Tab are outside the grammar, so they are rejected as invalid.
 */

export const SWITCHER_ACTIONS = [
  "previous",
  "next",
  "select1",
  "select2",
  "select3",
  "select4",
  "select5",
  "select6",
  "select7",
  "select8",
  "select9",
] as const;

export type SwitcherAction = (typeof SWITCHER_ACTIONS)[number];
export type SwitcherShortcutMap = Record<SwitcherAction, string>;

export const SWITCHER_SHORTCUT_NONE = "none";

export const DEFAULT_SWITCHER_SHORTCUTS: Readonly<SwitcherShortcutMap> = {
  previous: "left",
  next: "right",
  select1: "ctrl+1",
  select2: "ctrl+2",
  select3: "ctrl+3",
  select4: "ctrl+4",
  select5: "ctrl+5",
  select6: "ctrl+6",
  select7: "ctrl+7",
  select8: "ctrl+8",
  select9: "ctrl+9",
};

export type SwitcherShortcutErrorCode =
  | "unknown"
  | "duplicate"
  | "reserved"
  | "invalid";

/** `code` selects the localized message shown by the editor. */
export class SwitcherShortcutError extends Error {
  readonly code: SwitcherShortcutErrorCode;

  constructor(code: SwitcherShortcutErrorCode) {
    super(`switcher shortcut error: ${code}`);
    this.name = "SwitcherShortcutError";
    this.code = code;
  }
}

const MODIFIER_ORDER = ["ctrl", "alt", "shift"] as const;
const NAMED_KEYS = new Set(["left", "right", ","]);
const RESERVED = new Set(["ctrl+r", "ctrl+q", "ctrl+,", "ctrl+w"]);

function isSwitcherAction(value: string): value is SwitcherAction {
  return (SWITCHER_ACTIONS as readonly string[]).includes(value);
}

/** Canonicalize one shortcut string, or throw `SwitcherShortcutError`. */
export function normalizeShortcut(shortcut: string): string {
  const parts = shortcut
    .toLowerCase()
    .split("+")
    .map((part) => part.trim());
  if (parts.length === 1 && parts[0] === SWITCHER_SHORTCUT_NONE) {
    return SWITCHER_SHORTCUT_NONE;
  }
  const key = parts[parts.length - 1];
  const modifiers = parts
    .slice(0, -1)
    .map((part) => (part === "cmd" ? "ctrl" : part));
  if (
    new Set(modifiers).size !== modifiers.length ||
    !modifiers.every((part) =>
      (MODIFIER_ORDER as readonly string[]).includes(part),
    ) ||
    !(NAMED_KEYS.has(key) || /^[a-z0-9]$/.test(key))
  ) {
    throw new SwitcherShortcutError("invalid");
  }
  const result = [
    ...MODIFIER_ORDER.filter((part) => modifiers.includes(part)),
    key,
  ].join("+");
  const isArrow = key === "left" || key === "right";
  if (
    RESERVED.has(result) ||
    (!isArrow && modifiers.every((part) => part === "shift"))
  ) {
    throw new SwitcherShortcutError("reserved");
  }
  return result;
}

/**
 * Overlay `overrides` on the defaults and validate the result: unknown
 * actions and duplicate assignments are rejected; omitted actions keep their
 * default. Returns the fully resolved, normalized map.
 */
export function validateSwitcherShortcuts(
  overrides: Readonly<Record<string, string>> = {},
): SwitcherShortcutMap {
  const result: SwitcherShortcutMap = { ...DEFAULT_SWITCHER_SHORTCUTS };
  for (const [action, shortcut] of Object.entries(overrides)) {
    if (!isSwitcherAction(action)) throw new SwitcherShortcutError("unknown");
    result[action] = normalizeShortcut(shortcut);
  }
  const assigned = Object.values(result).filter(
    (value) => value !== SWITCHER_SHORTCUT_NONE,
  );
  if (new Set(assigned).size !== assigned.length) {
    throw new SwitcherShortcutError("duplicate");
  }
  return result;
}

export type SwitcherKeyEvent = Pick<
  KeyboardEvent,
  "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey"
>;

/**
 * Canonical shortcut string for a key event, or null when the event cannot
 * be expressed in the grammar (bare modifier, Windows key, punctuation other
 * than `,`, ...). Letters and digits come from `code` so Shift+1 stays `1`.
 */
export function shortcutFromEvent(event: SwitcherKeyEvent): string | null {
  if (event.metaKey) return null;
  let key: string | undefined = /^(?:Digit|Key)([0-9A-Z])$/
    .exec(event.code)?.[1]
    ?.toLowerCase();
  if (key === undefined) {
    if (event.key === "ArrowLeft") key = "left";
    else if (event.key === "ArrowRight") key = "right";
    else if (/^[a-z0-9,]$/i.test(event.key)) key = event.key.toLowerCase();
  }
  if (key === undefined) return null;
  const modifiers = [
    event.ctrlKey && "ctrl",
    event.altKey && "alt",
    event.shiftKey && "shift",
  ].filter((part): part is string => part !== false);
  return [...modifiers, key].join("+");
}

/** The action bound to this key event under `mapping`, or null. */
export function matchSwitcherAction(
  event: SwitcherKeyEvent,
  mapping: Readonly<SwitcherShortcutMap> = DEFAULT_SWITCHER_SHORTCUTS,
): SwitcherAction | null {
  const shortcut = shortcutFromEvent(event);
  if (shortcut === null) return null;
  return SWITCHER_ACTIONS.find((action) => mapping[action] === shortcut) ?? null;
}

/** Result of resolving an action; `providerId: null` means Overview. */
export interface SwitcherTarget {
  providerId: string | null;
}

/**
 * Resolve `action` against the switcher order (Overview first, then
 * `providerIds` in display order). previous/next wrap around; `selectN` is
 * the Nth segment with Overview as 1. Returns null when there is no target
 * (N past the last segment, or nothing to switch to).
 */
export function resolveSwitcherTarget(
  action: SwitcherAction,
  providerIds: readonly string[],
  selectedProviderId: string | null,
): SwitcherTarget | null {
  const segments: (string | null)[] = [null, ...providerIds];
  if (action === "previous" || action === "next") {
    if (segments.length < 2) return null;
    const current = Math.max(0, segments.indexOf(selectedProviderId));
    const step = action === "next" ? 1 : -1;
    const index = (current + step + segments.length) % segments.length;
    return { providerId: segments[index] };
  }
  const index = Number(action.slice("select".length)) - 1;
  return index < segments.length ? { providerId: segments[index] } : null;
}
