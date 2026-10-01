use super::{
    ModelsDevCache, ModelsDevCacheArtifact, ModelsDevCatalog, ModelsDevRefreshCoordinator,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn decodes_top_level_provider_map_and_converts_million_token_rates() {
    let catalog = ModelsDevCatalog::decode(
        r#"{
            "openai": {
                "id": "openai",
                "models": {
                    "openai/gpt-fresh": {
                        "id": "openai/gpt-fresh",
                        "cost": {
                            "input": 2.5,
                            "output": 10,
                            "cache_read": 0.25,
                            "cache_write": 3.75,
                            "context_over_200k": {
                                "input": 5,
                                "output": 15,
                                "cache_read": 0.5,
                                "cache_write": 7.5
                            }
                        }
                    }
                }
            },
            "anthropic": {
                "models": {
                    "claude-fresh": {
                        "id": "claude-fresh",
                        "cost": { "input": 3, "output": 15 }
                    }
                }
            }
        }"#,
    )
    .expect("top-level catalog");

    let pricing = catalog.lookup("openai", "gpt-fresh").expect("pricing");
    assert_eq!(pricing.input_cost_per_token, 2.5e-6);
    assert_eq!(pricing.output_cost_per_token, 10e-6);
    assert_eq!(pricing.cache_read_input_cost_per_token, Some(0.25e-6));
    assert_eq!(pricing.cache_write_input_cost_per_token, Some(3.75e-6));
    assert_eq!(pricing.threshold_tokens, Some(200_000));
    assert_eq!(pricing.input_cost_per_token_above_threshold, Some(5e-6));
}

#[test]
fn decodes_providers_envelope() {
    let catalog = ModelsDevCatalog::decode(
        r#"{
            "providers": {
                "anthropic": {
                    "id": "anthropic",
                    "models": {
                        "claude-fresh": {
                            "id": "claude-fresh",
                            "cost": { "input": 3, "output": 15 }
                        }
                    }
                },
                "openai": {
                    "models": {
                        "gpt-fresh": {
                            "id": "gpt-fresh",
                            "cost": { "input": 2.5, "output": 10 }
                        }
                    }
                }
            }
        }"#,
    )
    .expect("enveloped catalog");

    assert_eq!(
        catalog
            .lookup("anthropic", "claude-fresh")
            .expect("pricing")
            .output_cost_per_token,
        15e-6
    );
}

#[test]
fn exact_lookup_matches_only_the_trimmed_key_or_model_id() {
    let catalog = ModelsDevCatalog::decode(
        r#"{
            "Nous": {
                "models": {
                    "z-ai/glm-5": {"id": "z-ai/glm-5", "cost": {"input": 1, "output": 2}},
                    "catalog-key": {"id": "deepseek/deepseek-v4", "cost": {"input": 3, "output": 4}},
                    "gpt-5": {"id": "gpt-5", "cost": {"input": 5, "output": 6}}
                }
            }
        }"#,
    )
    .expect("catalog");
    let input_rate = |model: &str| {
        catalog
            .lookup_exact(" NOUS ", model)
            .map(|pricing| pricing.input_cost_per_token)
    };

    assert_eq!(input_rate(" z-ai/glm-5 "), Some(1e-6));
    assert_eq!(input_rate("deepseek/deepseek-v4"), Some(3e-6));
    assert_eq!(input_rate("Z-AI/GLM-5"), None);
    // Aliases the fuzzy lookup resolves never supply an exact price.
    for alias in ["z-ai/glm-5@20260101", "z-ai/glm-5-20260101", "openai/gpt-5"] {
        assert!(catalog.lookup("nous", alias).is_some(), "{alias}");
        assert_eq!(input_rate(alias), None, "{alias}");
    }
}

#[test]
fn cache_artifact_is_versioned_and_expires_after_one_day() {
    let catalog = ModelsDevCatalog::decode(
        r#"{
            "openai": {
                "models": {
                    "gpt-fresh": {
                        "id": "gpt-fresh",
                        "cost": { "input": 2.5, "output": 10 }
                    }
                }
            },
            "anthropic": {
                "models": {
                    "claude-fresh": {
                        "id": "claude-fresh",
                        "cost": { "input": 3, "output": 15 }
                    }
                }
            }
        }"#,
    )
    .expect("catalog");
    let fetched_at = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let artifact = ModelsDevCacheArtifact::new(catalog, fetched_at);

    assert_eq!(artifact.version, ModelsDevCache::ARTIFACT_VERSION);
    assert!(!artifact.is_stale(fetched_at + Duration::from_secs(86_400)));
    assert!(artifact.is_stale(fetched_at + Duration::from_secs(86_401)));
    assert_eq!(
        ModelsDevCache::cache_path(Some(PathBuf::from("cache-root").as_path())),
        PathBuf::from("cache-root")
            .join("model-pricing")
            .join("models-dev-v1.json")
    );
}

#[tokio::test]
async fn concurrent_refreshes_for_one_cache_path_share_one_operation() {
    let coordinator = ModelsDevRefreshCoordinator::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let first_calls = Arc::clone(&calls);
    let path = PathBuf::from("pricing.json");
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);

    let first = coordinator.refresh(path.clone(), now, async move {
        first_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(10)).await;
        true
    });
    let second = coordinator.refresh(path, now, async {
        panic!("the second caller must await the first operation");
    });

    assert!(tokio::join!(first, second).0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_refresh_is_not_retried_within_the_attempt_window() {
    let coordinator = ModelsDevRefreshCoordinator::default();
    let path = PathBuf::from("pricing.json");
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);

    assert!(
        !coordinator
            .refresh(path.clone(), now, async { false })
            .await
    );
    assert!(
        !coordinator
            .refresh(path, now + Duration::from_secs(60), async {
                panic!("the 15-minute bound must suppress this attempt");
            })
            .await
    );
}

#[test]
fn cache_path_uses_the_existing_per_user_cache_root() {
    let cache_root = ModelsDevCache::default_cache_root().expect("per-user cache root");
    assert_eq!(
        ModelsDevCache::cache_path(None),
        cache_root.join("model-pricing").join("models-dev-v1.json")
    );
}
