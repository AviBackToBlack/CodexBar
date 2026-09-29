//! Provider-switcher keyboard shortcuts (upstream CodexBar 0.67.0,
//! `ProviderSwitcherShortcuts`).
//!
//! Shortcuts are menu-local strings such as `left`, `ctrl+3` or
//! `ctrl+shift+right`; `none` disables an action. The grammar is upstream's
//! with one Windows difference: `cmd` is accepted as an alias for `ctrl` and
//! normalized to `ctrl`, so documents exported from macOS stay portable.
//! Windows reserves `ctrl+r`, `ctrl+q`, `ctrl+,` and `ctrl+w` (tray and
//! pop-out commands).
//!
//! The frontend mirrors this module in `src/lib/switcherShortcuts.ts`; the
//! two must stay in step.

use std::collections::BTreeMap;

use thiserror::Error;

pub const NONE: &str = "none";

/// Switcher actions in display order with their default shortcuts.
pub const DEFAULTS: [(&str, &str); 11] = [
    ("previous", "left"),
    ("next", "right"),
    ("select1", "ctrl+1"),
    ("select2", "ctrl+2"),
    ("select3", "ctrl+3"),
    ("select4", "ctrl+4"),
    ("select5", "ctrl+5"),
    ("select6", "ctrl+6"),
    ("select7", "ctrl+7"),
    ("select8", "ctrl+8"),
    ("select9", "ctrl+9"),
];

const MODIFIER_ORDER: [&str; 3] = ["ctrl", "alt", "shift"];
const NAMED_KEYS: [&str; 3] = ["left", "right", ","];
const RESERVED: [&str; 4] = ["ctrl+r", "ctrl+q", "ctrl+,", "ctrl+w"];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SwitcherShortcutError {
    #[error("Unknown switcher shortcut action: {0}")]
    UnknownAction(String),
    #[error("Each switcher shortcut can be assigned to only one action")]
    Duplicate,
    #[error("{0} is reserved and cannot be used as a switcher shortcut")]
    Reserved(String),
    #[error("{0} is not a valid switcher shortcut")]
    Invalid(String),
}

/// Canonicalize one shortcut string (lowercase, modifiers ordered
/// `ctrl+alt+shift`, `cmd` folded into `ctrl`).
pub fn normalize(shortcut: &str) -> Result<String, SwitcherShortcutError> {
    let lowered = shortcut.to_lowercase();
    let parts: Vec<&str> = lowered.split('+').map(str::trim).collect();
    if parts == [NONE] {
        return Ok(NONE.to_string());
    }
    let invalid = || SwitcherShortcutError::Invalid(shortcut.to_string());
    let (key, modifier_parts) = parts.split_last().ok_or_else(invalid)?;
    let modifiers: Vec<&str> = modifier_parts
        .iter()
        .map(|part| if *part == "cmd" { "ctrl" } else { part })
        .collect();
    let mut distinct = modifiers.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let key_is_valid = NAMED_KEYS.contains(key)
        || (key.len() == 1
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    if distinct.len() != modifiers.len()
        || !modifiers.iter().all(|part| MODIFIER_ORDER.contains(part))
        || !key_is_valid
    {
        return Err(invalid());
    }
    let mut canonical: Vec<&str> = MODIFIER_ORDER
        .into_iter()
        .filter(|part| modifiers.contains(part))
        .collect();
    canonical.push(key);
    let result = canonical.join("+");
    let is_arrow = matches!(*key, "left" | "right");
    if RESERVED.contains(&result.as_str()) || (!is_arrow && modifiers.iter().all(|m| *m == "shift"))
    {
        return Err(SwitcherShortcutError::Reserved(shortcut.to_string()));
    }
    Ok(result)
}

/// Overlay `overrides` on the defaults and validate the result: unknown
/// actions and duplicate assignments are rejected; omitted actions keep their
/// default. Returns the fully resolved, normalized map.
pub fn resolve(
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, SwitcherShortcutError> {
    let mut resolved = default_map();
    for (action, shortcut) in overrides {
        let slot = resolved
            .get_mut(action)
            .ok_or_else(|| SwitcherShortcutError::UnknownAction(action.clone()))?;
        *slot = normalize(shortcut)?;
    }
    let mut assigned: Vec<&String> = resolved.values().filter(|value| *value != NONE).collect();
    assigned.sort_unstable();
    let count = assigned.len();
    assigned.dedup();
    if assigned.len() != count {
        return Err(SwitcherShortcutError::Duplicate);
    }
    Ok(resolved)
}

/// [`resolve`], falling back to the defaults when `overrides` is invalid
/// (stored maps are validated on write, so this only guards a corrupt file).
pub fn resolve_or_default(overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    resolve(overrides).unwrap_or_else(|_| default_map())
}

fn default_map() -> BTreeMap<String, String> {
    DEFAULTS
        .iter()
        .map(|(action, shortcut)| (action.to_string(), shortcut.to_string()))
        .collect()
}

/// Validate `overrides` and reduce them to the entries that differ from the
/// defaults, normalized. This is what gets persisted.
pub fn normalize_overrides(
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, SwitcherShortcutError> {
    let resolved = resolve(overrides)?;
    Ok(DEFAULTS
        .iter()
        .filter_map(|(action, default)| {
            let value = &resolved[*action];
            (value != default).then(|| (action.to_string(), value.clone()))
        })
        .collect())
}

#[cfg(test)]
mod tests;
