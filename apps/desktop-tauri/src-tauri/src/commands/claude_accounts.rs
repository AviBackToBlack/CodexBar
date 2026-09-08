use super::invalidate_account_usage;
use crate::state::AppState;
use codexbar::core::ProviderId;
use codexbar::providers::claude::accounts::{self, AccountManager, ClaudeAccount};
use std::sync::Mutex;
use tauri::Emitter;
use tauri::Manager;

static MUTATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tauri::command]
pub fn claude_accounts_list() -> Result<Vec<ClaudeAccount>, String> {
    AccountManager::new()
        .and_then(|m| m.list())
        .map_err(|e| e.to_string())
}

fn changed(app: &tauri::AppHandle) {
    let _emit = app.emit("claude-accounts-updated", ());
}

#[tauri::command]
pub async fn claude_account_add(app: tauri::AppHandle) -> Result<(), String> {
    let _mutation = MUTATION
        .try_lock()
        .map_err(|_| "A Claude account operation is already in progress.")?;
    accounts::begin_login();
    let login = tauri::async_runtime::spawn_blocking(accounts::login)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let _credentials = accounts::CREDENTIAL_OPERATION.lock().await;
    AccountManager::new()
        .and_then(|m| m.import(login))
        .map_err(|e| e.to_string())?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn claude_account_cancel_login() {
    accounts::cancel_login();
}

#[tauri::command]
pub async fn claude_account_save_current(app: tauri::AppHandle) -> Result<(), String> {
    let _mutation = MUTATION
        .try_lock()
        .map_err(|_| "A Claude account operation is already in progress.")?;
    let _credentials = accounts::CREDENTIAL_OPERATION.lock().await;
    AccountManager::new()
        .and_then(|m| m.save_current())
        .map_err(|e| e.to_string())?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn claude_account_remove(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let _mutation = MUTATION
        .try_lock()
        .map_err(|_| "A Claude account operation is already in progress.")?;
    let _credentials = accounts::CREDENTIAL_OPERATION.lock().await;
    AccountManager::new()
        .and_then(|m| m.remove(&id))
        .map_err(|e| e.to_string())?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn claude_account_switch(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let _mutation = MUTATION
        .try_lock()
        .map_err(|_| "A Claude account operation is already in progress.")?;
    let _credentials = accounts::CREDENTIAL_OPERATION.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        accounts::require_cli_closed()?;
        AccountManager::new()?.switch(&id)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let pending = {
        let state = app.state::<Mutex<AppState>>();
        let mut state = state.lock().map_err(|e| e.to_string())?;
        invalidate_account_usage(&mut state, ProviderId::Claude)
    };
    crate::events::emit_provider_updated(&app, &pending);
    drop(_credentials);
    changed(&app);
    tauri::async_runtime::spawn(async move {
        let _refresh = super::refresh_providers(app).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_invalidates_old_identity_usage_and_inflight_results() {
        let mut state = AppState::new();
        let mut old = invalidate_account_usage(&mut state, ProviderId::Claude);
        old.account_email = Some("old@example.com".into());
        old.plan_name = Some("old-plan".into());
        old.error = None;
        old.primary.used_percent = 80.0;
        state.provider_cache = vec![old];
        state.is_refreshing = true;
        state
            .transient_provider_failure_counts
            .insert(ProviderId::Claude, 1);
        let generation = state.provider_refresh_generation;
        let pending = invalidate_account_usage(&mut state, ProviderId::Claude);
        assert!(pending.account_email.is_none());
        assert!(pending.plan_name.is_none());
        assert!(pending.error.is_some());
        assert_eq!(pending.primary.used_percent, 0.0);
        assert_eq!(state.provider_cache.len(), 1);
        assert!(state.provider_cache[0].error.is_some());
        assert_ne!(state.provider_refresh_generation, generation);
        assert!(!state.is_refreshing);
        assert!(
            !state
                .transient_provider_failure_counts
                .contains_key(&ProviderId::Claude)
        );
    }
}
