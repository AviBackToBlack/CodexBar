use super::{
    ModelIdMatch, ModelsDevCache, ModelsDevPricingTarget, counting_fetch, pricing_snapshot_at,
    refresh_exact_pricing_targets_with, refresh_unknown_models_at, save_catalog_json_for_tests,
};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(2_000_000_000)
}

/// A plausible catalog (OpenAI and Anthropic are priceable) plus one
/// OpenAI model under `fixture_key` priced at `fixture_input` per million.
fn catalog(fixture_key: &str, fixture_input: f64) -> String {
    format!(
        r#"{{
            "openai": {{"models": {{
                "gpt-base": {{"id": "gpt-base", "cost": {{"input": 1, "output": 2}}}},
                "{fixture_key}": {{"id": "{fixture_key}", "cost": {{"input": {fixture_input}, "output": 8}}}}
            }}}},
            "anthropic": {{"models": {{
                "claude-base": {{"id": "claude-base", "cost": {{"input": 3, "output": 15}}}}
            }}}}
        }}"#
    )
}

fn fixture_input_rate(root: &Path, model_id: &str) -> Option<f64> {
    pricing_snapshot_at(now(), Some(root))
        .lookup_exact("openai", model_id)
        .map(|pricing| pricing.input_cost_per_token)
}

fn targets(model_id: &str) -> Vec<ModelsDevPricingTarget> {
    vec![ModelsDevPricingTarget {
        provider_id: "openai".to_string(),
        model_id: model_id.to_string(),
    }]
}

#[tokio::test]
async fn exact_miss_outside_the_attempt_window_refreshes_once() {
    let root = tempdir().unwrap();
    let before = now() - Duration::from_secs(901);
    assert!(save_catalog_json_for_tests(
        &catalog("gpt-other", 1.0),
        before,
        root.path()
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let response = Some(catalog("gpt-fixture", 2.0));

    let fetch = counting_fetch(response.clone(), Arc::clone(&calls));
    refresh_exact_pricing_targets_with(&targets("gpt-fixture"), now(), Some(root.path()), fetch)
        .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture_input_rate(root.path(), "gpt-fixture"), Some(2e-6));
    // The replaced catalog keeps the earlier priceable model.
    assert_eq!(fixture_input_rate(root.path(), "gpt-other"), Some(1e-6));

    let fetch = counting_fetch(response, Arc::clone(&calls));
    refresh_exact_pricing_targets_with(&targets("gpt-fixture"), now(), Some(root.path()), fetch)
        .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_catalog_fetched_within_fifteen_minutes_is_not_refetched() {
    let root = tempdir().unwrap();
    let recent = now() - Duration::from_secs(60);
    assert!(save_catalog_json_for_tests(
        &catalog("gpt-other", 1.0),
        recent,
        root.path()
    ));
    let calls = Arc::new(AtomicUsize::new(0));

    let fetch = counting_fetch(Some(catalog("gpt-fixture", 2.0)), Arc::clone(&calls));
    refresh_exact_pricing_targets_with(&targets("gpt-fixture"), now(), Some(root.path()), fetch)
        .await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture_input_rate(root.path(), "gpt-fixture"), None);
}

#[tokio::test]
async fn a_dated_alias_satisfies_only_a_fuzzy_refresh() {
    let root = tempdir().unwrap();
    let before = now() - Duration::from_secs(901);
    let cached = catalog("gpt-fixture-20260101", 1.0);
    let calls = Arc::new(AtomicUsize::new(0));
    let models = HashSet::from(["gpt-fixture".to_string()]);

    assert!(save_catalog_json_for_tests(&cached, before, root.path()));
    let fetch = counting_fetch(Some(catalog("gpt-fixture", 2.0)), Arc::clone(&calls));
    assert!(
        refresh_unknown_models_at(
            "openai",
            &models,
            ModelIdMatch::Fuzzy,
            now(),
            Some(root.path()),
            fetch,
        )
        .await
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let fetch = counting_fetch(Some(catalog("gpt-fixture", 2.0)), Arc::clone(&calls));
    assert!(
        refresh_unknown_models_at(
            "openai",
            &models,
            ModelIdMatch::Exact,
            now(),
            Some(root.path()),
            fetch,
        )
        .await
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture_input_rate(root.path(), "gpt-fixture"), Some(2e-6));
}

#[tokio::test]
async fn a_stale_catalog_is_replaced_before_exact_lookups() {
    let root = tempdir().unwrap();
    let stale = now() - Duration::from_secs(90_000);
    assert!(save_catalog_json_for_tests(
        &catalog("gpt-fixture", 1.0),
        stale,
        root.path()
    ));
    assert_eq!(fixture_input_rate(root.path(), "gpt-fixture"), None);
    let calls = Arc::new(AtomicUsize::new(0));

    let fetch = counting_fetch(Some(catalog("gpt-fixture", 2.0)), Arc::clone(&calls));
    refresh_exact_pricing_targets_with(&targets("gpt-fixture"), now(), Some(root.path()), fetch)
        .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture_input_rate(root.path(), "gpt-fixture"), Some(2e-6));
}

#[tokio::test]
async fn failed_or_implausible_refreshes_keep_the_previous_cache() {
    for response in [None, Some(r#"{"openai": {"models": {}}}"#.to_string())] {
        let root = tempdir().unwrap();
        let stale = now() - Duration::from_secs(90_000);
        assert!(save_catalog_json_for_tests(
            &catalog("gpt-other", 1.0),
            stale,
            root.path()
        ));
        let cache_path = ModelsDevCache::cache_path(Some(root.path()));
        let before = std::fs::read(&cache_path).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));

        let fetch = counting_fetch(response.clone(), Arc::clone(&calls));
        refresh_exact_pricing_targets_with(
            &targets("gpt-fixture"),
            now(),
            Some(root.path()),
            fetch,
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "{response:?}");
        assert_eq!(std::fs::read(&cache_path).unwrap(), before, "{response:?}");
    }
}

#[tokio::test]
async fn no_targets_never_fetch() {
    let root = tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let fetch = counting_fetch(Some(catalog("gpt-fixture", 2.0)), Arc::clone(&calls));

    refresh_exact_pricing_targets_with(&[], now(), Some(root.path()), fetch).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!ModelsDevCache::cache_path(Some(root.path())).exists());
}
