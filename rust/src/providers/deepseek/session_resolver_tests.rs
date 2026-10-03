use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::core::FailureOwnership;
use crate::providers::deepseek::BalanceInfo;

fn candidate(id: &str, token: &str) -> TokenInfo {
    TokenInfo {
        id: id.to_string(),
        token: token.to_string(),
        label: format!("Google Chrome {id}"),
    }
}

fn select(id: &str) -> ProfileSelection {
    ProfileSelection::from_value(Some(id))
}

fn balance(total: &str) -> BalanceResponse {
    BalanceResponse {
        is_available: true,
        balance_infos: vec![BalanceInfo {
            currency: "USD".into(),
            total_balance: total.into(),
            granted_balance: "0".into(),
            topped_up_balance: total.into(),
        }],
    }
}

fn fresh_cache() -> ValidationCache {
    ValidationCache::new(VALIDITY_TTL)
}

fn owner_of(id: &str, token: &str) -> LastGoodOwner {
    balance_owner(id, token).expect("owner")
}

/// Token-keyed outcomes: `ok-*` succeed, `rejected-*` are refused by the
/// platform, `down-*` hit a transport failure, `broken-*` a non-transport one.
async fn answer(token: String) -> Result<BalanceResponse, ProviderError> {
    if token.starts_with("rejected") {
        Err(ProviderError::AuthRequired)
    } else if token.starts_with("down") {
        Err(ProviderError::Timeout)
    } else if token.starts_with("broken") {
        Err(ProviderError::Parse("bad body".into()))
    } else {
        Ok(balance("5"))
    }
}

fn failed(resolution: Resolution) -> ProviderError {
    match resolution {
        Resolution::Failed(error) => error,
        other => panic!("expected failure, got {other:?}"),
    }
}

#[tokio::test]
async fn no_candidates_requires_a_session() {
    let resolution = resolve(&[], &ProfileSelection::default(), &fresh_cache(), answer).await;
    assert!(matches!(resolution, Resolution::SessionRequired));
}

#[tokio::test]
async fn single_valid_session_supplies_balance_and_owner() {
    let candidates = [candidate("chrome:Default", "ok-token")];
    let resolution = resolve(
        &candidates,
        &ProfileSelection::default(),
        &fresh_cache(),
        answer,
    )
    .await;
    let Resolution::Balance { balance, owner } = resolution else {
        panic!("expected balance");
    };
    assert!(balance.is_available);
    assert_eq!(owner, Some(owner_of("chrome:Default", "ok-token")));
}

#[tokio::test]
async fn owner_follows_profile_and_token() {
    assert_ne!(
        owner_of("chrome:Default", "ok-one"),
        owner_of("chrome:Default", "ok-two")
    );
    assert_ne!(
        owner_of("chrome:Default", "ok-one"),
        owner_of("chrome:Profile 1", "ok-one")
    );
    assert_eq!(
        owner_of("chrome:Default", "ok-one"),
        owner_of(" chrome:Default", "ok-one ")
    );
}

#[tokio::test]
async fn rejected_session_requires_a_new_sign_in() {
    let candidates = [candidate("chrome:Default", "rejected-token")];
    let resolution = resolve(
        &candidates,
        &ProfileSelection::default(),
        &fresh_cache(),
        answer,
    )
    .await;
    assert!(matches!(resolution, Resolution::SessionRequired));
}

#[tokio::test]
async fn transport_failure_keeps_the_session_that_failed() {
    let candidates = [candidate("chrome:Default", "down-token")];
    let error = failed(
        resolve(
            &candidates,
            &ProfileSelection::default(),
            &fresh_cache(),
            answer,
        )
        .await,
    );
    assert!(error.is_transport_failure());
    assert_eq!(
        error.failure_ownership(),
        FailureOwnership::Owned(owner_of("chrome:Default", "down-token"))
    );
}

#[tokio::test]
async fn non_transport_failure_is_not_owned() {
    let candidates = [candidate("chrome:Default", "broken-token")];
    let error = failed(
        resolve(
            &candidates,
            &ProfileSelection::default(),
            &fresh_cache(),
            answer,
        )
        .await,
    );
    assert!(matches!(error, ProviderError::Parse(_)));
    assert_eq!(error.failure_ownership(), FailureOwnership::Unchecked);
}

#[tokio::test]
async fn outage_after_rejection_cannot_revive_the_old_balance() {
    let cache = fresh_cache();
    let candidates = [candidate("chrome:Default", "token-flaky")];
    let calls = Arc::new(AtomicUsize::new(0));
    let flaky = |token: String| {
        let calls = Arc::clone(&calls);
        async move {
            let _ = token;
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Err(ProviderError::AuthRequired),
                _ => Err(ProviderError::Timeout),
            }
        }
    };
    let selection = select("chrome:Default");

    assert!(matches!(
        resolve(&candidates, &selection, &cache, &flaky).await,
        Resolution::SessionRequired
    ));
    let error = failed(resolve(&candidates, &selection, &cache, &flaky).await);
    assert!(error.is_transport_failure());
    assert_eq!(error.failure_ownership(), FailureOwnership::Unowned);
}

#[tokio::test]
async fn several_sessions_without_a_selection_ask_for_one() {
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "ok-b"),
    ];
    let resolution = resolve(
        &candidates,
        &ProfileSelection::default(),
        &fresh_cache(),
        answer,
    )
    .await;
    let Resolution::SelectionRequired(ids) = resolution else {
        panic!("expected selection");
    };
    assert_eq!(ids, vec!["chrome:Default", "chrome:Profile 1"]);
}

#[tokio::test]
async fn explicit_selection_supplies_that_profiles_balance() {
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "ok-b"),
    ];
    let resolution = resolve(
        &candidates,
        &select("chrome:Profile 1"),
        &fresh_cache(),
        answer,
    )
    .await;
    let Resolution::Balance { owner, .. } = resolution else {
        panic!("expected balance");
    };
    assert_eq!(owner, Some(owner_of("chrome:Profile 1", "ok-b")));
}

#[tokio::test]
async fn selected_profile_keeps_its_own_outage_beside_a_working_profile() {
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "down-b"),
    ];
    let error = failed(
        resolve(
            &candidates,
            &select("chrome:Profile 1"),
            &fresh_cache(),
            answer,
        )
        .await,
    );
    assert_eq!(
        error.failure_ownership(),
        FailureOwnership::Owned(owner_of("chrome:Profile 1", "down-b"))
    );
}

#[tokio::test]
async fn ambiguous_outage_does_not_borrow_another_profiles_failure() {
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "down-b"),
    ];
    let resolution = resolve(
        &candidates,
        &ProfileSelection::default(),
        &fresh_cache(),
        answer,
    )
    .await;
    // Only Default is known valid; with no explicit selection the single valid
    // profile is selected and supplies its own balance.
    let Resolution::Balance { owner, .. } = resolution else {
        panic!("expected balance");
    };
    assert_eq!(owner, Some(owner_of("chrome:Default", "ok-a")));
}

#[tokio::test]
async fn rejected_selection_beside_a_valid_profile_asks_for_a_choice() {
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "rejected-b"),
    ];
    let resolution = resolve(
        &candidates,
        &select("chrome:Profile 1"),
        &fresh_cache(),
        answer,
    )
    .await;
    let Resolution::SelectionRequired(ids) = resolution else {
        panic!("expected selection, got {resolution:?}");
    };
    assert_eq!(ids, vec!["chrome:Default"]);
}

#[tokio::test]
async fn unknown_selection_asks_for_a_choice() {
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "ok-b"),
    ];
    let resolution = resolve(
        &candidates,
        &select("chrome:Missing"),
        &fresh_cache(),
        answer,
    )
    .await;
    assert!(matches!(resolution, Resolution::SelectionRequired(_)));
}

#[tokio::test]
async fn fresh_cache_skips_revalidating_unselected_profiles() {
    let cache = fresh_cache();
    let calls = Arc::new(AtomicUsize::new(0));
    let counting = |token: String| {
        let calls = Arc::clone(&calls);
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            answer(token).await
        }
    };
    let candidates = [
        candidate("chrome:Default", "ok-a"),
        candidate("chrome:Profile 1", "ok-b"),
    ];
    let selection = select("chrome:Default");

    resolve(&candidates, &selection, &cache, &counting).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    resolve(&candidates, &selection, &cache, &counting).await;
    // Only the selected profile is asked again: its balance is always live.
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn changed_token_in_the_same_profile_is_validated_again() {
    let cache = fresh_cache();
    let selection = select("chrome:Default");
    let first = [candidate("chrome:Default", "rejected-old")];
    assert!(matches!(
        resolve(&first, &selection, &cache, answer).await,
        Resolution::SessionRequired
    ));

    let second = [candidate("chrome:Default", "ok-new")];
    let resolution = resolve(&second, &selection, &cache, answer).await;
    let Resolution::Balance { owner, .. } = resolution else {
        panic!("expected balance");
    };
    assert_eq!(owner, Some(owner_of("chrome:Default", "ok-new")));
}

#[tokio::test]
async fn cached_status_survives_a_later_validation_outage() {
    let cache = ValidationCache::new(Duration::ZERO);
    let candidates = [candidate("chrome:Default", "token-x")];
    let calls = Arc::new(AtomicUsize::new(0));
    let script = |_token: String| {
        let calls = Arc::clone(&calls);
        async move {
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Ok(balance("5")),
                _ => Err(ProviderError::Timeout),
            }
        }
    };
    let selection = ProfileSelection::default();

    assert!(matches!(
        resolve(&candidates, &selection, &cache, &script).await,
        Resolution::Balance { .. }
    ));
    let error = failed(resolve(&candidates, &selection, &cache, &script).await);
    assert_eq!(
        error.failure_ownership(),
        FailureOwnership::Owned(owner_of("chrome:Default", "token-x"))
    );
}

#[test]
fn validation_cache_stores_no_plain_token() {
    let cache = fresh_cache();
    let candidate = candidate("chrome:Default", "super-secret-token-value");
    cache.record(&candidate, true, Instant::now());
    let entries = cache.entries.lock().unwrap();
    let rendered = format!("{:?}", entries.get("chrome:Default").unwrap().proof);
    assert!(!rendered.contains("super-secret"));
}
