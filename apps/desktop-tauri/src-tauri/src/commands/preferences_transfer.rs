//! Export and import of the portable preferences document.
//!
//! The document format, allowlist and validation live in
//! `codexbar::settings::PreferencesDocument`; this module only adds the
//! desktop side effects that a settings save normally triggers.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use codexbar::core::ProviderId;
use codexbar::settings::{PreferencesDocument, Settings};
use tauri::{Emitter, Manager};

use super::{SettingsSnapshot, language_label};
use crate::events;
use crate::state::AppState;

fn checked_path(path: &str) -> Result<&Path, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Preferences path must not be empty".to_string());
    }
    Ok(Path::new(path))
}

fn enabled_provider_set_changed(before: &[ProviderId], after: &[ProviderId]) -> bool {
    before.iter().copied().collect::<HashSet<_>>() != after.iter().copied().collect()
}

/// Write the current portable preferences to `path`; returns how many were written.
#[tauri::command]
pub fn export_preferences(path: String) -> Result<usize, String> {
    let path = checked_path(&path)?;
    let document =
        PreferencesDocument::from_settings(&Settings::load()).map_err(|error| error.to_string())?;
    document
        .write_file(path)
        .map_err(|error| error.to_string())?;
    tracing::info!(count = document.len(), "exported portable preferences");
    Ok(document.len())
}

/// Apply the preferences file at `path` to this machine's settings.
///
/// Nothing is saved when the file is rejected. On success the same live
/// updates as `update_settings` follow: locale, float bar, tray and dependent
/// windows, plus a provider cache prune and refresh when the enabled set changed.
#[tauri::command]
pub async fn import_preferences(
    app: tauri::AppHandle,
    path: String,
) -> Result<SettingsSnapshot, String> {
    let path = checked_path(&path)?;
    let document = PreferencesDocument::read_file(path).map_err(|error| error.to_string())?;
    let mut settings = Settings::load();
    let previous_language = settings.ui_language;
    let previous_enabled = settings.get_enabled_provider_ids();
    let applied = document
        .apply_to(&mut settings)
        .map_err(|error| error.to_string())?;
    settings.save().map_err(|error| error.to_string())?;
    tracing::info!(applied, "imported portable preferences");

    if settings.ui_language != previous_language {
        let _ = app.emit(events::LOCALE_CHANGED, language_label(settings.ui_language));
    }
    let enabled_ids = settings.get_enabled_provider_ids();
    let providers_changed = enabled_provider_set_changed(&previous_enabled, &enabled_ids);
    if providers_changed {
        let state = app.state::<Mutex<AppState>>();
        let _ = super::invalidate_provider_refresh_and_prune_disabled(&state, &enabled_ids);
    }
    crate::floatbar::notify_settings_changed(&app);
    crate::floatbar::apply_state(&app, &settings);
    crate::tray_bridge::rebuild_tray_menu(&app);
    crate::tray_bridge::refresh_tray_presentation(&app);
    events::emit_settings_changed(&app);

    if providers_changed {
        let refresh_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = super::do_refresh_providers(&refresh_app).await;
        });
    }
    Ok(SettingsSnapshot::from(settings))
}

#[cfg(test)]
mod tests {
    use super::enabled_provider_set_changed;
    use codexbar::core::ProviderId;

    #[test]
    fn refresh_decision_tracks_provider_membership() {
        let before = [ProviderId::Claude, ProviderId::Codex];
        assert!(!enabled_provider_set_changed(
            &before,
            &[ProviderId::Codex, ProviderId::Claude]
        ));
        assert!(enabled_provider_set_changed(&before, &[ProviderId::Claude]));
    }
}
