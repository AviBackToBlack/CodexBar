use super::*;
use crate::cost_reporting_period::cost_bucket_zone;

fn codex_cache_with_one_day(bucket_time_zone: Option<String>) -> CostUsageCache {
    CostUsageCache {
        codex_cache_schema_version: CODEX_CACHE_SCHEMA_VERSION,
        bucket_time_zone,
        days: HashMap::from([(
            "2026-09-09".to_string(),
            HashMap::from([("gpt-5.6-luna".to_string(), vec![1, 2, 3])]),
        )]),
        ..CostUsageCache::default()
    }
}

/// A valid IANA zone that differs from the one days are bucketed in now.
fn another_bucket_zone() -> String {
    let current = cost_bucket_zone().identifier();
    ["Pacific/Kiritimati", "Pacific/Pago_Pago"]
        .into_iter()
        .find(|zone| *zone != current)
        .expect("two distinct zones")
        .to_string()
}

#[test]
fn codex_cache_from_another_bucket_zone_is_rebuilt() {
    let stamp = CacheStamp::from_bytes(b"baseline");

    let moved = codex_cache_with_one_day(Some(another_bucket_zone()));
    let rebuilt = codex_cache_apply_load_policy(moved, stamp.clone());
    assert!(rebuilt.days.is_empty());
    assert!(rebuilt.files.is_empty());
    assert_eq!(
        rebuilt.codex_cache_schema_version,
        CODEX_CACHE_SCHEMA_VERSION
    );
    assert!(rebuilt.loaded_stamp.is_some());

    // Caches written before the zone stamp existed were bucketed in the
    // machine zone, which is what first launch pins, so they are kept.
    for kept_zone in [None, Some(cost_bucket_zone().identifier())] {
        let cache = codex_cache_with_one_day(kept_zone.clone());
        let kept = codex_cache_apply_load_policy(cache, stamp.clone());
        assert_eq!(
            kept.days["2026-09-09"]["gpt-5.6-luna"],
            vec![1, 2, 3],
            "{kept_zone:?}"
        );
        assert_eq!(kept.bucket_time_zone, kept_zone);
    }
}

#[test]
fn saved_codex_caches_record_their_bucket_zone() {
    let mut cache = CostUsageCache::default();
    codex_cache_stamp_schema_version(&mut cache);
    assert_eq!(
        cache.bucket_time_zone,
        Some(cost_bucket_zone().identifier())
    );

    assert!(codex_cache_zone_is_current(None));
    assert!(codex_cache_zone_is_current(Some(
        &cost_bucket_zone().identifier()
    )));
    assert!(!codex_cache_zone_is_current(Some(&another_bucket_zone())));
}

#[test]
fn codex_cache_status_ignores_history_from_another_bucket_zone() {
    let root = tempfile::tempdir().unwrap();
    let cache_root = root.path();
    let mut cache = codex_cache_with_one_day(None);
    cache.previous_report = Some(CachedCostReport {
        total_cost_usd: 1.0,
        input_tokens: 11,
        cached_tokens: 2,
        output_tokens: 3,
        reasoning_tokens: None,
        sessions_count: 1,
        updated_at: Some("2026-09-16T10:00:00Z".to_string()),
        partial: false,
    });
    JsonlScanner::save_cache(ProviderId::Codex, &mut cache, Some(cache_root));

    let cache_path = JsonlScanner::cache_path(ProviderId::Codex, Some(cache_root));
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    assert_eq!(
        saved["bucket_time_zone"],
        serde_json::json!(cost_bucket_zone().identifier())
    );
    let status = JsonlScanner::load_cache_status(ProviderId::Codex, Some(cache_root));
    assert!(status.has_days);
    assert!(status.previous_report.is_some());

    saved["bucket_time_zone"] = serde_json::json!(another_bucket_zone());
    std::fs::write(&cache_path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let status = JsonlScanner::load_cache_status(ProviderId::Codex, Some(cache_root));
    assert!(!status.has_days);
    assert!(status.previous_report.is_none());
    let reloaded = JsonlScanner::load_cache(ProviderId::Codex, Some(cache_root));
    assert!(reloaded.days.is_empty());
    assert!(reloaded.previous_report.is_none());
}
