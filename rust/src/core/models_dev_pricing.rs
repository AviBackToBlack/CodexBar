#[cfg(test)]
mod tests {
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
}

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex as AsyncMutex, watch};

use super::ModelsDevPricingTarget;

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const REFRESH_ATTEMPT_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Per-token pricing decoded from the models.dev catalog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_input_cost_per_token: Option<f64>,
    pub cache_write_input_cost_per_token: Option<f64>,
    pub threshold_tokens: Option<u64>,
    pub input_cost_per_token_above_threshold: Option<f64>,
    pub output_cost_per_token_above_threshold: Option<f64>,
    pub cache_read_input_cost_per_token_above_threshold: Option<f64>,
    pub cache_write_input_cost_per_token_above_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ModelsDevCatalog {
    providers: HashMap<String, ModelsDevProvider>,
}

/// How a refresh decides that a model already has a catalog price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelIdMatch {
    /// Dated, `@` and vendor-prefixed aliases may supply the price.
    Fuzzy,
    /// Only the trimmed catalog key or model id (upstream `exactModelID: true`).
    Exact,
}

/// Immutable models.dev view for callers that price many rows in one pass.
/// Loading this once avoids repeating cache metadata checks for every row.
#[derive(Debug, Clone)]
pub struct ModelsDevPricingSnapshot {
    artifact: Option<Arc<ModelsDevCacheArtifact>>,
}

impl ModelsDevPricingSnapshot {
    pub fn lookup(&self, provider_id: &str, model_id: &str) -> Option<DynamicModelPricing> {
        self.artifact
            .as_ref()
            .and_then(|artifact| artifact.catalog.lookup(provider_id, model_id))
    }

    /// Exact-id lookup (upstream `exactModelID: true`): the trimmed id must
    /// equal a catalog key or model id. No dated, `@`, or vendor-prefix alias
    /// of another model can supply the price.
    pub fn lookup_exact(&self, provider_id: &str, model_id: &str) -> Option<DynamicModelPricing> {
        self.artifact
            .as_ref()
            .and_then(|artifact| artifact.catalog.lookup_exact(provider_id, model_id))
    }

    #[cfg(test)]
    pub(crate) fn from_catalog_json_for_tests(json: &str) -> Option<Self> {
        let catalog = ModelsDevCatalog::decode(json)?;
        Some(Self {
            artifact: Some(Arc::new(ModelsDevCacheArtifact::new(
                catalog,
                SystemTime::now(),
            ))),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ModelsDevCatalogWire {
    Envelope {
        providers: HashMap<String, ModelsDevProvider>,
    },
    ProviderMap(HashMap<String, ModelsDevProvider>),
}

impl<'de> Deserialize<'de> for ModelsDevCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let providers = match ModelsDevCatalogWire::deserialize(deserializer)? {
            ModelsDevCatalogWire::Envelope { providers }
            | ModelsDevCatalogWire::ProviderMap(providers) => providers,
        };
        Ok(Self {
            providers: providers
                .into_iter()
                .map(|(key, provider)| {
                    (
                        normalize_provider_id(provider.id.as_deref().unwrap_or(&key)),
                        provider,
                    )
                })
                .collect(),
        })
    }
}

impl ModelsDevCatalog {
    #[cfg(test)]
    fn decode(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    fn lookup(&self, provider_id: &str, model_id: &str) -> Option<DynamicModelPricing> {
        let provider = self.providers.get(&normalize_provider_id(provider_id))?;
        let candidates = model_id_candidates(model_id);
        for candidate in &candidates {
            if let Some(model) = provider.models.get(candidate)
                && let Some(pricing) = DynamicModelPricing::from_model(model)
            {
                return Some(pricing);
            }
        }
        provider.models.values().find_map(|model| {
            let model_candidates = model_id_candidates(&model.id);
            candidates
                .iter()
                .any(|candidate| model_candidates.contains(candidate))
                .then(|| DynamicModelPricing::from_model(model))
                .flatten()
        })
    }

    fn lookup_exact(&self, provider_id: &str, model_id: &str) -> Option<DynamicModelPricing> {
        let provider = self.providers.get(&normalize_provider_id(provider_id))?;
        let model_id = normalize_model_id(model_id);
        provider
            .models
            .get(&model_id)
            .and_then(DynamicModelPricing::from_model)
            .or_else(|| {
                provider.models.values().find_map(|model| {
                    (normalize_model_id(&model.id) == model_id)
                        .then(|| DynamicModelPricing::from_model(model))
                        .flatten()
                })
            })
    }

    fn lookup_matching(
        &self,
        provider_id: &str,
        model_id: &str,
        matching: ModelIdMatch,
    ) -> Option<DynamicModelPricing> {
        match matching {
            ModelIdMatch::Fuzzy => self.lookup(provider_id, model_id),
            ModelIdMatch::Exact => self.lookup_exact(provider_id, model_id),
        }
    }

    fn is_plausible_refresh(&self) -> bool {
        ["openai", "anthropic"].into_iter().all(|provider_id| {
            self.providers
                .get(provider_id)
                .is_some_and(|provider| provider.models.values().any(ModelsDevModel::is_priceable))
        })
    }

    fn merge_priceable_entries_from(&mut self, cached: &Self) {
        for (provider_id, cached_provider) in &cached.providers {
            let provider = self
                .providers
                .entry(provider_id.clone())
                .or_insert_with(|| cached_provider.clone());
            let present_ids: HashSet<String> = provider
                .models
                .values()
                .filter(|model| model.is_priceable())
                .map(|model| stable_model_identity(&model.id))
                .collect();
            for (model_key, cached_model) in &cached_provider.models {
                if !cached_model.is_priceable()
                    || present_ids.contains(&stable_model_identity(&cached_model.id))
                {
                    continue;
                }
                let mut fallback_key = model_key.clone();
                if provider.models.contains_key(&fallback_key) {
                    fallback_key = format!(
                        "codexbar-fallback:{model_key}:{}",
                        normalize_model_id(&cached_model.id)
                    );
                }
                provider.models.insert(fallback_key, cached_model.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsDevProvider {
    id: Option<String>,
    #[serde(default)]
    models: HashMap<String, ModelsDevModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsDevModel {
    id: String,
    cost: Option<ModelsDevCost>,
}

impl ModelsDevModel {
    fn is_priceable(&self) -> bool {
        self.cost.as_ref().is_some_and(|cost| {
            cost.input.is_some_and(|rate| valid_number(&rate))
                && cost.output.is_some_and(|rate| valid_number(&rate))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsDevCost {
    input: Option<f64>,
    output: Option<f64>,
    #[serde(rename = "cache_read")]
    cache_read: Option<f64>,
    #[serde(rename = "cache_write")]
    cache_write: Option<f64>,
    #[serde(rename = "context_over_200k")]
    context_over_200k: Option<ModelsDevContextCost>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsDevContextCost {
    input: Option<f64>,
    output: Option<f64>,
    #[serde(rename = "cache_read")]
    cache_read: Option<f64>,
    #[serde(rename = "cache_write")]
    cache_write: Option<f64>,
}

impl DynamicModelPricing {
    fn from_model(model: &ModelsDevModel) -> Option<Self> {
        let cost = model.cost.as_ref()?;
        let input = cost.input.filter(valid_number)?;
        let output = cost.output.filter(valid_number)?;
        let above = cost.context_over_200k.as_ref();
        Some(Self {
            input_cost_per_token: per_token(input),
            output_cost_per_token: per_token(output),
            cache_read_input_cost_per_token: cost.cache_read.filter(valid_number).map(per_token),
            cache_write_input_cost_per_token: cost.cache_write.filter(valid_number).map(per_token),
            threshold_tokens: above.is_some().then_some(200_000),
            input_cost_per_token_above_threshold: above
                .and_then(|cost| cost.input)
                .filter(valid_number)
                .map(per_token),
            output_cost_per_token_above_threshold: above
                .and_then(|cost| cost.output)
                .filter(valid_number)
                .map(per_token),
            cache_read_input_cost_per_token_above_threshold: above
                .and_then(|cost| cost.cache_read)
                .filter(valid_number)
                .map(per_token),
            cache_write_input_cost_per_token_above_threshold: above
                .and_then(|cost| cost.cache_write)
                .filter(valid_number)
                .map(per_token),
        })
    }
}

fn valid_number(rate: &f64) -> bool {
    rate.is_finite() && *rate >= 0.0
}

fn per_token(rate: f64) -> f64 {
    rate / 1_000_000.0
}

fn normalize_provider_id(provider_id: &str) -> String {
    provider_id.trim().to_ascii_lowercase()
}

fn normalize_model_id(model_id: &str) -> String {
    model_id.trim().to_string()
}

fn stable_model_identity(model_id: &str) -> String {
    let model_id = normalize_model_id(model_id);
    if let Some((base, suffix)) = model_id.split_once('@') {
        if suffix == "default" {
            return base.to_string();
        }
        if suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return format!("{base}-{suffix}");
        }
    }
    model_id
}

fn model_id_candidates(model_id: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    append_model_candidate(&mut candidates, model_id.to_string());
    let mut index = 0;
    while index < candidates.len() {
        let candidate = candidates[index].clone();
        if let Some(rest) = candidate.strip_prefix("openai/") {
            append_model_candidate(&mut candidates, rest.to_string());
        }
        if let Some(rest) = candidate.strip_prefix("anthropic.") {
            append_model_candidate(&mut candidates, rest.to_string());
        }
        if candidate.contains("claude-")
            && let Some((_, tail)) = candidate.rsplit_once('.')
            && tail.starts_with("claude-")
        {
            append_model_candidate(&mut candidates, tail.to_string());
        }
        if let Some((base, suffix)) = candidate.split_once('@') {
            if suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
                append_model_candidate(&mut candidates, format!("{base}-{suffix}"));
            }
            append_model_candidate(&mut candidates, base.to_string());
        } else if candidate.starts_with("claude-") {
            append_model_candidate(&mut candidates, format!("{candidate}@default"));
        }
        if let Some(base) = candidate.strip_suffix("-v1:0") {
            append_model_candidate(&mut candidates, base.to_string());
        }
        if let Some(base) = strip_date_suffix(&candidate) {
            append_model_candidate(&mut candidates, base.to_string());
        }
        index += 1;
    }
    candidates
}

fn append_model_candidate(candidates: &mut Vec<String>, candidate: String) {
    let candidate = normalize_model_id(&candidate);
    if !candidate.is_empty() && !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn strip_date_suffix(model_id: &str) -> Option<&str> {
    let suffix = model_id.rsplit_once('-')?.1;
    if suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(&model_id[..model_id.len() - suffix.len() - 1]);
    }
    if suffix.len() != 2 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let without_day = &model_id[..model_id.len() - 3];
    let month = without_day.rsplit_once('-')?.1;
    if month.len() != 2 || !month.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let without_month = &without_day[..without_day.len() - 3];
    let year = without_month.rsplit_once('-')?.1;
    if year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()) {
        Some(&without_month[..without_month.len() - 5])
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsDevCacheArtifact {
    version: u32,
    fetched_at_unix_ms: u64,
    catalog: ModelsDevCatalog,
}

impl ModelsDevCacheArtifact {
    fn new(catalog: ModelsDevCatalog, fetched_at: SystemTime) -> Self {
        Self {
            version: ModelsDevCache::ARTIFACT_VERSION,
            fetched_at_unix_ms: unix_ms(fetched_at),
            catalog,
        }
    }

    fn fetched_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.fetched_at_unix_ms)
    }

    fn is_stale(&self, now: SystemTime) -> bool {
        now.duration_since(self.fetched_at()).unwrap_or_default() > CACHE_TTL
    }
}

struct ModelsDevCacheLoad {
    artifact: Option<Arc<ModelsDevCacheArtifact>>,
    is_stale: bool,
}

struct ModelsDevCacheMemoEntry {
    modified_at: Option<SystemTime>,
    size: Option<u64>,
    artifact: Option<Arc<ModelsDevCacheArtifact>>,
}

static CACHE_MEMO: LazyLock<Mutex<HashMap<PathBuf, ModelsDevCacheMemoEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct ModelsDevCache;

impl ModelsDevCache {
    const ARTIFACT_VERSION: u32 = 1;

    fn default_cache_root() -> Option<PathBuf> {
        dirs::cache_dir().map(|path| path.join("CodexBar"))
    }

    fn cache_path(cache_root: Option<&Path>) -> PathBuf {
        cache_root
            .map(Path::to_path_buf)
            .or_else(Self::default_cache_root)
            .map(|root| {
                root.join("model-pricing")
                    .join(format!("models-dev-v{}.json", Self::ARTIFACT_VERSION))
            })
            .unwrap_or_default()
    }
}

fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

impl ModelsDevCache {
    fn load(now: SystemTime, cache_root: Option<&Path>) -> ModelsDevCacheLoad {
        let cache_path = Self::cache_path(cache_root);
        if cache_path.as_os_str().is_empty() {
            return ModelsDevCacheLoad {
                artifact: None,
                is_stale: true,
            };
        }
        let cache_path = standardized_cache_path(&cache_path);
        let (modified_at, size) = file_identity(&cache_path);
        let memoized = {
            let memo = CACHE_MEMO.lock().expect("models.dev cache memo lock");
            memo.get(&cache_path)
                .filter(|entry| entry.modified_at == modified_at && entry.size == size)
                .map(|entry| entry.artifact.clone())
        };
        let artifact = memoized.unwrap_or_else(|| {
            let artifact = fs::read(&cache_path)
                .ok()
                .and_then(|contents| {
                    serde_json::from_slice::<ModelsDevCacheArtifact>(&contents).ok()
                })
                .filter(|artifact| artifact.version == Self::ARTIFACT_VERSION)
                .map(Arc::new);
            CACHE_MEMO
                .lock()
                .expect("models.dev cache memo lock")
                .insert(
                    cache_path,
                    ModelsDevCacheMemoEntry {
                        modified_at,
                        size,
                        artifact: artifact.clone(),
                    },
                );
            artifact
        });
        let is_stale = artifact
            .as_ref()
            .is_none_or(|artifact| artifact.is_stale(now));
        ModelsDevCacheLoad { artifact, is_stale }
    }
}

fn file_identity(path: &Path) -> (Option<SystemTime>, Option<u64>) {
    let Ok(metadata) = fs::metadata(path) else {
        return (None, None);
    };
    (metadata.modified().ok(), Some(metadata.len()))
}

fn standardized_cache_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|current_dir| current_dir.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

impl ModelsDevCache {
    fn save(catalog: ModelsDevCatalog, fetched_at: SystemTime, cache_root: Option<&Path>) -> bool {
        let cache_path = Self::cache_path(cache_root);
        if cache_path.as_os_str().is_empty() {
            return false;
        }
        let cache_path = standardized_cache_path(&cache_path);
        let Some(parent) = cache_path.parent() else {
            return false;
        };
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
        let artifact = Arc::new(ModelsDevCacheArtifact::new(catalog, fetched_at));
        let Ok(contents) = serde_json::to_vec(&*artifact) else {
            return false;
        };
        if crate::atomic_file::write_atomic(&cache_path, &contents).is_err() {
            return false;
        }
        let (modified_at, size) = file_identity(&cache_path);
        CACHE_MEMO
            .lock()
            .expect("models.dev cache memo lock")
            .insert(
                cache_path,
                ModelsDevCacheMemoEntry {
                    modified_at,
                    size,
                    artifact: Some(artifact),
                },
            );
        true
    }
}

#[cfg(test)]
mod models_dev_cache_atomic_tests {
    use super::ModelsDevCache;
    use tempfile::tempdir;

    #[test]
    fn failed_staged_replacement_preserves_previous_cache() {
        let root = tempdir().unwrap();
        let cache_path = ModelsDevCache::cache_path(Some(root.path()));
        let parent = cache_path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();

        crate::atomic_file::write_atomic(&cache_path, b"old-cache").unwrap();

        let staged = parent.join("failed-staged-replacement");
        std::fs::create_dir(&staged).unwrap();

        assert!(crate::atomic_file::replace_staged(&staged, &cache_path).is_err());
        assert_eq!(std::fs::read(cache_path).unwrap(), b"old-cache");
    }
}

#[derive(Default)]
struct ModelsDevRefreshCoordinator {
    state: Arc<AsyncMutex<ModelsDevRefreshState>>,
}

#[derive(Default)]
struct ModelsDevRefreshState {
    in_flight: HashMap<PathBuf, watch::Receiver<Option<bool>>>,
    last_attempt: HashMap<PathBuf, SystemTime>,
}

impl ModelsDevRefreshCoordinator {
    async fn refresh<F>(&self, cache_path: PathBuf, now: SystemTime, operation: F) -> bool
    where
        F: Future<Output = bool> + Send + 'static,
    {
        let cache_path = standardized_cache_path(&cache_path);
        let mut state = self.state.lock().await;
        if let Some(in_flight) = state.in_flight.get(&cache_path) {
            let receiver = in_flight.clone();
            drop(state);
            return wait_for_refresh(receiver).await;
        }
        if state
            .last_attempt
            .get(&cache_path)
            .is_some_and(|last_attempt| {
                now.duration_since(*last_attempt).unwrap_or_default() < REFRESH_ATTEMPT_WINDOW
            })
        {
            return false;
        }

        state.last_attempt.insert(cache_path.clone(), now);
        let (sender, receiver) = watch::channel(None);
        state.in_flight.insert(cache_path.clone(), receiver.clone());
        drop(state);

        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let result = operation.await;
            // Fire-and-forget result delivery; send fails only if all receivers
            // dropped, and the watch channel keeps the latest value regardless.
            let _delivered = sender.send(Some(result));
            state
                .lock()
                .await
                .in_flight
                .retain(|path, _| path != &cache_path);
        });
        wait_for_refresh(receiver).await
    }
}

async fn wait_for_refresh(mut receiver: watch::Receiver<Option<bool>>) -> bool {
    loop {
        if let Some(result) = *receiver.borrow() {
            return result;
        }
        if receiver.changed().await.is_err() {
            return false;
        }
    }
}

static REFRESH_COORDINATOR: LazyLock<ModelsDevRefreshCoordinator> =
    LazyLock::new(ModelsDevRefreshCoordinator::default);

/// Loads the cached models.dev catalog once for bulk-pricing callers.
pub fn pricing_snapshot() -> ModelsDevPricingSnapshot {
    pricing_snapshot_at(SystemTime::now(), None)
}

/// A stale catalog prices nothing: its rows stay unpriced until a refresh.
fn pricing_snapshot_at(now: SystemTime, cache_root: Option<&Path>) -> ModelsDevPricingSnapshot {
    let load = ModelsDevCache::load(now, cache_root);
    let artifact = (!load.is_stale).then_some(load.artifact).flatten();
    ModelsDevPricingSnapshot { artifact }
}

/// Looks up a cached models.dev price for a provider/model pair.
pub fn lookup(provider_id: &str, model_id: &str) -> Option<DynamicModelPricing> {
    let load = ModelsDevCache::load(SystemTime::now(), None);
    (!load.is_stale)
        .then_some(load.artifact)
        .flatten()
        .and_then(|artifact| artifact.catalog.lookup(provider_id, model_id))
}

/// Why a coordinated refresh runs. A stale-catalog refresh re-checks the
/// cache inside the coordinator, so callers that queued behind a refresh that
/// already landed do not fetch again (upstream `refreshStaleCache`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshReason {
    StaleCatalog,
    UnknownModels,
}

/// Refreshes the models.dev cache once when supplied models lack cached pricing.
///
/// Returns true only if at least one supplied model has pricing after the coordinated refresh.
pub async fn refresh_unknown_models_if_needed(
    provider_id: &str,
    model_ids: &HashSet<String>,
) -> bool {
    if model_ids.is_empty() {
        return false;
    }
    refresh_unknown_models_at(
        provider_id,
        model_ids,
        ModelIdMatch::Fuzzy,
        SystemTime::now(),
        None,
        fetch_catalog,
    )
    .await
}

/// Refreshes the models.dev cache for exact pricing identities (upstream
/// OpenCodex `refreshPricingIfNeeded`). A stale catalog refreshes first; then
/// each provider whose exact model ids still lack a price may trigger one
/// coordinated refresh, outside the 15-minute attempt window. Cached pricing
/// reads never start this network work; callers run it before a fresh build.
pub async fn refresh_exact_pricing_targets_if_needed(targets: &[ModelsDevPricingTarget]) {
    refresh_exact_pricing_targets_with(targets, SystemTime::now(), None, fetch_catalog).await;
}

async fn refresh_exact_pricing_targets_with<F, Fut>(
    targets: &[ModelsDevPricingTarget],
    now: SystemTime,
    cache_root: Option<&Path>,
    fetch: F,
) where
    F: FnOnce() -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Option<ModelsDevCatalog>> + Send + 'static,
{
    if targets.is_empty() {
        return;
    }
    if ModelsDevCache::load(now, cache_root).is_stale {
        let _refreshed =
            coordinated_refresh(now, cache_root, RefreshReason::StaleCatalog, fetch.clone()).await;
    }
    let mut grouped: BTreeMap<&str, HashSet<String>> = BTreeMap::new();
    for target in targets {
        grouped
            .entry(target.provider_id.as_str())
            .or_default()
            .insert(target.model_id.clone());
    }
    for (provider_id, model_ids) in grouped {
        let _priced = refresh_unknown_models_at(
            provider_id,
            &model_ids,
            ModelIdMatch::Exact,
            now,
            cache_root,
            fetch.clone(),
        )
        .await;
    }
}

async fn refresh_unknown_models_at<F, Fut>(
    provider_id: &str,
    model_ids: &HashSet<String>,
    matching: ModelIdMatch,
    now: SystemTime,
    cache_root: Option<&Path>,
    fetch: F,
) -> bool
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Option<ModelsDevCatalog>> + Send + 'static,
{
    // A stale catalog prices nothing (see `pricing_snapshot_at`).
    let is_priced = |load: &ModelsDevCacheLoad, model_id: &str| {
        !load.is_stale
            && load.artifact.as_ref().is_some_and(|artifact| {
                artifact
                    .catalog
                    .lookup_matching(provider_id, model_id, matching)
                    .is_some()
            })
    };
    let load = ModelsDevCache::load(now, cache_root);
    let unknown_models: Vec<&String> = model_ids
        .iter()
        .filter(|model_id| !is_priced(&load, model_id))
        .collect();
    if unknown_models.is_empty() {
        return true;
    }
    if load.artifact.as_ref().is_some_and(|artifact| {
        now.duration_since(artifact.fetched_at())
            .unwrap_or_default()
            < REFRESH_ATTEMPT_WINDOW
    }) {
        return false;
    }
    drop(load);

    let _refreshed =
        coordinated_refresh(now, cache_root, RefreshReason::UnknownModels, fetch).await;

    let refreshed = ModelsDevCache::load(now, cache_root);
    unknown_models
        .iter()
        .any(|model_id| is_priced(&refreshed, model_id))
}

/// Runs one fetch through the per-cache-path coordinator: concurrent callers
/// share it, and a path that attempted a refresh in the last 15 minutes waits.
async fn coordinated_refresh<F, Fut>(
    now: SystemTime,
    cache_root: Option<&Path>,
    reason: RefreshReason,
    fetch: F,
) -> bool
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Option<ModelsDevCatalog>> + Send + 'static,
{
    let cache_path = ModelsDevCache::cache_path(cache_root);
    if cache_path.as_os_str().is_empty() {
        return false;
    }
    let cache_root = cache_root.map(Path::to_path_buf);
    REFRESH_COORDINATOR
        .refresh(cache_path, now, async move {
            let cache_root = cache_root.as_deref();
            if reason == RefreshReason::StaleCatalog
                && !ModelsDevCache::load(now, cache_root).is_stale
            {
                return true;
            }
            match fetch().await {
                Some(catalog) => store_refreshed_catalog(catalog, now, cache_root),
                None => false,
            }
        })
        .await
}

/// Saves a plausible fetched catalog, keeping priceable entries that the
/// previous catalog had and the new one dropped.
fn store_refreshed_catalog(
    mut catalog: ModelsDevCatalog,
    now: SystemTime,
    cache_root: Option<&Path>,
) -> bool {
    if !catalog.is_plausible_refresh() {
        return false;
    }
    if let Some(cached) = ModelsDevCache::load(now, cache_root).artifact {
        catalog.merge_priceable_entries_from(&cached.catalog);
    }
    ModelsDevCache::save(catalog, now, cache_root)
}

async fn fetch_catalog() -> Option<ModelsDevCatalog> {
    let client = crate::core::apply_app_proxy(reqwest::Client::builder())
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?;
    let response = client.get(MODELS_DEV_URL).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<ModelsDevCatalog>().await.ok()
}

/// Saves `json` as the models.dev cache under `cache_root`, as a refresh at
/// `fetched_at` would.
#[cfg(test)]
pub(crate) fn save_catalog_json_for_tests(
    json: &str,
    fetched_at: SystemTime,
    cache_root: &Path,
) -> bool {
    ModelsDevCatalog::decode(json)
        .is_some_and(|catalog| ModelsDevCache::save(catalog, fetched_at, Some(cache_root)))
}

#[cfg(test)]
pub(crate) fn models_dev_cache_path_for_tests(cache_root: &Path) -> PathBuf {
    ModelsDevCache::cache_path(Some(cache_root))
}

#[cfg(test)]
pub(crate) fn pricing_snapshot_for_tests(
    now: SystemTime,
    cache_root: &Path,
) -> ModelsDevPricingSnapshot {
    pricing_snapshot_at(now, Some(cache_root))
}

/// Runs the exact-target refresh against `cache_root`; each fetch counts one
/// call and answers `response_json` (`None` is a failed download).
#[cfg(test)]
pub(crate) async fn refresh_exact_pricing_targets_for_tests(
    targets: &[ModelsDevPricingTarget],
    now: SystemTime,
    cache_root: &Path,
    response_json: Option<String>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
) {
    refresh_exact_pricing_targets_with(
        targets,
        now,
        Some(cache_root),
        counting_fetch(response_json, calls),
    )
    .await;
}

#[cfg(test)]
fn counting_fetch(
    response_json: Option<String>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
) -> impl FnOnce() -> std::future::Ready<Option<ModelsDevCatalog>> + Clone + Send + 'static {
    move || {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::ready(response_json.as_deref().and_then(ModelsDevCatalog::decode))
    }
}

#[cfg(test)]
mod exact_refresh_tests {
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
        refresh_exact_pricing_targets_with(
            &targets("gpt-fixture"),
            now(),
            Some(root.path()),
            fetch,
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture_input_rate(root.path(), "gpt-fixture"), Some(2e-6));
        // The replaced catalog keeps the earlier priceable model.
        assert_eq!(fixture_input_rate(root.path(), "gpt-other"), Some(1e-6));

        let fetch = counting_fetch(response, Arc::clone(&calls));
        refresh_exact_pricing_targets_with(
            &targets("gpt-fixture"),
            now(),
            Some(root.path()),
            fetch,
        )
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
        refresh_exact_pricing_targets_with(
            &targets("gpt-fixture"),
            now(),
            Some(root.path()),
            fetch,
        )
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
        refresh_exact_pricing_targets_with(
            &targets("gpt-fixture"),
            now(),
            Some(root.path()),
            fetch,
        )
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
}
