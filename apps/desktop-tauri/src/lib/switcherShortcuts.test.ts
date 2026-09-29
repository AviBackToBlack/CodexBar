import { describe, expect, it } from "vitest";
import {
  DEFAULT_SWITCHER_SHORTCUTS,
  SWITCHER_ACTIONS,
  SwitcherShortcutError,
  matchSwitcherAction,
  normalizeShortcut,
  resolveSwitcherShortcuts,
  resolveSwitcherTarget,
  shortcutFromEvent,
  switcherShortcutOverrides,
  validateSwitcherShortcuts,
  type SwitcherKeyEvent,
} from "./switcherShortcuts";

function codeOf(error: unknown): string | undefined {
  return error instanceof SwitcherShortcutError ? error.code : undefined;
}

function failure(run: () => unknown): string | undefined {
  try {
    run();
  } catch (error) {
    return codeOf(error);
  }
  return undefined;
}

function keyEvent(init: Partial<SwitcherKeyEvent>): SwitcherKeyEvent {
  return {
    key: "",
    code: "",
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...init,
  };
}

describe("defaults", () => {
  it("cover every action with upstream keys (cmd becomes ctrl)", () => {
    expect(Object.keys(DEFAULT_SWITCHER_SHORTCUTS)).toEqual([...SWITCHER_ACTIONS]);
    expect(DEFAULT_SWITCHER_SHORTCUTS.previous).toBe("left");
    expect(DEFAULT_SWITCHER_SHORTCUTS.next).toBe("right");
    expect(DEFAULT_SWITCHER_SHORTCUTS.select9).toBe("ctrl+9");
  });

  it("are valid and unchanged by validation", () => {
    expect(validateSwitcherShortcuts()).toEqual(DEFAULT_SWITCHER_SHORTCUTS);
    expect(validateSwitcherShortcuts({})).toEqual(DEFAULT_SWITCHER_SHORTCUTS);
  });
});

describe("normalizeShortcut", () => {
  it("orders modifiers canonically and lowercases", () => {
    expect(normalizeShortcut("Shift+Ctrl+Right")).toBe("ctrl+shift+right");
    expect(normalizeShortcut(" alt + ctrl + 2 ")).toBe("ctrl+alt+2");
    expect(normalizeShortcut("Alt+,")).toBe("alt+,");
  });

  it("aliases cmd to ctrl", () => {
    expect(normalizeShortcut("alt+cmd+2")).toBe("ctrl+alt+2");
    expect(normalizeShortcut("cmd+3")).toBe("ctrl+3");
  });

  it("accepts none", () => {
    expect(normalizeShortcut("NONE")).toBe("none");
  });

  it("rejects malformed shortcuts", () => {
    for (const bad of ["", "ctrl+", "ctrl+ab", "ctrl+ctrl+1", "ctrl+cmd+1", "meta+1", "ctrl+f1", "ctrl+up", "alt+f4", "alt+tab", "ctrl+none"]) {
      expect(failure(() => normalizeShortcut(bad)), bad).toBe("invalid");
    }
  });

  it("rejects reserved Windows shortcuts", () => {
    for (const reserved of ["ctrl+r", "cmd+q", "ctrl+,", "ctrl+w"]) {
      expect(failure(() => normalizeShortcut(reserved)), reserved).toBe("reserved");
    }
  });

  it("requires a non-shift modifier for letters, digits and comma", () => {
    for (const bare of ["a", "1", ",", "shift+a", "shift+1"]) {
      expect(failure(() => normalizeShortcut(bare)), bare).toBe("reserved");
    }
    expect(normalizeShortcut("alt+a")).toBe("alt+a");
    expect(normalizeShortcut("ctrl+shift+1")).toBe("ctrl+shift+1");
    expect(normalizeShortcut("shift+ctrl+r")).toBe("ctrl+shift+r");
  });

  it("allows arrows with any or no modifiers", () => {
    expect(normalizeShortcut("left")).toBe("left");
    expect(normalizeShortcut("shift+right")).toBe("shift+right");
  });
});

describe("validateSwitcherShortcuts", () => {
  it("overlays overrides and normalizes them", () => {
    const resolved = validateSwitcherShortcuts({ select2: "alt+cmd+2", next: "shift+right" });
    expect(resolved.select2).toBe("ctrl+alt+2");
    expect(resolved.next).toBe("shift+right");
    expect(resolved.previous).toBe("left");
  });

  it("rejects unknown actions", () => {
    expect(failure(() => validateSwitcherShortcuts({ select10: "ctrl+0" }))).toBe("unknown");
  });

  it("rejects duplicates, including against untouched defaults", () => {
    expect(failure(() => validateSwitcherShortcuts({ next: "left" }))).toBe("duplicate");
    expect(failure(() => validateSwitcherShortcuts({ select1: "cmd+2" }))).toBe("duplicate");
  });

  it("lets none disable an action and free its key", () => {
    const resolved = validateSwitcherShortcuts({ previous: "none", next: "none" });
    expect(resolved.previous).toBe("none");
    expect(validateSwitcherShortcuts({ previous: "none", next: "left" }).next).toBe("left");
  });

  it("allows several disabled actions at once", () => {
    expect(() => validateSwitcherShortcuts({ select1: "none", select2: "none" })).not.toThrow();
  });
});

describe("shortcutFromEvent", () => {
  it("maps arrows and modifiers", () => {
    expect(shortcutFromEvent(keyEvent({ key: "ArrowLeft", code: "ArrowLeft" }))).toBe("left");
    expect(
      shortcutFromEvent(keyEvent({ key: "ArrowRight", code: "ArrowRight", ctrlKey: true, shiftKey: true })),
    ).toBe("ctrl+shift+right");
  });

  it("uses the physical key so shifted digits stay digits", () => {
    expect(shortcutFromEvent(keyEvent({ key: "!", code: "Digit1", ctrlKey: true, shiftKey: true }))).toBe(
      "ctrl+shift+1",
    );
    expect(shortcutFromEvent(keyEvent({ key: "A", code: "KeyA", altKey: true }))).toBe("alt+a");
  });

  it("maps comma and numpad digits via key", () => {
    expect(shortcutFromEvent(keyEvent({ key: ",", code: "Comma", ctrlKey: true }))).toBe("ctrl+,");
    expect(shortcutFromEvent(keyEvent({ key: "3", code: "Numpad3", ctrlKey: true }))).toBe("ctrl+3");
  });

  it("returns null for the Windows key and unsupported keys", () => {
    expect(shortcutFromEvent(keyEvent({ key: "1", code: "Digit1", metaKey: true }))).toBeNull();
    expect(shortcutFromEvent(keyEvent({ key: "Control", code: "ControlLeft", ctrlKey: true }))).toBeNull();
    expect(shortcutFromEvent(keyEvent({ key: "ArrowUp", code: "ArrowUp" }))).toBeNull();
    expect(shortcutFromEvent(keyEvent({ key: "F4", code: "F4", altKey: true }))).toBeNull();
  });
});

describe("matchSwitcherAction", () => {
  it("matches defaults", () => {
    expect(matchSwitcherAction(keyEvent({ key: "ArrowLeft", code: "ArrowLeft" }))).toBe("previous");
    expect(matchSwitcherAction(keyEvent({ key: "ArrowRight", code: "ArrowRight" }))).toBe("next");
    expect(matchSwitcherAction(keyEvent({ key: "3", code: "Digit3", ctrlKey: true }))).toBe("select3");
  });

  it("does not match extra or missing modifiers", () => {
    expect(matchSwitcherAction(keyEvent({ key: "ArrowLeft", code: "ArrowLeft", shiftKey: true }))).toBeNull();
    expect(matchSwitcherAction(keyEvent({ key: "3", code: "Digit3" }))).toBeNull();
    expect(matchSwitcherAction(keyEvent({ key: "3", code: "Digit3", ctrlKey: true, altKey: true }))).toBeNull();
  });

  it("honors a custom mapping and disabled actions", () => {
    const mapping = validateSwitcherShortcuts({ previous: "none", next: "shift+right" });
    expect(matchSwitcherAction(keyEvent({ key: "ArrowLeft", code: "ArrowLeft" }), mapping)).toBeNull();
    expect(
      matchSwitcherAction(keyEvent({ key: "ArrowRight", code: "ArrowRight", shiftKey: true }), mapping),
    ).toBe("next");
  });
});

describe("resolveSwitcherTarget", () => {
  const ids = ["codex", "claude", "gemini"];

  it("moves Overview -> providers -> Overview with wraparound", () => {
    expect(resolveSwitcherTarget("next", ids, null)).toEqual({ providerId: "codex" });
    expect(resolveSwitcherTarget("next", ids, "gemini")).toEqual({ providerId: null });
    expect(resolveSwitcherTarget("previous", ids, null)).toEqual({ providerId: "gemini" });
    expect(resolveSwitcherTarget("previous", ids, "codex")).toEqual({ providerId: null });
  });

  it("treats an unknown selection as Overview", () => {
    expect(resolveSwitcherTarget("next", ids, "gone")).toEqual({ providerId: "codex" });
  });

  it("does nothing when there are no providers", () => {
    expect(resolveSwitcherTarget("next", [], null)).toBeNull();
    expect(resolveSwitcherTarget("previous", [], null)).toBeNull();
  });

  it("selectN counts Overview as 1 and ignores positions past the end", () => {
    expect(resolveSwitcherTarget("select1", ids, "codex")).toEqual({ providerId: null });
    expect(resolveSwitcherTarget("select2", ids, null)).toEqual({ providerId: "codex" });
    expect(resolveSwitcherTarget("select4", ids, null)).toEqual({ providerId: "gemini" });
    expect(resolveSwitcherTarget("select5", ids, null)).toBeNull();
  });
});

describe("resolveSwitcherShortcuts", () => {
  it("returns the defaults for a missing or invalid map", () => {
    expect(resolveSwitcherShortcuts()).toEqual(DEFAULT_SWITCHER_SHORTCUTS);
    expect(resolveSwitcherShortcuts({})).toEqual(DEFAULT_SWITCHER_SHORTCUTS);
    expect(resolveSwitcherShortcuts({ next: "left" })).toEqual(DEFAULT_SWITCHER_SHORTCUTS);
    expect(resolveSwitcherShortcuts({ bogus: "ctrl+1" })).toEqual(DEFAULT_SWITCHER_SHORTCUTS);
  });

  it("accepts the fully resolved map the backend sends", () => {
    const stored = { ...DEFAULT_SWITCHER_SHORTCUTS, next: "none", select2: "ctrl+alt+2" };
    expect(resolveSwitcherShortcuts(stored)).toEqual(stored);
  });
});

describe("switcherShortcutOverrides", () => {
  it("keeps only entries that differ from the defaults", () => {
    expect(switcherShortcutOverrides(DEFAULT_SWITCHER_SHORTCUTS)).toEqual({});
    expect(
      switcherShortcutOverrides(validateSwitcherShortcuts({ next: "none", select2: "alt+2" })),
    ).toEqual({ next: "none", select2: "alt+2" });
  });
});
