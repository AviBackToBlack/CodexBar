//! Portable UI preferences document (upstream 0.67.0 "portable preferences").
//!
//! A versioned JSON file, `{"version":1,"preferences":{...}}`, that moves the
//! look-and-behavior preferences between machines. It is an explicit
//! allowlist, independent of the on-disk `settings.json` shape:
//!
//! - Unknown keys, unsupported versions, wrong types and out-of-range values
//!   are rejected before anything is applied.
//! - Keys missing from a document leave the receiving machine unchanged.
//! - `null` restores the documented default (`Settings::default()`).
//!
//! Secrets and machine-specific or consent settings are never portable. The
//! [`EXCLUDED_KEYS`] list names every other [`Settings`] field, and a test
//! fails when a new field is added without being classified, so a new field
//! is excluded until someone opts it in.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::{MetricPreference, Settings};
use crate::core::ProviderId;

/// Current (and only) document version.
pub const PREFERENCES_DOCUMENT_VERSION: u64 = 1;

/// Upper bound for a document read from disk. A real document is a few KiB.
pub const MAX_PREFERENCES_DOCUMENT_BYTES: u64 = 256 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PreferencesError {
    #[error("Preferences file is not a valid preferences document")]
    Malformed,
    #[error("Unsupported preferences version")]
    UnsupportedVersion,
    #[error("Invalid or non-portable preference: {0}")]
    InvalidPreference(String),
    #[error("Preferences file is larger than {} KiB", MAX_PREFERENCES_DOCUMENT_BYTES / 1024)]
    TooLarge,
    #[error("Could not read or write the preferences file: {0}")]
    Io(String),
    #[error("Could not apply preferences: {0}")]
    Apply(String),
}

type Validator = fn(&Value) -> bool;

/// Portable keys, named after the `Settings` fields, with their validators.
const ALLOWED_PREFERENCES: &[(&str, Validator)] = &[
    ("refresh_interval_secs", refresh_interval_secs),
    ("adaptive_refresh", typed::<bool>),
    ("refresh_all_providers_on_menu_open", typed::<bool>),
    (
        "low_power_mode_preference",
        typed::<super::LowPowerModePreference>,
    ),
    ("show_notifications", typed::<bool>),
    ("sound_enabled", typed::<bool>),
    ("high_usage_threshold", percent),
    ("critical_usage_threshold", percent),
    ("provider_usage_thresholds", provider_usage_thresholds),
    ("predictive_pace_warning_enabled", typed::<bool>),
    ("show_pace", typed::<bool>),
    ("show_as_used", typed::<bool>),
    ("reset_time_relative", typed::<bool>),
    ("show_reset_when_exhausted", typed::<bool>),
    ("hide_personal_info", typed::<bool>),
    ("menu_bar_shows_highest_usage", typed::<bool>),
    ("menu_bar_shows_percent", typed::<bool>),
    ("tray_icon_mode", typed::<super::TrayIconMode>),
    ("switcher_shows_icons", typed::<bool>),
    ("overview_layout", |v| one_of(v, &["compact", "detailed"])),
    ("menu_bar_display_mode", |v| {
        one_of(v, &["minimal", "compact", "detailed"])
    }),
    ("provider_order", provider_id_list),
    ("enabled_providers", provider_id_list),
    ("provider_metrics", provider_metrics),
    ("theme", typed::<super::ThemePreference>),
    ("ui_language", typed::<super::Language>),
    ("window_scale_percent", uint_in::<100, 250>),
    ("tray_scale_percent", uint_in::<100, 200>),
    ("float_bar_opacity", uint_in::<30, 100>),
    ("float_bar_scale", uint_in::<75, 200>),
    ("float_bar_orientation", |v| {
        one_of(v, &["horizontal", "vertical"])
    }),
    ("float_bar_style", |v| one_of(v, &["floating", "taskbar"])),
    ("float_bar_dark_text", typed::<bool>),
    ("float_bar_show_reset_inline", typed::<bool>),
    ("float_bar_show_cost", typed::<bool>),
];

/// Every other `Settings` field. Secrets (`provider_configs` holds API
/// tokens and manual cookie headers, `http_proxy_password`), machine paths and
/// hosts, and consent or side-effect toggles must stay out of a portable file.
pub const EXCLUDED_KEYS: &[&str] = &[
    // Secrets and credential-bearing config.
    "provider_configs",
    "http_proxy_enabled",
    "http_proxy_url",
    "http_proxy_username",
    "http_proxy_password",
    // Machine-specific paths and hosts.
    "notification_sound_paths",
    "codex_custom_sessions_dirs",
    "agent_session_ssh_hosts",
    // Consent, autostart and other side-effect toggles.
    "start_at_login",
    "start_minimized",
    "hooks_enabled",
    "global_shortcut",
    "agent_sessions_enabled",
    "disable_keychain_access",
    "claude_allow_reading_claude_code_credentials",
    "codex_external_oauth_sources_allowed",
    "open_codex_usage_logs_enabled",
    "hide_native_codex_cost_when_open_codex_present",
    "powertoys_status_pipe_enabled",
    "float_bar_enabled",
    "float_bar_click_through",
    "tray_panel_always_on_top",
    "promote_tray_icon",
    // Update settings.
    "update_channel",
    "auto_download_updates",
    "install_updates_on_quit",
    // Not part of the v1 allowlist.
    "notification_sound_theme",
    "enable_animations",
    "show_all_token_accounts_in_menu",
    "float_bar_provider_ids",
    "claude_daily_routines_usage_visible",
    "weekly_progress_work_days",
    "alibaba_token_plan_region",
    "cost_summary_display_style",
    // No reader anywhere in the app.
    "merge_tray_icons",
];

fn typed<T: DeserializeOwned>(value: &Value) -> bool {
    serde_json::from_value::<T>(value.clone()).is_ok()
}

fn one_of(value: &Value, choices: &[&str]) -> bool {
    value.as_str().is_some_and(|raw| choices.contains(&raw))
}

fn uint_in<const MIN: u64, const MAX: u64>(value: &Value) -> bool {
    value
        .as_u64()
        .is_some_and(|number| (MIN..=MAX).contains(&number))
}

/// `0` is manual refresh; otherwise the interval must be at least a minute
/// (the shortest option offered in Settings) and at most a day.
fn refresh_interval_secs(value: &Value) -> bool {
    value
        .as_u64()
        .is_some_and(|secs| secs == 0 || (60..=86_400).contains(&secs))
}

fn percent(value: &Value) -> bool {
    value.is_number()
        && value
            .as_f64()
            .is_some_and(|number| number.is_finite() && (0.0..=100.0).contains(&number))
}

fn known_provider_ids() -> HashSet<&'static str> {
    ProviderId::all().iter().map(|id| id.cli_name()).collect()
}

fn provider_id_list(value: &Value) -> bool {
    let (Some(items), known) = (value.as_array(), known_provider_ids()) else {
        return false;
    };
    let mut seen = HashSet::new();
    items.iter().all(|item| {
        item.as_str()
            .is_some_and(|id| known.contains(id) && seen.insert(id))
    })
}

fn provider_metrics(value: &Value) -> bool {
    let known = known_provider_ids();
    value.as_object().is_some_and(|map| {
        map.iter().all(|(provider, metric)| {
            known.contains(provider.as_str()) && typed::<MetricPreference>(metric)
        })
    })
}

/// Keys are `<provider>` or `<provider>:session|weekly`; values carry optional
/// `high` / `critical` percentages.
fn provider_usage_thresholds(value: &Value) -> bool {
    let known = known_provider_ids();
    let Some(map) = value.as_object() else {
        return false;
    };
    map.iter().all(|(key, entry)| {
        let (provider, window) = key
            .split_once(':')
            .map_or((key.as_str(), None), |(provider, window)| {
                (provider, Some(window))
            });
        known.contains(provider)
            && window.is_none_or(|window| matches!(window, "session" | "weekly"))
            && entry.as_object().is_some_and(|fields| {
                fields.iter().all(|(field, number)| {
                    matches!(field.as_str(), "high" | "critical")
                        && (number.is_null() || percent(number))
                })
            })
    })
}

fn invalid(key: &str) -> PreferencesError {
    PreferencesError::InvalidPreference(key.to_string())
}

/// A validated preferences document.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesDocument {
    preferences: BTreeMap<String, Value>,
}

impl PreferencesDocument {
    /// Parse and fully validate a document. Nothing is applied here.
    pub fn from_json(text: &str) -> Result<Self, PreferencesError> {
        if text.len() as u64 > MAX_PREFERENCES_DOCUMENT_BYTES {
            return Err(PreferencesError::TooLarge);
        }
        let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| PreferencesError::Malformed)?;
        let Value::Object(mut root) = root else {
            return Err(PreferencesError::Malformed);
        };
        if root.get("version").and_then(Value::as_u64) != Some(PREFERENCES_DOCUMENT_VERSION) {
            return Err(PreferencesError::UnsupportedVersion);
        }
        let Some(Value::Object(preferences)) = root.remove("preferences") else {
            return Err(PreferencesError::Malformed);
        };
        if root.len() != 1 {
            return Err(PreferencesError::Malformed);
        }
        Self::validated(preferences)
    }

    fn validated(preferences: Map<String, Value>) -> Result<Self, PreferencesError> {
        for (key, value) in &preferences {
            let Some((_, valid)) = ALLOWED_PREFERENCES.iter().find(|(name, _)| name == key) else {
                return Err(invalid(key));
            };
            if !value.is_null() && !valid(value) {
                return Err(invalid(key));
            }
        }
        Ok(Self {
            preferences: preferences.into_iter().collect(),
        })
    }

    /// Snapshot the portable preferences of `settings`.
    pub fn from_settings(settings: &Settings) -> Result<Self, PreferencesError> {
        let Value::Object(mut all) = serde_json::to_value(settings)
            .map_err(|error| PreferencesError::Apply(error.to_string()))?
        else {
            return Err(PreferencesError::Apply(
                "settings did not serialize to an object".to_string(),
            ));
        };
        let mut preferences = Map::new();
        for (key, _) in ALLOWED_PREFERENCES {
            if let Some(value) = all.remove(*key) {
                preferences.insert((*key).to_string(), value);
            }
        }
        if let Some(Value::Array(ids)) = preferences.get_mut("enabled_providers") {
            // `enabled_providers` is a set; keep exports deterministic.
            ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
        Self::validated(preferences)
    }

    /// Pretty-printed JSON with a trailing newline.
    pub fn to_json(&self) -> String {
        let document = serde_json::json!({
            "version": PREFERENCES_DOCUMENT_VERSION,
            "preferences": self.preferences,
        });
        let mut text = serde_json::to_string_pretty(&document).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Number of preferences the document sets.
    pub fn len(&self) -> usize {
        self.preferences.len()
    }

    pub fn is_empty(&self) -> bool {
        self.preferences.is_empty()
    }

    /// Apply to `settings`. Absent keys are untouched; `null` restores the
    /// default. Returns the number of preferences applied. `settings` is left
    /// unchanged on error.
    pub fn apply_to(&self, settings: &mut Settings) -> Result<usize, PreferencesError> {
        let apply_error = |error: serde_json::Error| PreferencesError::Apply(error.to_string());
        let Value::Object(mut current) = serde_json::to_value(&*settings).map_err(apply_error)?
        else {
            return Err(PreferencesError::Apply(
                "settings did not serialize to an object".to_string(),
            ));
        };
        let Value::Object(defaults) =
            serde_json::to_value(Settings::default()).map_err(apply_error)?
        else {
            return Err(PreferencesError::Apply(
                "default settings did not serialize to an object".to_string(),
            ));
        };
        for (key, value) in &self.preferences {
            let replacement = if value.is_null() {
                defaults.get(key).cloned().ok_or_else(|| invalid(key))?
            } else {
                value.clone()
            };
            current.insert(key.clone(), replacement);
        }
        // Re-read through the normal settings loader so clamping and
        // normalization match what `settings.json` gets on load.
        *settings = serde_json::from_value(Value::Object(current)).map_err(apply_error)?;
        Ok(self.preferences.len())
    }

    /// Read a document from `path`, bounded to [`MAX_PREFERENCES_DOCUMENT_BYTES`].
    pub fn read_file(path: &Path) -> Result<Self, PreferencesError> {
        let file =
            std::fs::File::open(path).map_err(|error| PreferencesError::Io(error.to_string()))?;
        let mut bytes = Vec::new();
        file.take(MAX_PREFERENCES_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PreferencesError::Io(error.to_string()))?;
        if bytes.len() as u64 > MAX_PREFERENCES_DOCUMENT_BYTES {
            return Err(PreferencesError::TooLarge);
        }
        let text = String::from_utf8(bytes).map_err(|_| PreferencesError::Malformed)?;
        Self::from_json(&text)
    }

    pub fn write_file(&self, path: &Path) -> Result<(), PreferencesError> {
        std::fs::write(path, self.to_json())
            .map_err(|error| PreferencesError::Io(error.to_string()))
    }
}

#[cfg(test)]
mod tests;
