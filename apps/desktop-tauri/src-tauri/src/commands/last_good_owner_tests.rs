//! Owner-checked last-good retention in the provider refresh shell.
//!
//! A failure tied to a live session may keep the cached snapshot only when the
//! same session produced that snapshot. Scenarios use DeepSeek's Chrome
//! session balance, the provider that declares owned transport failures.

use super::ProviderUsageSnapshot;
use super::providers::{preserve_last_good_transient_failure, record_last_good_owner};
use crate::state::AppState;
use codexbar::core::{
    LastGoodOwner, ProviderError, ProviderFetchResult, ProviderId, ProviderStateKind, RateWindow,
    UsageSnapshot, instantiate_provider,
};

const MEASURED_AT: &str = "2026-09-01T00:00:00Z";

fn owner(profile: &str, token: &str) -> LastGoodOwner {
    LastGoodOwner::derive("deepseek-platform-balance", profile, token).expect("owner")
}

fn cached_balance() -> ProviderUsageSnapshot {
    let metadata = instantiate_provider(ProviderId::DeepSeek)
        .metadata()
        .clone();
    let result = ProviderFetchResult::new(UsageSnapshot::new(RateWindow::new(0.0)), "web");
    let mut snapshot =
        ProviderUsageSnapshot::from_fetch_result(ProviderId::DeepSeek, &metadata, &result, None);
    snapshot.updated_at = MEASURED_AT.to_string();
    snapshot
}

fn failed_refresh() -> ProviderUsageSnapshot {
    let metadata = instantiate_provider(ProviderId::DeepSeek)
        .metadata()
        .clone();
    ProviderUsageSnapshot::from_error(
        ProviderId::DeepSeek,
        &metadata,
        "Timeout".to_string(),
        ProviderStateKind::Unknown,
    )
}

fn state_with_cached_balance(cached_owner: Option<LastGoodOwner>) -> AppState {
    let mut state = AppState::new();
    state.provider_cache.push(cached_balance());
    record_last_good_owner(&mut state, ProviderId::DeepSeek, cached_owner);
    state
}

fn refresh_failure(state: &mut AppState, error: &ProviderError) -> ProviderUsageSnapshot {
    preserve_last_good_transient_failure(state, ProviderId::DeepSeek, failed_refresh(), error)
}

#[test]
fn matching_session_keeps_the_balance_and_its_original_time() {
    let session = owner("chrome:Default", "token-a");
    let mut state = state_with_cached_balance(Some(session.clone()));

    let kept = refresh_failure(
        &mut state,
        &ProviderError::Timeout.with_failure_owner(Some(session)),
    );

    assert_eq!(kept.error, None);
    assert_eq!(kept.updated_at, MEASURED_AT);
}

#[test]
fn different_profile_or_token_shows_the_error() {
    for failed_session in [
        owner("chrome:Profile 1", "token-a"),
        owner("chrome:Default", "token-b"),
    ] {
        let mut state = state_with_cached_balance(Some(owner("chrome:Default", "token-a")));
        let shown = refresh_failure(
            &mut state,
            &ProviderError::Timeout.with_failure_owner(Some(failed_session)),
        );
        assert_eq!(shown.error.as_deref(), Some("Timeout"));
    }
}

#[test]
fn failure_without_a_session_owner_fails_closed() {
    let mut state = state_with_cached_balance(Some(owner("chrome:Default", "token-a")));

    let shown = refresh_failure(&mut state, &ProviderError::Timeout.with_failure_owner(None));

    assert_eq!(shown.error.as_deref(), Some("Timeout"));
}

#[test]
fn balance_without_an_owner_is_never_retained() {
    // An API-key balance, a decoded cache, or a seeded snapshot has no owner.
    let session = owner("chrome:Default", "token-a");
    let mut state = state_with_cached_balance(None);

    let shown = refresh_failure(
        &mut state,
        &ProviderError::Timeout.with_failure_owner(Some(session)),
    );

    assert_eq!(shown.error.as_deref(), Some("Timeout"));
}

#[test]
fn unattributed_transport_failure_does_not_retain_deepseek_balances() {
    let mut state = state_with_cached_balance(Some(owner("chrome:Default", "token-a")));

    let shown = refresh_failure(&mut state, &ProviderError::Timeout);

    assert_eq!(shown.error.as_deref(), Some("Timeout"));
}

#[test]
fn fresh_snapshot_replaces_or_clears_the_owner() {
    let first = owner("chrome:Default", "token-a");
    let second = owner("chrome:Default", "token-b");
    let mut state = state_with_cached_balance(Some(first.clone()));

    record_last_good_owner(&mut state, ProviderId::DeepSeek, Some(second.clone()));
    assert_eq!(
        state.last_good_owners.get(&ProviderId::DeepSeek),
        Some(&second)
    );
    let shown = refresh_failure(
        &mut state,
        &ProviderError::Timeout.with_failure_owner(Some(first)),
    );
    assert_eq!(shown.error.as_deref(), Some("Timeout"));

    record_last_good_owner(&mut state, ProviderId::DeepSeek, None);
    assert!(state.last_good_owners.is_empty());
}

#[test]
fn other_providers_keep_their_existing_failure_policy() {
    let metadata = instantiate_provider(ProviderId::Codex).metadata().clone();
    let result = ProviderFetchResult::new(UsageSnapshot::new(RateWindow::new(42.0)), "OAuth");
    let good =
        ProviderUsageSnapshot::from_fetch_result(ProviderId::Codex, &metadata, &result, None);
    let failed = ProviderUsageSnapshot::from_error(
        ProviderId::Codex,
        &metadata,
        "Timeout".to_string(),
        ProviderStateKind::Unknown,
    );
    let mut state = AppState::new();
    state.provider_cache.push(good);

    let kept = preserve_last_good_transient_failure(
        &mut state,
        ProviderId::Codex,
        failed,
        &ProviderError::Timeout,
    );

    assert_eq!(kept.error, None);
}
