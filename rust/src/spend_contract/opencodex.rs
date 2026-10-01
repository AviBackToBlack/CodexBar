use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    CostUsagePricing, ModelsDevPricingSnapshot, ModelsDevPricingTarget, models_dev_pricing_targets,
};

use super::{
    CostCoverageCounts, CostProvenance, CustomPricing, CustomRates, ImportedSpendSource,
    SpendActivityCell, SpendDailyPoint, SpendModelRow, SpendTokenMix,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodexEntry {
    request_id: String,
    timestamp: DateTime<Utc>,
    provider: String,
    model: String,
    usage_status: String,
    conversation_id: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

mod cache;
mod nous;
#[cfg(test)]
mod nous_tests;
#[cfg(test)]
mod pricing_tests;

#[derive(Default)]
struct ModelAccumulator {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
    total: Option<u64>,
    cost: Option<f64>,
    custom_pricing: bool,
}

#[derive(Default)]
struct DailyAccumulator {
    cost: f64,
    saw_cost: bool,
    total_tokens: u64,
    saw_tokens: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteTarget {
    Subscription(&'static str),
    TokenOnly,
    Unknown,
}

fn route_provider(provider: &str) -> RouteTarget {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" => RouteTarget::Subscription("codex"),
        "opencode-go" => RouteTarget::Subscription("opencodego"),
        "kimi-coding" | "kimi-for-coding" => RouteTarget::Subscription("kimi"),
        "deepseek" => RouteTarget::Subscription("deepseek"),
        "nous" => RouteTarget::Subscription(nous::SUBSCRIPTION_ID),
        "opencode-free" | "opencode" => RouteTarget::TokenOnly,
        _ => RouteTarget::Unknown,
    }
}

fn route_model(model: &str) -> RouteTarget {
    let trimmed = model.trim();
    let Some((prefix, _)) = trimmed.split_once('/') else {
        return RouteTarget::Subscription("codex");
    };
    if prefix.is_empty() {
        RouteTarget::Unknown
    } else {
        route_provider(prefix)
    }
}

fn route_entry(entry: &OpenCodexEntry) -> RouteTarget {
    // OpenCodex normally records the billing provider separately from the model
    // namespace. Only the historical OpenAI transport format used the model
    // prefix as an explicit route; never let another provider's namespace
    // override its recorded provider.
    if entry.provider.trim().eq_ignore_ascii_case("openai") && entry.model.trim().contains('/') {
        let routed = route_model(&entry.model);
        if routed != RouteTarget::Unknown {
            return routed;
        }
    }
    route_provider(&entry.provider)
}

pub(super) fn load_for_subscription(
    provider_id: &str,
    history_days: u32,
    custom: &CustomPricing,
) -> Option<ImportedSpendSource> {
    let source_path = usage_path()?;
    let entries = cache::load_entries(&source_path)?;
    let entries = entries
        .into_iter()
        .filter(|entry| matches!(route_entry(entry), RouteTarget::Subscription(id) if id == provider_id))
        .collect();
    aggregate(entries, Utc::now(), history_days.clamp(1, 365), custom)
}

/// Refreshes the models.dev catalog when a ledger row that a Usage & Spend
/// build may import needs a price it lacks (upstream 0.60.4
/// `OpenCodexUsageStore.refreshPricingIfNeeded`).
pub(super) async fn refresh_pricing_if_needed() {
    let targets = tokio::task::spawn_blocking(|| {
        let entries = cache::load_entries(&usage_path()?)?;
        Some(pricing_targets(&entries, Utc::now()))
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default();
    crate::core::refresh_exact_pricing_targets_if_needed(&targets).await;
}

/// The exact models.dev identities the cached catalog must price for the
/// rows a build may show. Windows imports only rows routed to a
/// subscription. A direct OpenAI model with bundled Codex rates never reads
/// the catalog, and a model-less row is never priced, so neither refreshes.
fn pricing_targets(entries: &[OpenCodexEntry], now: DateTime<Utc>) -> Vec<ModelsDevPricingTarget> {
    let mut targets = BTreeSet::new();
    for entry in entries {
        if entry.timestamp > now
            || !is_priceable_status(entry)
            || !matches!(route_entry(entry), RouteTarget::Subscription(_))
        {
            continue;
        }
        let resolved = models_dev_pricing_targets(&pricing_provider(entry), &entry.model);
        match resolved.first() {
            Some(first) if is_codex_target(first) => {
                if !CostUsagePricing::has_bundled_codex_pricing(&first.model_id)
                    && !CostUsagePricing::is_codex_unattributed_model(&first.model_id)
                {
                    targets.insert(first.clone());
                }
            }
            _ => targets.extend(resolved),
        }
    }
    targets.into_iter().collect()
}

fn aggregate(
    entries: Vec<OpenCodexEntry>,
    now: DateTime<Utc>,
    history_days: u32,
    custom: &CustomPricing,
) -> Option<ImportedSpendSource> {
    aggregate_with_pricing(entries, now, history_days, custom, None)
}

/// `pricing_snapshot` pins the models.dev catalog; `None` reads the cached one.
fn aggregate_with_pricing(
    entries: Vec<OpenCodexEntry>,
    now: DateTime<Utc>,
    history_days: u32,
    custom: &CustomPricing,
    pricing_snapshot: Option<&ModelsDevPricingSnapshot>,
) -> Option<ImportedSpendSource> {
    let first_day = now.with_timezone(&Local).date_naive()
        - Duration::days(i64::from(history_days.saturating_sub(1)));

    // requestId is authoritative: a later row replaces an earlier row with the same id.
    let mut unique: HashMap<String, OpenCodexEntry> = HashMap::new();
    for entry in entries {
        unique.insert(entry.request_id.clone(), entry);
    }
    let mut entries: Vec<_> = unique
        .into_values()
        .filter(|entry| {
            entry.timestamp <= now
                && entry.timestamp.with_timezone(&Local).date_naive() >= first_day
        })
        .collect();
    entries.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.request_id.cmp(&right.request_id))
    });
    if entries.is_empty() {
        return None;
    }

    let mut conversations = HashSet::new();
    let mut token_mix = SpendTokenMix::default();
    let mut coverage = CostCoverageCounts::default();
    let mut activity: BTreeMap<(u8, u8), u32> = BTreeMap::new();
    let mut models: HashMap<String, ModelAccumulator> = HashMap::new();
    let mut daily: BTreeMap<String, DailyAccumulator> = BTreeMap::new();
    let mut known_cost = 0.0;
    let mut saw_known_cost = false;
    let mut saw_vendor_provenance = false;
    let mut saw_list_provenance = false;
    let mut saw_metered_cost = false;
    // Upstream 0.55.0 #3136: resolve the dynamic pricing catalog once per
    // aggregate instead of re-checking its cache metadata for every usage row.
    let pricing_snapshot = pricing_snapshot
        .cloned()
        .unwrap_or_else(crate::core::pricing_snapshot);

    for entry in &entries {
        // A row without a conversationId is its own session (upstream 0.68.0).
        let session = entry.conversation_id.as_ref().unwrap_or(&entry.request_id);
        conversations.insert(session.clone());
        token_mix.input_tokens = add_optional(token_mix.input_tokens, entry.input_tokens);
        token_mix.output_tokens = add_optional(token_mix.output_tokens, entry.output_tokens);
        token_mix.cache_read_tokens =
            add_optional(token_mix.cache_read_tokens, entry.cache_read_tokens);
        token_mix.cache_creation_tokens =
            add_optional(token_mix.cache_creation_tokens, entry.cache_creation_tokens);
        token_mix.reasoning_tokens =
            add_optional(token_mix.reasoning_tokens, entry.reasoning_tokens);

        let pricing = RowPricing::resolve(entry, custom);
        let cost = pricing.cost(entry, &pricing_snapshot);
        match entry.usage_status.as_str() {
            "reported" => saw_vendor_provenance = true,
            "estimated" => saw_list_provenance = true,
            _ => {}
        }
        match entry.usage_status.as_str() {
            "reported" if cost.is_some() => coverage.priced = coverage.priced.saturating_add(1),
            "estimated" if cost.is_some() => {
                coverage.estimated = coverage.estimated.saturating_add(1)
            }
            "unsupported" => coverage.unmetered = coverage.unmetered.saturating_add(1),
            _ => coverage.unpriced = coverage.unpriced.saturating_add(1),
        }
        if let Some(cost) = cost {
            known_cost += cost;
            saw_known_cost = true;
            if entry.usage_status == "reported" {
                saw_metered_cost = true;
            }
        }

        let local = entry.timestamp.with_timezone(&Local);
        // Weekday (0-6) and hour (0-23) both fit u8.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "weekday (0-6) and hour (0-23) fit u8"
        )]
        let key = (
            local.weekday().num_days_from_monday() as u8,
            local.hour() as u8,
        );
        activity.insert(
            key,
            activity.get(&key).copied().unwrap_or(0).saturating_add(1),
        );

        let day = daily
            .entry(local.date_naive().format("%Y-%m-%d").to_string())
            .or_default();
        if let Some(cost) = cost {
            day.cost += cost;
            day.saw_cost = true;
        }
        if let Some(total) = entry.resolved_total_tokens() {
            day.total_tokens = day.total_tokens.saturating_add(total);
            day.saw_tokens = true;
        }

        let model = models.entry(entry.model.clone()).or_default();
        model.input = model.input.saturating_add(entry.input_tokens.unwrap_or(0));
        model.output = model
            .output
            .saturating_add(entry.output_tokens.unwrap_or(0));
        model.cache_read = model
            .cache_read
            .saturating_add(entry.cache_read_tokens.unwrap_or(0));
        model.cache_creation = model
            .cache_creation
            .saturating_add(entry.cache_creation_tokens.unwrap_or(0));
        if let Some(total) = entry.resolved_total_tokens() {
            model.total = Some(model.total.unwrap_or(0).saturating_add(total));
        }
        if let Some(cost) = cost {
            model.cost = Some(model.cost.unwrap_or(0.0) + cost);
        }
        model.custom_pricing |= pricing.custom.is_some();
    }

    let mut model_rows: Vec<_> = models
        .into_iter()
        .map(|(model, acc)| SpendModelRow {
            model,
            cost_usd: acc.cost,
            input_tokens: acc.input,
            output_tokens: acc.output,
            cache_read_tokens: acc.cache_read,
            total_tokens: acc.total.unwrap_or_else(|| {
                acc.input
                    .saturating_add(acc.output)
                    .saturating_add(acc.cache_creation)
            }),
            custom_pricing: acc.custom_pricing,
        })
        .collect();
    model_rows.sort_by(|left, right| match (left.cost_usd, right.cost_usd) {
        (Some(a), Some(b)) => b
            .partial_cmp(&a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.model.cmp(&right.model)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.model.cmp(&right.model),
    });

    // Counts are clamped to u32::MAX before casting.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to u32::MAX before casting"
    )]
    let request_count = entries.len().min(u32::MAX as usize) as u32;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to u32::MAX before casting"
    )]
    let conversation_count = conversations.len().min(u32::MAX as usize) as u32;

    let snapshot_provenance =
        CostProvenance::from_source_kinds(saw_vendor_provenance, saw_list_provenance);
    let provenance =
        CostProvenance::for_window(snapshot_provenance, saw_known_cost, saw_metered_cost);

    Some(ImportedSpendSource {
        source_id: "opencodex".to_string(),
        display_name: "OpenCodex".to_string(),
        request_count,
        conversation_count,
        known_cost_usd: saw_known_cost.then_some(known_cost),
        provenance,
        token_mix,
        coverage,
        models: model_rows,
        daily: daily
            .into_iter()
            .map(|(day, acc)| SpendDailyPoint {
                day,
                cost_usd: acc.saw_cost.then_some(acc.cost),
                total_tokens: acc.saw_tokens.then_some(acc.total_tokens),
            })
            .collect(),
        hourly_activity: activity
            .into_iter()
            .map(|((weekday, hour), conversations)| SpendActivityCell {
                weekday,
                hour,
                conversations,
            })
            .collect(),
    })
}

/// Providers a legacy OpenAI-transport row may name in its model prefix as
/// the billing route (upstream `CostUsagePricing.codexModelsDevProviderIDs`).
const ROUTED_MODEL_PROVIDER_IDS: [&str; 7] = [
    "deepseek",
    "kimi-coding",
    "kimi-for-coding",
    "openai",
    "opencode",
    "opencode-free",
    "opencode-go",
];

/// The provider whose catalog prices `entry` (upstream
/// `OpenCodexUsagePricing.providerID(for:)`): the recorded provider, except
/// that a legacy OpenAI-transport row names a known route in its model prefix.
fn pricing_provider(entry: &OpenCodexEntry) -> String {
    let provider = entry.provider.trim().to_ascii_lowercase();
    if provider == "openai"
        && let Some((prefix, _)) = entry.model.trim().split_once('/')
    {
        let prefix = prefix.to_ascii_lowercase();
        if ROUTED_MODEL_PROVIDER_IDS.contains(&prefix.as_str()) {
            return prefix;
        }
    }
    provider
}

fn is_priceable_status(entry: &OpenCodexEntry) -> bool {
    matches!(entry.usage_status.as_str(), "reported" | "estimated")
}

/// A direct OpenAI model keeps the Codex pricing convention.
fn is_codex_target(target: &ModelsDevPricingTarget) -> bool {
    target.provider_id == "openai" && !target.model_id.contains('/')
}

/// The token lanes of a priceable row. A row without both input and output
/// is unpriced (upstream `listPriceUSD`); a missing cache lane is zero.
#[derive(Debug, Clone, Copy)]
struct RowTokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

impl RowTokens {
    fn of(entry: &OpenCodexEntry) -> Option<Self> {
        if !is_priceable_status(entry) {
            return None;
        }
        Some(Self {
            input: entry.input_tokens?,
            output: entry.output_tokens?,
            cache_read: entry.cache_read_tokens.unwrap_or(0),
            cache_write: entry.cache_creation_tokens.unwrap_or(0),
        })
    }
}

/// How one ledger row is priced (upstream 0.60.4 `OpenCodexUsagePricing`):
/// by a custom override, or by the models.dev catalog of its recorded
/// billing route. Another vendor's namespace in the model id never borrows
/// that vendor's rates.
struct RowPricing<'a> {
    /// The override that prices the row. It also marks the model row as
    /// custom-priced, even when its rates leave the cost unknown.
    custom: Option<&'a CustomRates>,
    targets: Vec<ModelsDevPricingTarget>,
}

impl<'a> RowPricing<'a> {
    /// Overrides resolve in upstream order: the recorded provider and model,
    /// then (for a resolvable row) the billing route, then each catalog
    /// identity. The first match wins whole; a rate it lacks is never filled
    /// from a later override or from the catalog.
    fn resolve(entry: &OpenCodexEntry, custom: &'a CustomPricing) -> Self {
        let provider = pricing_provider(entry);
        let targets = models_dev_pricing_targets(&provider, &entry.model);
        let custom = custom
            .overlay_rates(&entry.provider, &entry.model)
            .or_else(|| {
                targets
                    .first()
                    .and_then(|_| custom.overlay_rates(&provider, &entry.model))
            })
            .or_else(|| {
                targets
                    .iter()
                    .find_map(|target| custom.overlay_rates(&target.provider_id, &target.model_id))
            });
        Self { custom, targets }
    }

    fn cost(&self, entry: &OpenCodexEntry, snapshot: &ModelsDevPricingSnapshot) -> Option<f64> {
        let tokens = RowTokens::of(entry)?;
        let Some(rates) = self.custom else {
            return self.catalog_cost(tokens, entry.timestamp.date_naive(), snapshot);
        };
        // Custom rates keep the historical convention that input includes
        // cache reads and writes. Nous rows record input without them and
        // bill each lane on its own (upstream 0.68.0 Nous fixtures).
        let input = if route_entry(entry) == RouteTarget::Subscription(nous::SUBSCRIPTION_ID) {
            tokens.input
        } else {
            tokens
                .input
                .saturating_sub(tokens.cache_read)
                .saturating_sub(tokens.cache_write)
        };
        rates.lane_cost(input, tokens.output, tokens.cache_read, tokens.cache_write)
    }

    /// Upstream `providerCostUSD`. A direct OpenAI model keeps the Codex
    /// convention: inclusive input, request-day rates, bundled rates before
    /// the catalog. Every other identity needs an exact catalog entry and
    /// bills independent token lanes; a consumed cache lane without its own
    /// catalog rate leaves the row unpriced instead of borrowing the input
    /// rate.
    fn catalog_cost(
        &self,
        tokens: RowTokens,
        day: NaiveDate,
        snapshot: &ModelsDevPricingSnapshot,
    ) -> Option<f64> {
        let first = self.targets.first()?;
        if is_codex_target(first) {
            return CostUsagePricing::codex_cost_usd_at_date_with_cache_write_and_pricing_snapshot(
                &first.model_id,
                tokens.input,
                tokens.cache_read,
                tokens.cache_write,
                tokens.output,
                day,
                Some(snapshot),
            );
        }
        let pricing = self
            .targets
            .iter()
            .find_map(|target| snapshot.lookup_exact(&target.provider_id, &target.model_id))?;
        if (tokens.cache_read > 0 && pricing.cache_read_input_cost_per_token.is_none())
            || (tokens.cache_write > 0 && pricing.cache_write_input_cost_per_token.is_none())
        {
            return None;
        }
        let inclusive_input = tokens
            .input
            .checked_add(tokens.cache_read)?
            .checked_add(tokens.cache_write)?;
        Some(CostUsagePricing::models_dev_cost_usd(
            &pricing,
            inclusive_input,
            tokens.cache_read,
            tokens.cache_write,
            tokens.output,
        ))
    }
}

#[cfg(test)]
fn entry_cost(
    entry: &OpenCodexEntry,
    custom: &CustomPricing,
    snapshot: &ModelsDevPricingSnapshot,
) -> Option<f64> {
    RowPricing::resolve(entry, custom).cost(entry, snapshot)
}

fn usage_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("OPENCODEX_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("usage.jsonl"));
        }
    }
    dirs::home_dir().map(|home| home.join(".opencodex").join("usage.jsonl"))
}

fn parse_line(line: &str) -> Option<OpenCodexEntry> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let request_id = value.get("requestId")?.as_str()?.trim().to_string();
    let model = value.get("model")?.as_str()?.trim().to_string();
    if request_id.is_empty() || model.is_empty() {
        return None;
    }
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openai")
        .to_string();
    let timestamp = parse_timestamp(value.get("timestamp")?)?;
    let usage = value.get("usage").and_then(Value::as_object);
    Some(OpenCodexEntry {
        request_id,
        timestamp,
        provider,
        model,
        usage_status: value
            .get("usageStatus")
            .and_then(Value::as_str)
            .unwrap_or("unreported")
            .trim()
            .to_ascii_lowercase(),
        conversation_id: value
            .get("conversationId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        input_tokens: usage.and_then(|object| nonnegative_u64(object.get("inputTokens"))),
        output_tokens: usage.and_then(|object| nonnegative_u64(object.get("outputTokens"))),
        cache_read_tokens: usage.and_then(|object| {
            nonnegative_u64(object.get("cacheReadInputTokens"))
                .or_else(|| nonnegative_u64(object.get("cachedInputTokens")))
        }),
        cache_creation_tokens: usage
            .and_then(|object| nonnegative_u64(object.get("cacheCreationInputTokens"))),
        reasoning_tokens: usage
            .and_then(|object| nonnegative_u64(object.get("reasoningOutputTokens"))),
        // The Hermes extractor reports the total inside `usage`.
        total_tokens: nonnegative_u64(value.get("totalTokens"))
            .or_else(|| usage.and_then(|object| nonnegative_u64(object.get("totalTokens")))),
    })
}

impl OpenCodexEntry {
    fn resolved_total_tokens(&self) -> Option<u64> {
        self.total_tokens.or_else(|| {
            let mut saw = false;
            let mut total = 0u64;
            for value in [
                self.input_tokens,
                self.output_tokens,
                self.cache_creation_tokens,
            ]
            .into_iter()
            .flatten()
            {
                saw = true;
                total = total.saturating_add(value);
            }
            saw.then_some(total)
        })
    }
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(raw) = value.as_str() {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(raw.trim()) {
            return Some(parsed.with_timezone(&Utc));
        }
        if let Ok(number) = raw.trim().parse::<f64>() {
            return timestamp_from_epoch(number);
        }
    }
    value.as_f64().and_then(timestamp_from_epoch)
}

fn timestamp_from_epoch(value: f64) -> Option<DateTime<Utc>> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let seconds = if value >= 1_000_000_000_000.0 {
        value / 1000.0
    } else {
        value
    };
    // Epoch timestamps fit i64 seconds; float-to-int casts saturate otherwise.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "epoch timestamps fit i64 seconds"
    )]
    let whole = seconds.trunc() as i64;
    // Fractional seconds in [0, 1e9) fit u32.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "fractional seconds in [0, 1e9) fit u32"
    )]
    let nanos = (seconds.fract().abs() * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(whole, nanos).single()
}

fn nonnegative_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    let number = value.as_f64()?;
    // Guarded to finite non-negative values within u64 range.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "guarded to finite non-negative values within u64 range"
    )]
    let parsed =
        (number.is_finite() && number >= 0.0 && number <= u64::MAX as f64).then_some(number as u64);
    parsed
}

fn add_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::CustomRates;
    use super::cache::{load_entries_with_cache, read_cache};
    use super::*;
    use std::fs;

    #[test]
    fn aggregate_deduplicates_requests_and_applies_history_window() {
        let now = DateTime::parse_from_rfc3339("2026-08-19T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let make = |request_id: &str, timestamp: &str, input: u64| OpenCodexEntry {
            request_id: request_id.into(),
            timestamp: DateTime::parse_from_rfc3339(timestamp)
                .unwrap()
                .with_timezone(&Utc),
            provider: "openai".into(),
            model: "gpt-5".into(),
            usage_status: "reported".into(),
            conversation_id: Some(request_id.into()),
            input_tokens: Some(input),
            output_tokens: Some(1),
            cache_read_tokens: Some(0),
            cache_creation_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(input + 1),
        };
        let source = aggregate(
            vec![
                make("same", "2026-08-18T10:00:00Z", 10),
                make("same", "2026-08-18T11:00:00Z", 20),
                make("old", "2026-08-01T10:00:00Z", 30),
            ],
            now,
            7,
            &CustomPricing::default(),
        )
        .expect("source");
        assert_eq!(source.request_count, 1);
        assert_eq!(source.conversation_count, 1);
        assert_eq!(source.token_mix.input_tokens, Some(20));
        assert_eq!(source.coverage.priced, 1);
        assert!(source.known_cost_usd.is_some());
        assert_eq!(source.provenance, CostProvenance::VendorMetered);
    }

    #[test]
    fn aggregate_preserves_list_and_mixed_provenance() {
        let now = DateTime::parse_from_rfc3339("2026-08-19T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut estimated = entry("openai", "gpt-5");
        estimated.request_id = "estimated".to_string();
        estimated.usage_status = "estimated".to_string();
        let list_only = aggregate(vec![estimated.clone()], now, 30, &CustomPricing::default())
            .expect("list-price source");
        assert_eq!(list_only.provenance, CostProvenance::ListPriceEstimate);

        let mut reported = entry("openai", "gpt-5");
        reported.request_id = "reported".to_string();
        let mixed = aggregate(
            vec![reported, estimated],
            now,
            30,
            &CustomPricing::default(),
        )
        .expect("mixed source");
        assert_eq!(mixed.provenance, CostProvenance::Mixed);
    }

    #[test]
    fn aggregate_preserves_zero_cost_authoritative_provenance() {
        let now = DateTime::parse_from_rfc3339("2026-08-19T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let custom = CustomPricing {
            entries: std::collections::HashMap::from([(
                "openai/gpt-5".to_string(),
                CustomRates {
                    input: Some(0.0),
                    output: Some(0.0),
                    cache_read: Some(0.0),
                    cache_write: Some(0.0),
                },
            )]),
        };

        let reported = aggregate(vec![entry("openai", "gpt-5")], now, 30, &custom)
            .expect("zero-cost vendor source");
        assert_eq!(reported.known_cost_usd, Some(0.0));
        assert_eq!(reported.provenance, CostProvenance::VendorMetered);

        let mut estimated_entry = entry("openai", "gpt-5");
        estimated_entry.usage_status = "estimated".to_string();
        let estimated =
            aggregate(vec![estimated_entry], now, 30, &custom).expect("zero-cost list source");
        assert_eq!(estimated.known_cost_usd, Some(0.0));
        assert_eq!(estimated.provenance, CostProvenance::ListPriceEstimate);
    }

    fn entry(provider: &str, model: &str) -> OpenCodexEntry {
        OpenCodexEntry {
            request_id: format!("{provider}:{model}"),
            timestamp: DateTime::parse_from_rfc3339("2026-07-29T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            provider: provider.to_string(),
            model: model.to_string(),
            usage_status: "reported".to_string(),
            conversation_id: None,
            input_tokens: Some(100),
            output_tokens: Some(5),
            cache_read_tokens: Some(10),
            cache_creation_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(105),
        }
    }

    #[test]
    fn routes_opencodex_entries_into_subscription_rows() {
        assert_eq!(
            route_entry(&entry("openai", "gpt-5.6-sol")),
            RouteTarget::Subscription("codex")
        );
        assert_eq!(
            route_entry(&entry("opencode-go", "gpt-5.6-sol")),
            RouteTarget::Subscription("opencodego")
        );
        assert_eq!(
            route_entry(&entry("kimi-coding", "k2p5")),
            RouteTarget::Subscription("kimi")
        );
        assert_eq!(
            route_entry(&entry("deepseek", "deepseek-chat")),
            RouteTarget::Subscription("deepseek")
        );
        assert_eq!(
            route_entry(&entry("opencode-free", "free-model")),
            RouteTarget::TokenOnly
        );
    }

    #[test]
    fn recorded_provider_wins_over_mismatched_model_namespace() {
        assert_eq!(
            route_entry(&entry("opencode-go", "openai/gpt-5.6-sol")),
            RouteTarget::Subscription("opencodego")
        );
        assert_eq!(
            route_entry(&entry("deepseek", "openai/gpt-5.6-sol")),
            RouteTarget::Subscription("deepseek")
        );
    }

    /// The models.dev identities that price a row, as (provider, model).
    fn targets_of(provider: &str, model: &str) -> Vec<(String, String)> {
        models_dev_pricing_targets(&pricing_provider(&entry(provider, model)), model)
            .into_iter()
            .map(|target| (target.provider_id, target.model_id))
            .collect()
    }

    fn pair(provider: &str, model: &str) -> (String, String) {
        (provider.to_string(), model.to_string())
    }

    #[test]
    fn legacy_openai_transport_still_uses_explicit_route() {
        assert_eq!(
            route_entry(&entry("openai", "opencode-go/deepseek-v4-flash")),
            RouteTarget::Subscription("opencodego")
        );
        assert_eq!(
            targets_of("openai", "opencode-go/gpt-5"),
            vec![pair("opencode-go", "gpt-5")]
        );
    }

    #[test]
    fn pricing_targets_follow_the_recorded_provider() {
        assert_eq!(
            targets_of("opencode-go", "gpt-5"),
            vec![pair("opencode-go", "gpt-5")]
        );
        assert_eq!(
            targets_of("kimi-coding", "k2p5"),
            vec![pair("kimi-coding", "k2p5"), pair("kimi-for-coding", "k2p5")]
        );
        assert_eq!(
            targets_of("deepseek", "deepseek-chat"),
            vec![pair("deepseek", "deepseek-chat")]
        );
        // Another vendor's namespace is part of the model id on the recorded
        // provider's catalog, never a route to that vendor's own rates.
        assert_eq!(
            targets_of("opencode-go", "openai/gpt-5"),
            vec![pair("opencode-go", "openai/gpt-5")]
        );
    }

    #[test]
    fn unknown_provider_or_namespace_fails_closed_for_routing_and_pricing() {
        let snapshot = ModelsDevPricingSnapshot::from_catalog_json_for_tests(
            r#"{"openai":{"models":{"gpt-5":{"id":"gpt-5","cost":{"input":2,"output":8,"cache_read":0.2}}}}}"#,
        )
        .expect("catalog");
        let none = CustomPricing::default();
        let proxy = entry("private-proxy", "openai/gpt-5");
        assert_eq!(route_entry(&proxy), RouteTarget::Unknown);
        assert_eq!(entry_cost(&proxy, &none, &snapshot), None);
        let malformed = entry("openai", "/gpt-5");
        assert_eq!(route_entry(&malformed), RouteTarget::Subscription("codex"));
        assert!(targets_of("openai", "/gpt-5").is_empty());
        assert_eq!(entry_cost(&malformed, &none, &snapshot), None);
    }

    #[test]
    fn opencodex_uses_request_day_for_historical_gpt56_pricing() {
        let entry = entry("openai", "gpt-5.6-terra");
        let empty = ModelsDevPricingSnapshot::from_catalog_json_for_tests("{}").expect("catalog");
        let cost = entry_cost(&entry, &CustomPricing::default(), &empty).unwrap();
        let expected = 90.0 * 2.5e-6 + 10.0 * 2.5e-7 + 5.0 * 1.5e-5;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn opencodex_historical_gpt56_bills_cache_writes_at_their_own_rate() {
        let mut entry = entry("openai", "gpt-5.6-terra");
        entry.cache_creation_tokens = Some(20);
        let empty = ModelsDevPricingSnapshot::from_catalog_json_for_tests("{}").expect("catalog");
        let cost = entry_cost(&entry, &CustomPricing::default(), &empty).unwrap();
        // Input includes cache reads and writes; each lane has its own rate.
        let expected = 70.0 * 2.5e-6 + 10.0 * 2.5e-7 + 20.0 * 3.125e-6 + 5.0 * 1.5e-5;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn parser_keeps_reported_token_classes() {
        let value = serde_json::json!({
            "requestId": "r1", "timestamp": "2026-08-18T10:00:00Z", "provider": "openai",
            "model": "gpt-test", "usageStatus": "reported", "conversationId": "c1",
            "usage": {"inputTokens": 10, "outputTokens": 4, "cachedInputTokens": 3, "reasoningOutputTokens": 2}
        });
        let entry = parse_line(&value.to_string()).expect("entry");
        assert_eq!(entry.model, "gpt-test");
        assert_eq!(entry.input_tokens, Some(10));
        assert_eq!(entry.output_tokens, Some(4));
        assert_eq!(entry.cache_read_tokens, Some(3));
        assert_eq!(entry.reasoning_tokens, Some(2));
    }

    #[test]
    fn parser_normalizes_defaults_and_rejects_malformed_lines() {
        let minimal = serde_json::json!({
            "requestId": "  r1  ", "model": "gpt-test", "timestamp": "2026-08-18T10:00:00Z",
            "usageStatus": "  REPORTED ", "usage": {"cacheCreationInputTokens": 7}
        });
        let entry = parse_line(&minimal.to_string()).expect("entry");
        assert_eq!(entry.request_id, "r1", "ids are trimmed");
        assert_eq!(
            entry.provider, "openai",
            "missing provider defaults to openai"
        );
        assert_eq!(
            entry.usage_status, "reported",
            "status is lowercased and trimmed"
        );
        assert_eq!(entry.conversation_id, None);
        assert_eq!(entry.cache_creation_tokens, Some(7));

        for malformed in [
            "{}",
            r#"{"requestId": "", "model": "m", "timestamp": "2026-08-18T10:00:00Z"}"#,
            r#"{"requestId": "r1", "model": "   ", "timestamp": "2026-08-18T10:00:00Z"}"#,
            r#"{"requestId": "r1", "model": "m"}"#,
            "not json at all",
        ] {
            assert!(parse_line(malformed).is_none(), "rejected: {malformed}");
        }
    }

    #[test]
    fn incremental_cache_appends_only_newline_terminated_tail() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("usage.jsonl");
        let cache = dir.path().join("cache.sqlite");
        let row = |id: &str, input: u64| {
            format!(
                r#"{{"requestId":"{id}","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z","usageStatus":"reported","usage":{{"inputTokens":{input}}}}}"#
            )
        };

        fs::write(&log, format!("{}\n{}\n", row("a", 1), row("b", 2))).unwrap();
        let first = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            first
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        let first_cursor = read_cache(&cache).unwrap().cursor;

        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        use std::io::Write as _;
        writeln!(file, "{}", row("c", 3)).unwrap();
        drop(file);

        let second = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            second
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        let second_cursor = read_cache(&cache).unwrap().cursor;
        assert!(second_cursor.parsed_offset > first_cursor.parsed_offset);
    }

    #[test]
    fn incomplete_trailing_opencodex_record_waits_for_newline() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("usage.jsonl");
        let cache = dir.path().join("cache.sqlite");
        let complete = r#"{"requestId":"a","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z"}"#;
        let pending = r#"{"requestId":"b","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z"}"#;
        let split = pending.len() / 2;
        fs::write(&log, format!("{complete}\n{}", &pending[..split])).unwrap();

        let first = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            first
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a"]
        );
        let cursor = read_cache(&cache).unwrap().cursor;
        assert_eq!(
            cursor.parsed_offset,
            u64::try_from(complete.len() + 1).unwrap()
        );

        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        use std::io::Write as _;
        writeln!(file, "{}", &pending[split..]).unwrap();
        drop(file);
        let second = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            second
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn complete_trailing_opencodex_record_waits_for_newline() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("usage.jsonl");
        let cache = dir.path().join("cache.sqlite");
        let first = r#"{"requestId":"a","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z"}"#;
        let trailing = r#"{"requestId":"b","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z"}"#;
        fs::write(&log, format!("{first}\n{trailing}")).unwrap();

        let before_newline = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            before_newline
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a"]
        );

        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        use std::io::Write as _;
        writeln!(file).unwrap();
        drop(file);

        let after_newline = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            after_newline
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn later_request_id_replaces_cached_entry_without_full_cache_loss() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("usage.jsonl");
        let cache = dir.path().join("cache.sqlite");
        let row = |id: &str, input: u64| {
            format!(
                r#"{{"requestId":"{id}","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z","usageStatus":"reported","usage":{{"inputTokens":{input}}}}}"#
            )
        };
        fs::write(&log, format!("{}\n{}\n", row("dup", 1), row("keep", 2))).unwrap();
        let _ = load_entries_with_cache(&log, &cache).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        use std::io::Write as _;
        writeln!(file, "{}", row("dup", 9)).unwrap();
        drop(file);

        let entries = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.request_id == "dup")
                .unwrap()
                .input_tokens,
            Some(9)
        );
        assert!(entries.iter().any(|entry| entry.request_id == "keep"));
    }

    #[test]
    fn truncation_invalidates_opencodex_cursor_and_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("usage.jsonl");
        let cache = dir.path().join("cache.sqlite");
        let old = r#"{"requestId":"old","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z"}"#;
        let replacement =
            r#"{"requestId":"new","model":"gpt-5","timestamp":"2026-08-18T10:00:00Z"}"#;
        fs::write(&log, format!("{old}\n{old}\n")).unwrap();
        let _ = load_entries_with_cache(&log, &cache).unwrap();
        fs::write(&log, format!("{replacement}\n")).unwrap();

        let rebuilt = load_entries_with_cache(&log, &cache).unwrap();
        assert_eq!(
            rebuilt
                .iter()
                .map(|entry| entry.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new"]
        );
    }

    #[test]
    fn timestamps_parse_rfc3339_epoch_seconds_and_millis() {
        let expected = Utc.with_ymd_and_hms(2026, 8, 18, 10, 0, 0).unwrap();
        let rfc3339 = serde_json::json!("2026-08-18T10:00:00Z");
        assert_eq!(parse_timestamp(&rfc3339), Some(expected));
        let epoch_seconds = serde_json::json!(1_787_047_200i64);
        assert_eq!(parse_timestamp(&epoch_seconds), Some(expected));
        let epoch_millis = serde_json::json!(1_787_047_200_000f64);
        assert_eq!(parse_timestamp(&epoch_millis), Some(expected));
        let numeric_string = serde_json::json!("1787047200.0");
        assert_eq!(parse_timestamp(&numeric_string), Some(expected));
        for invalid in [
            serde_json::json!("not a date"),
            serde_json::json!(0),
            serde_json::json!(-5.0),
            serde_json::Value::Null,
            serde_json::json!(true),
        ] {
            assert!(parse_timestamp(&invalid).is_none(), "rejected: {invalid}");
        }
    }

    #[test]
    fn nonnegative_u64_accepts_json_numbers_and_bounded_floats() {
        assert_eq!(nonnegative_u64(Some(&serde_json::json!(42))), Some(42));
        assert_eq!(nonnegative_u64(Some(&serde_json::json!(12.0))), Some(12));
        // Fractional floats are accepted via `as u64` truncation.
        assert_eq!(nonnegative_u64(Some(&serde_json::json!(1.5))), Some(1));
        // `u64::MAX as f64` rounds up to 2^64; f64 spacing there is 4096, so
        // +2048.0 rounds back into range. First out-of-range step is +4096.0.
        assert_eq!(
            nonnegative_u64(Some(&serde_json::json!(u64::MAX as f64 + 4096.0))),
            None
        );
        assert_eq!(nonnegative_u64(None), None);
    }
}
