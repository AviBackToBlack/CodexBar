//! Unified Usage & Spend accounting contract for upstream 0.53 parity.
//! Accounting semantics live here so UI/CLI never infer unknown vs zero.

mod opencodex;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};

use crate::codex_workspaces::{CodexWorkspacesIndex, ProjectUsage, SessionUsage, SourceStatus};
use crate::cost_scanner::{
    CostScanner, CostSummary, ModelTokenCounts, get_daily_cost_history, get_daily_token_history,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CostProvenance {
    ListPriceEstimate,
    VendorMetered,
    Mixed,
    Unknown,
}

impl CostProvenance {
    /// Narrow snapshot provenance to the costs actually present in a window.
    ///
    /// This mirrors upstream 0.56.2 `CostProvenance.forWindow`: a vendor source
    /// remains vendor-metered when it has window costs, while a mixed source is
    /// mixed only when both its list-price and metered sides are present.
    pub(crate) fn for_window(
        snapshot: Self,
        has_window_costs: bool,
        includes_metered: bool,
    ) -> Self {
        match snapshot {
            Self::VendorMetered => {
                if includes_metered || has_window_costs {
                    Self::VendorMetered
                } else {
                    Self::Unknown
                }
            }
            Self::Mixed => match (includes_metered, has_window_costs) {
                (true, true) => Self::Mixed,
                (true, false) => Self::VendorMetered,
                (false, true) => Self::ListPriceEstimate,
                (false, false) => Self::Unknown,
            },
            Self::ListPriceEstimate => {
                if has_window_costs {
                    Self::ListPriceEstimate
                } else {
                    Self::Unknown
                }
            }
            Self::Unknown => Self::Unknown,
        }
    }

    fn from_source_kinds(includes_vendor: bool, includes_list: bool) -> Self {
        match (includes_vendor, includes_list) {
            (true, true) => Self::Mixed,
            (true, false) => Self::VendorMetered,
            (false, true) => Self::ListPriceEstimate,
            (false, false) => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostCoverageCounts {
    pub priced: u32,
    pub unpriced: u32,
    pub unmetered: u32,
    pub estimated: u32,
}

impl CostCoverageCounts {
    pub fn total(&self) -> u32 {
        self.checked_total().unwrap_or(u32::MAX)
    }

    fn checked_total(&self) -> Option<u32> {
        self.priced
            .checked_add(self.unpriced)?
            .checked_add(self.unmetered)?
            .checked_add(self.estimated)
    }

    pub fn coverage_ratio(&self) -> Option<f64> {
        let denominator = self.checked_total()?;
        let covered = self.priced.checked_add(self.estimated)?;
        (denominator > 0).then(|| covered as f64 / denominator as f64)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendTokenMix {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    /// Keeps arithmetic overflow distinct from an ordinary missing class while
    /// the report is merged in memory.  It is deliberately not part of the
    /// wire contract: both cases are exposed as unknown (`None`).
    #[serde(skip)]
    pub(crate) overflowed_classes: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendModelRow {
    pub model: String,
    /// None = unknown/unpriced. Some(0.0) = known free.
    pub cost_usd: Option<f64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
    pub custom_pricing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendDailyPoint {
    pub day: String,
    pub cost_usd: Option<f64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendActivityCell {
    /// Monday=0, Sunday=6.
    pub weekday: u8,
    pub hour: u8,
    pub conversations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedSpendSource {
    pub source_id: String,
    pub display_name: String,
    pub request_count: u32,
    pub conversation_count: u32,
    pub known_cost_usd: Option<f64>,
    pub provenance: CostProvenance,
    pub token_mix: SpendTokenMix,
    pub coverage: CostCoverageCounts,
    pub models: Vec<SpendModelRow>,
    pub daily: Vec<SpendDailyPoint>,
    pub hourly_activity: Vec<SpendActivityCell>,
}

struct NativeSpendData {
    projects: Vec<ProjectUsage>,
    conversations: Vec<SessionUsage>,
    project_source_status: Option<SourceStatus>,
    activity: Vec<SpendActivityCell>,
    daily: Vec<SpendDailyPoint>,
}

struct ResolvedSpendData {
    known_cost_usd: Option<f64>,
    provenance: CostProvenance,
    price_coverage: CostCoverageCounts,
    price_coverage_exact: bool,
    token_mix: SpendTokenMix,
    models: Vec<SpendModelRow>,
    daily: Vec<SpendDailyPoint>,
    hourly_activity: Vec<SpendActivityCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendContract {
    pub provider_id: String,
    pub history_days: u32,
    /// Known subtotal for this window. None means unknown, never implicit zero.
    pub known_cost_usd: Option<f64>,
    pub known_zero: bool,
    pub provenance: CostProvenance,
    pub price_coverage: CostCoverageCounts,
    pub price_coverage_ratio: Option<f64>,
    pub history_coverage_established: bool,
    pub token_mix: SpendTokenMix,
    pub conversation_count: u32,
    pub models: Vec<SpendModelRow>,
    pub projects: Vec<ProjectUsage>,
    pub conversations: Vec<SessionUsage>,
    pub daily: Vec<SpendDailyPoint>,
    pub hourly_activity: Vec<SpendActivityCell>,
    pub project_source_status: Option<SourceStatus>,
    pub custom_pricing_active: bool,
    pub imports: Vec<ImportedSpendSource>,
}

#[derive(Debug, Clone, Default)]
struct CustomPricing {
    entries: HashMap<String, CustomRates>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CustomRates {
    input: Option<f64>,
    output: Option<f64>,
    #[serde(rename = "cacheRead", alias = "cache_read")]
    cache_read: Option<f64>,
    #[serde(
        rename = "cacheWrite",
        alias = "cache_write",
        alias = "cacheCreation",
        alias = "cache_creation"
    )]
    cache_write: Option<f64>,
}

impl CustomPricing {
    fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|path| path.join("CodexBar").join("custom-pricing.json"))
    }

    fn load() -> Self {
        Self::default_path()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<HashMap<String, CustomRates>>(&bytes).ok())
            .map(|entries| Self {
                entries: entries
                    .into_iter()
                    .filter_map(|(key, rates)| {
                        let key = key.trim().to_ascii_lowercase();
                        (!key.is_empty() && rates.is_valid()).then_some((key, rates))
                    })
                    .collect(),
            })
            .unwrap_or_default()
    }

    fn rates(&self, provider_id: &str, model: &str) -> Option<&CustomRates> {
        let model_key = model.trim().to_ascii_lowercase();
        let provider_key = format!("{}/{}", provider_id.trim().to_ascii_lowercase(), model_key);
        self.entries
            .get(&provider_key)
            .or_else(|| self.entries.get(&model_key))
    }
}

impl CustomRates {
    fn is_valid(&self) -> bool {
        [self.input, self.output, self.cache_read, self.cache_write]
            .into_iter()
            .flatten()
            .all(|value| value.is_finite() && value >= 0.0)
    }

    fn cost(&self, counts: &ModelTokenCounts) -> Option<f64> {
        self.cost_parts(
            counts.input_tokens,
            counts.output_tokens,
            counts.cached_tokens,
            0,
        )
    }

    fn cost_parts(
        &self,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> Option<f64> {
        let cached = cache_read.min(input);
        let uncached = input.saturating_sub(cached);
        let mut total = 0.0;
        if uncached > 0 {
            total += uncached as f64 * self.input? / 1_000_000.0;
        }
        if output > 0 {
            total += output as f64 * self.output? / 1_000_000.0;
        }
        if cached > 0 {
            total += cached as f64 * self.cache_read? / 1_000_000.0;
        }
        if cache_write > 0 {
            total += cache_write as f64 * self.cache_write? / 1_000_000.0;
        }
        total.is_finite().then_some(total)
    }
}

/// Build a stable accounting contract for a local-log provider.
///  means the upstream All-time UI window, bounded to 365 days locally.
pub fn build_local_spend_contract(
    provider_id: &str,
    days: u32,
    include_opencodex: bool,
) -> SpendContract {
    let history_days = if days == 0 { 365 } else { days.clamp(1, 365) };
    let scanner = CostScanner::new(history_days);
    let summary = match provider_id {
        "codex" => scanner.scan_codex(),
        "claude" => scanner.scan_claude(),
        "opencodego" => scanner.scan_opencodego_with_cancel(None),
        _ => CostSummary::default(),
    };
    build_local_spend_contract_from_summary(
        provider_id,
        history_days,
        include_opencodex,
        false,
        crate::settings::Settings::load().hide_personal_info,
        summary,
    )
}

/// Build the accounting contract from an already-computed summary so callers do not rescan logs.
pub fn build_local_spend_contract_from_summary(
    provider_id: &str,
    history_days: u32,
    include_opencodex: bool,
    hide_native_codex_when_opencodex_present: bool,
    hide_personal_info: bool,
    summary: CostSummary,
) -> SpendContract {
    let history_days = history_days.clamp(1, 365);
    let custom = CustomPricing::load();
    let native_models = model_rows(provider_id, &summary, &custom);
    let native_coverage = coverage_for_models(&native_models);
    let native_cost = known_subtotal(&native_models, &summary);
    let native_has_window_costs = native_models.iter().any(|model| model.cost_usd.is_some());
    let native_provenance = CostProvenance::for_window(
        CostProvenance::ListPriceEstimate,
        native_has_window_costs,
        false,
    );
    let native_token_mix = SpendTokenMix {
        input_tokens: Some(summary.input_tokens),
        output_tokens: Some(summary.output_tokens),
        cache_read_tokens: Some(summary.cached_tokens),
        cache_creation_tokens: None,
        reasoning_tokens: summary.reasoning_tokens,
        ..SpendTokenMix::default()
    };

    let native = load_native_spend(provider_id, history_days, hide_personal_info);
    let imports: Vec<_> = if include_opencodex {
        opencodex::load_for_subscription(provider_id, history_days, &custom)
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let imported = imports.first();
    let replace_native =
        provider_id == "codex" && hide_native_codex_when_opencodex_present && imported.is_some();
    let resolved = resolve_spend(
        native_cost,
        native_provenance,
        native_has_window_costs,
        native_coverage,
        native_token_mix,
        native_models,
        native.daily.clone(),
        native.activity.clone(),
        imported,
        replace_native,
    );

    // Conversation count is clamped to u32::MAX before casting.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to u32::MAX before casting"
    )]
    let native_conversations = if native.conversations.is_empty() {
        summary.sessions_count
    } else {
        native.conversations.len().min(u32::MAX as usize) as u32
    };
    let imported_conversations = imported.map_or(0, |source| source.conversation_count);
    let conversation_count = if replace_native {
        imported_conversations
    } else {
        native_conversations.saturating_add(imported_conversations)
    };
    let known_zero = if replace_native {
        imported.is_some_and(|source| {
            source.known_cost_usd == Some(0.0) && source.coverage.unpriced == 0
        })
    } else {
        summary.known_zero && imports.is_empty()
    };

    SpendContract {
        provider_id: provider_id.to_string(),
        history_days,
        known_cost_usd: resolved.known_cost_usd,
        known_zero,
        provenance: resolved.provenance,
        price_coverage_ratio: if resolved.price_coverage_exact {
            resolved.price_coverage.coverage_ratio()
        } else {
            None
        },
        price_coverage: resolved.price_coverage,
        history_coverage_established: summary.history_coverage_established,
        token_mix: resolved.token_mix,
        conversation_count,
        models: resolved.models,
        projects: native.projects,
        conversations: native.conversations,
        daily: resolved.daily,
        hourly_activity: resolved.hourly_activity,
        project_source_status: native.project_source_status,
        custom_pricing_active: !custom.entries.is_empty(),
        imports,
    }
}

fn load_native_spend(
    provider_id: &str,
    history_days: u32,
    hide_personal_info: bool,
) -> NativeSpendData {
    if provider_id != "codex" {
        return NativeSpendData {
            projects: Vec::new(),
            conversations: Vec::new(),
            project_source_status: None,
            activity: Vec::new(),
            daily: daily_points(provider_id, history_days),
        };
    }
    match CodexWorkspacesIndex::new(history_days).load_snapshot(false, |_| {}) {
        Ok(mut snapshot) => {
            if hide_personal_info {
                snapshot.redact_for_privacy();
            }
            let activity = activity_from_sessions(&snapshot.sessions);
            let daily = snapshot
                .daily
                .iter()
                .map(|point| SpendDailyPoint {
                    day: point.day.clone(),
                    cost_usd: point.estimated_cost_usd,
                    total_tokens: Some(point.total_tokens),
                })
                .collect();
            NativeSpendData {
                projects: snapshot.projects,
                conversations: snapshot.sessions,
                project_source_status: Some(snapshot.source_status),
                activity,
                daily,
            }
        }
        Err(_) => NativeSpendData {
            projects: Vec::new(),
            conversations: Vec::new(),
            project_source_status: None,
            activity: Vec::new(),
            daily: daily_points(provider_id, history_days),
        },
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "signature mirrors the flat spend-contract config fields one-to-one"
)]
fn resolve_spend(
    native_cost: Option<f64>,
    native_provenance: CostProvenance,
    native_has_window_costs: bool,
    native_coverage: CostCoverageCounts,
    native_token_mix: SpendTokenMix,
    native_models: Vec<SpendModelRow>,
    native_daily: Vec<SpendDailyPoint>,
    native_activity: Vec<SpendActivityCell>,
    imported: Option<&ImportedSpendSource>,
    replace_native: bool,
) -> ResolvedSpendData {
    match imported {
        Some(imported) if replace_native => ResolvedSpendData {
            known_cost_usd: imported.known_cost_usd,
            provenance: imported.provenance,
            price_coverage: imported.coverage.clone(),
            price_coverage_exact: imported.coverage.checked_total().is_some(),
            token_mix: imported.token_mix.clone(),
            models: imported.models.clone(),
            daily: imported.daily.clone(),
            hourly_activity: imported.hourly_activity.clone(),
        },
        Some(imported) => {
            let (price_coverage, price_coverage_exact) =
                merge_coverage(native_coverage, &imported.coverage);
            ResolvedSpendData {
                known_cost_usd: sum_optional_cost(native_cost, imported.known_cost_usd),
                provenance: merge_provenance(
                    native_provenance,
                    native_has_window_costs,
                    imported.provenance,
                    imported.known_cost_usd.is_some(),
                ),
                price_coverage,
                price_coverage_exact,
                token_mix: merge_token_mix(native_token_mix, &imported.token_mix),
                models: merge_models(native_models, &imported.models),
                daily: merge_daily(native_daily, &imported.daily),
                hourly_activity: merge_activity(native_activity, &imported.hourly_activity),
            }
        }
        None => ResolvedSpendData {
            known_cost_usd: native_cost,
            provenance: native_provenance,
            price_coverage_exact: native_coverage.checked_total().is_some(),
            price_coverage: native_coverage,
            token_mix: native_token_mix,
            models: native_models,
            daily: native_daily,
            hourly_activity: native_activity,
        },
    }
}

fn merge_provenance(
    left: CostProvenance,
    left_has_window_costs: bool,
    right: CostProvenance,
    right_has_window_costs: bool,
) -> CostProvenance {
    let mut merged = None;
    for (provenance, has_window_costs) in [
        (left, left_has_window_costs),
        (right, right_has_window_costs),
    ] {
        if !has_window_costs {
            continue;
        }
        merged = Some(match merged {
            None => provenance,
            Some(existing) => combine_provenance(existing, provenance),
        });
    }
    merged.unwrap_or(CostProvenance::Unknown)
}

fn combine_provenance(left: CostProvenance, right: CostProvenance) -> CostProvenance {
    match (left, right) {
        (CostProvenance::Unknown, _) | (_, CostProvenance::Unknown) => CostProvenance::Unknown,
        (CostProvenance::Mixed, _) | (_, CostProvenance::Mixed) => CostProvenance::Mixed,
        (CostProvenance::ListPriceEstimate, CostProvenance::ListPriceEstimate) => {
            CostProvenance::ListPriceEstimate
        }
        (CostProvenance::VendorMetered, CostProvenance::VendorMetered) => {
            CostProvenance::VendorMetered
        }
        (CostProvenance::ListPriceEstimate, CostProvenance::VendorMetered)
        | (CostProvenance::VendorMetered, CostProvenance::ListPriceEstimate) => {
            CostProvenance::Mixed
        }
    }
}

fn model_rows(
    provider_id: &str,
    summary: &CostSummary,
    custom: &CustomPricing,
) -> Vec<SpendModelRow> {
    let mut names: HashSet<String> = summary.by_model.keys().cloned().collect();
    names.extend(summary.by_model_tokens.keys().cloned());
    names.extend(summary.unknown_models.iter().cloned());
    let mut rows: Vec<_> = names
        .into_iter()
        .map(|model| {
            let counts = summary
                .by_model_tokens
                .get(&model)
                .cloned()
                .unwrap_or_default();
            let custom_rates = custom.rates(provider_id, &model);
            // Exact-match overlay is authoritative when present. Missing fields
            // remain unknown rather than falling back to built-in/model.dev rates.
            let cost_usd = if let Some(rates) = custom_rates {
                rates.cost(&counts)
            } else if summary.unknown_models.contains(&model) {
                None
            } else {
                summary
                    .by_model
                    .get(&model)
                    .copied()
                    .filter(|value| value.is_finite() && *value >= 0.0)
            };
            SpendModelRow {
                model,
                cost_usd,
                input_tokens: counts.input_tokens,
                output_tokens: counts.output_tokens,
                cache_read_tokens: counts.cached_tokens,
                total_tokens: counts.total(),
                custom_pricing: custom_rates.is_some(),
            }
        })
        .collect();
    rows.sort_by(|left, right| match (left.cost_usd, right.cost_usd) {
        (Some(a), Some(b)) => b
            .partial_cmp(&a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.model.cmp(&right.model)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.model.cmp(&right.model),
    });
    rows
}

fn coverage_for_models(models: &[SpendModelRow]) -> CostCoverageCounts {
    let mut coverage = CostCoverageCounts::default();
    for model in models {
        if model.cost_usd.is_some() {
            coverage.estimated = coverage.estimated.saturating_add(1);
        } else {
            coverage.unpriced = coverage.unpriced.saturating_add(1);
        }
    }
    coverage
}

fn known_subtotal(models: &[SpendModelRow], summary: &CostSummary) -> Option<f64> {
    if models.is_empty() {
        return summary.known_zero.then_some(0.0);
    }
    let mut total = 0.0;
    let mut saw_known = false;
    for model in models {
        if let Some(cost) = model.cost_usd {
            total += cost;
            saw_known = true;
        }
    }
    (saw_known && total.is_finite()).then_some(total)
}

fn daily_points(provider_id: &str, days: u32) -> Vec<SpendDailyPoint> {
    let costs: HashMap<String, Option<f64>> = get_daily_cost_history(provider_id, days)
        .into_iter()
        .collect();
    let (tokens, incomplete) = get_daily_token_history(provider_id, days);
    tokens
        .into_iter()
        .map(|(day, total_tokens)| SpendDailyPoint {
            cost_usd: costs.get(&day).copied().flatten().filter(|_| !incomplete),
            day,
            total_tokens: (!incomplete).then_some(total_tokens),
        })
        .collect()
}

fn activity_from_sessions(sessions: &[SessionUsage]) -> Vec<SpendActivityCell> {
    let mut cells: BTreeMap<(u8, u8), u32> = BTreeMap::new();
    for session in sessions {
        let Some(timestamp) = session.latest_activity.or(session.started_at) else {
            continue;
        };
        let local = timestamp.with_timezone(&Local);
        // Weekday (0-6) and hour (0-23) both fit u8.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "weekday (0-6) and hour (0-23) fit u8"
        )]
        let key = (
            local.weekday().num_days_from_monday() as u8,
            local.hour() as u8,
        );
        let next = cells.get(&key).copied().unwrap_or(0).saturating_add(1);
        cells.insert(key, next);
    }
    cells
        .into_iter()
        .map(|((weekday, hour), conversations)| SpendActivityCell {
            weekday,
            hour,
            conversations,
        })
        .collect()
}
fn sum_optional_cost(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    let valid = |value: f64| value.is_finite() && value >= 0.0;
    match (left, right) {
        (Some(left), Some(right)) if valid(left) && valid(right) => {
            let total = left + right;
            valid(total).then_some(total)
        }
        (Some(value), None) | (None, Some(value)) if valid(value) => Some(value),
        (None, None) => None,
        _ => None,
    }
}

fn merge_coverage(
    left: CostCoverageCounts,
    right: &CostCoverageCounts,
) -> (CostCoverageCounts, bool) {
    let priced = left.priced.checked_add(right.priced);
    let unpriced = left.unpriced.checked_add(right.unpriced);
    let unmetered = left.unmetered.checked_add(right.unmetered);
    let estimated = left.estimated.checked_add(right.estimated);
    let exact =
        priced.is_some() && unpriced.is_some() && unmetered.is_some() && estimated.is_some();
    let merged = CostCoverageCounts {
        priced: priced.unwrap_or(u32::MAX),
        unpriced: unpriced.unwrap_or(u32::MAX),
        unmetered: unmetered.unwrap_or(u32::MAX),
        estimated: estimated.unwrap_or(u32::MAX),
    };
    let exact = exact && merged.checked_total().is_some();
    (merged, exact)
}

fn merge_token_mix(mut left: SpendTokenMix, right: &SpendTokenMix) -> SpendTokenMix {
    left.overflowed_classes |= right.overflowed_classes;
    left.input_tokens = merge_token_class(
        left.input_tokens,
        right.input_tokens,
        &mut left.overflowed_classes,
        1 << 0,
    );
    left.output_tokens = merge_token_class(
        left.output_tokens,
        right.output_tokens,
        &mut left.overflowed_classes,
        1 << 1,
    );
    left.cache_read_tokens = merge_token_class(
        left.cache_read_tokens,
        right.cache_read_tokens,
        &mut left.overflowed_classes,
        1 << 2,
    );
    left.cache_creation_tokens = merge_token_class(
        left.cache_creation_tokens,
        right.cache_creation_tokens,
        &mut left.overflowed_classes,
        1 << 3,
    );
    left.reasoning_tokens = merge_token_class(
        left.reasoning_tokens,
        right.reasoning_tokens,
        &mut left.overflowed_classes,
        1 << 4,
    );
    left
}

fn merge_token_class(
    left: Option<u64>,
    right: Option<u64>,
    overflowed_classes: &mut u8,
    bit: u8,
) -> Option<u64> {
    if *overflowed_classes & bit != 0 {
        return None;
    }
    match (left, right) {
        (Some(left), Some(right)) => match left.checked_add(right) {
            Some(total) => Some(total),
            None => {
                *overflowed_classes |= bit;
                None
            }
        },
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn add_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn merge_models(mut left: Vec<SpendModelRow>, right: &[SpendModelRow]) -> Vec<SpendModelRow> {
    for incoming in right {
        if let Some(existing) = left.iter_mut().find(|row| row.model == incoming.model) {
            existing.cost_usd = sum_optional_cost(existing.cost_usd, incoming.cost_usd);
            existing.input_tokens = existing.input_tokens.saturating_add(incoming.input_tokens);
            existing.output_tokens = existing
                .output_tokens
                .saturating_add(incoming.output_tokens);
            existing.cache_read_tokens = existing
                .cache_read_tokens
                .saturating_add(incoming.cache_read_tokens);
            existing.total_tokens = existing.total_tokens.saturating_add(incoming.total_tokens);
            existing.custom_pricing |= incoming.custom_pricing;
        } else {
            left.push(incoming.clone());
        }
    }
    left.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.model.cmp(&b.model))
    });
    left
}

fn merge_daily(left: Vec<SpendDailyPoint>, right: &[SpendDailyPoint]) -> Vec<SpendDailyPoint> {
    let mut days: BTreeMap<String, SpendDailyPoint> = left
        .into_iter()
        .map(|point| (point.day.clone(), point))
        .collect();
    for incoming in right {
        let entry = days
            .entry(incoming.day.clone())
            .or_insert_with(|| SpendDailyPoint {
                day: incoming.day.clone(),
                cost_usd: None,
                total_tokens: None,
            });
        entry.cost_usd = sum_optional_cost(entry.cost_usd, incoming.cost_usd);
        entry.total_tokens = add_optional(entry.total_tokens, incoming.total_tokens);
    }
    days.into_values().collect()
}

fn merge_activity(
    left: Vec<SpendActivityCell>,
    right: &[SpendActivityCell],
) -> Vec<SpendActivityCell> {
    let mut cells: BTreeMap<(u8, u8), u32> = left
        .into_iter()
        .map(|cell| ((cell.weekday, cell.hour), cell.conversations))
        .collect();
    for cell in right {
        let current = cells.get(&(cell.weekday, cell.hour)).copied().unwrap_or(0);
        cells.insert(
            (cell.weekday, cell.hour),
            current.saturating_add(cell.conversations),
        );
    }
    cells
        .into_iter()
        .map(|((weekday, hour), conversations)| SpendActivityCell {
            weekday,
            hour,
            conversations,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_ratio_counts_estimated_as_covered() {
        let coverage = CostCoverageCounts {
            priced: 1,
            unpriced: 1,
            unmetered: 0,
            estimated: 2,
        };
        assert_eq!(coverage.coverage_ratio(), Some(0.75));
    }

    #[test]
    fn coverage_overflow_is_unknown_instead_of_a_fake_ratio() {
        let coverage = CostCoverageCounts {
            priced: u32::MAX,
            unpriced: 1,
            unmetered: 0,
            estimated: 0,
        };
        assert_eq!(coverage.total(), u32::MAX);
        assert_eq!(coverage.checked_total(), None);
        assert_eq!(coverage.coverage_ratio(), None);

        let (merged, exact) = merge_coverage(
            CostCoverageCounts {
                priced: u32::MAX,
                ..CostCoverageCounts::default()
            },
            &CostCoverageCounts {
                priced: 1,
                ..CostCoverageCounts::default()
            },
        );
        assert!(!exact);
        assert_eq!(merged.priced, u32::MAX);
    }

    #[test]
    fn explicit_zero_custom_rate_is_known_free_but_missing_rate_is_unknown() {
        let counts = ModelTokenCounts {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cached_tokens: 0,
            reasoning_tokens: None,
        };
        let free = CustomRates {
            input: Some(0.0),
            ..CustomRates::default()
        };
        let missing = CustomRates::default();
        assert_eq!(free.cost(&counts), Some(0.0));
        assert_eq!(missing.cost(&counts), None);
    }

    #[test]
    fn local_spend_contract_exposes_reasoning_tokens_and_preserves_unknown() {
        let mut known_summary = CostSummary::default();
        known_summary.reasoning_tokens = Some(7);
        let known = build_local_spend_contract_from_summary(
            "unknown-provider",
            30,
            false,
            false,
            false,
            known_summary,
        );
        assert_eq!(known.token_mix.reasoning_tokens, Some(7));
        let known_json = serde_json::to_value(&known).expect("known spend contract serializes");
        assert_eq!(known_json["tokenMix"]["reasoningTokens"], 7);

        let unknown = build_local_spend_contract_from_summary(
            "unknown-provider",
            30,
            false,
            false,
            false,
            CostSummary::default(),
        );
        assert_eq!(unknown.token_mix.reasoning_tokens, None);
        let unknown_json =
            serde_json::to_value(&unknown).expect("unknown spend contract serializes");
        assert!(unknown_json["tokenMix"]["reasoningTokens"].is_null());
    }

    #[test]
    fn sum_optional_cost_propagates_unknown_and_rejects_non_finite() {
        assert_eq!(sum_optional_cost(Some(1.5), Some(2.25)), Some(3.75));
        assert_eq!(sum_optional_cost(Some(1.5), None), Some(1.5));
        assert_eq!(sum_optional_cost(None, Some(2.0)), Some(2.0));
        assert_eq!(sum_optional_cost(None, None), None);
        assert_eq!(sum_optional_cost(Some(f64::INFINITY), Some(1.0)), None);
        assert_eq!(sum_optional_cost(Some(-1.0), None), None);
        assert_eq!(sum_optional_cost(Some(1.0), Some(f64::NAN)), None);
    }

    #[test]
    fn add_optional_token_counts_guard_against_overflow() {
        assert_eq!(add_optional(Some(2), Some(3)), Some(5));
        assert_eq!(add_optional(Some(2), None), Some(2));
        assert_eq!(add_optional(None, None), None);
        assert_eq!(add_optional(Some(u64::MAX), Some(1)), None);
    }

    #[test]
    fn merge_token_mix_preserves_optional_reasoning_and_unknown_classes() {
        let merged = merge_token_mix(
            SpendTokenMix {
                input_tokens: None,
                reasoning_tokens: Some(2),
                ..SpendTokenMix::default()
            },
            &SpendTokenMix {
                input_tokens: Some(5),
                reasoning_tokens: Some(3),
                ..SpendTokenMix::default()
            },
        );
        assert_eq!(merged.input_tokens, Some(5));
        assert_eq!(merged.reasoning_tokens, Some(5));
        assert_eq!(merged.output_tokens, None);
    }

    #[test]
    fn merge_token_mix_keeps_overflow_unknown_across_later_sources() {
        let overflowed = merge_token_mix(
            SpendTokenMix {
                input_tokens: Some(u64::MAX),
                ..SpendTokenMix::default()
            },
            &SpendTokenMix {
                input_tokens: Some(1),
                ..SpendTokenMix::default()
            },
        );
        assert_eq!(overflowed.input_tokens, None);

        let merged = merge_token_mix(
            overflowed,
            &SpendTokenMix {
                input_tokens: Some(2),
                ..SpendTokenMix::default()
            },
        );
        assert_eq!(merged.input_tokens, None);
    }

    #[test]
    fn merge_models_combines_duplicate_models_and_sorts_priced_first() {
        let make_row = |model: &str, cost: Option<f64>, input: u64, custom: bool| SpendModelRow {
            model: model.to_string(),
            cost_usd: cost,
            input_tokens: input,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: input,
            custom_pricing: custom,
        };
        let merged = merge_models(
            vec![
                make_row("beta", Some(1.0), 10, false),
                make_row("alpha", None, 5, false),
                make_row("zzz", Some(1.0), 1, false),
                make_row("aaa", Some(1.0), 1, false),
            ],
            &[make_row("beta", Some(2.0), 7, true)],
        );
        let names: Vec<&str> = merged.iter().map(|row| row.model.as_str()).collect();
        assert_eq!(
            names,
            ["beta", "aaa", "zzz", "alpha"],
            "cost desc, then name asc for ties, unknown cost last"
        );
        let beta = &merged[0];
        assert_eq!(beta.cost_usd, Some(3.0));
        assert_eq!(beta.input_tokens, 17);
        assert_eq!(beta.total_tokens, 17);
        assert!(beta.custom_pricing, "custom pricing flags are OR-ed");
    }

    #[test]
    fn merge_daily_sums_matching_days_and_keeps_iso_day_ordering() {
        let make_point = |day: &str, cost: Option<f64>, tokens: Option<u64>| SpendDailyPoint {
            day: day.to_string(),
            cost_usd: cost,
            total_tokens: tokens,
        };
        let merged = merge_daily(
            vec![
                make_point("2026-08-02", Some(1.0), Some(10)),
                make_point("2026-08-01", None, None),
            ],
            &[
                make_point("2026-08-02", Some(2.5), Some(15)),
                make_point("2026-08-03", Some(4.0), None),
            ],
        );
        let days: Vec<&str> = merged.iter().map(|point| point.day.as_str()).collect();
        assert_eq!(days, ["2026-08-01", "2026-08-02", "2026-08-03"]);
        assert_eq!(merged[1].cost_usd, Some(3.5));
        assert_eq!(merged[1].total_tokens, Some(25));
        assert_eq!(merged[0].cost_usd, None, "unknown stays unknown");
        assert_eq!(merged[0].total_tokens, None);
        assert_eq!(merged[2].total_tokens, None);
    }

    #[test]
    fn resolve_spend_preserves_merged_report_details() {
        let native_models = vec![SpendModelRow {
            model: "gpt-5".to_string(),
            cost_usd: Some(1.0),
            input_tokens: 10,
            output_tokens: 2,
            cache_read_tokens: 0,
            total_tokens: 12,
            custom_pricing: false,
        }];
        let imported = ImportedSpendSource {
            source_id: "fixture".to_string(),
            display_name: "Fixture".to_string(),
            request_count: 2,
            conversation_count: 1,
            known_cost_usd: Some(2.0),
            provenance: CostProvenance::VendorMetered,
            token_mix: SpendTokenMix {
                input_tokens: Some(5),
                cache_read_tokens: Some(4),
                reasoning_tokens: Some(1),
                ..SpendTokenMix::default()
            },
            coverage: CostCoverageCounts {
                priced: 2,
                unpriced: 1,
                unmetered: 0,
                estimated: 1,
            },
            models: vec![SpendModelRow {
                model: "gpt-5".to_string(),
                cost_usd: Some(2.0),
                input_tokens: 5,
                output_tokens: 0,
                cache_read_tokens: 4,
                total_tokens: 9,
                custom_pricing: true,
            }],
            daily: vec![
                SpendDailyPoint {
                    day: "2026-08-01".to_string(),
                    cost_usd: Some(2.0),
                    total_tokens: Some(9),
                },
                SpendDailyPoint {
                    day: "2026-08-02".to_string(),
                    cost_usd: None,
                    total_tokens: None,
                },
            ],
            hourly_activity: vec![SpendActivityCell {
                weekday: 1,
                hour: 2,
                conversations: 4,
            }],
        };

        let resolved = resolve_spend(
            Some(1.0),
            CostProvenance::ListPriceEstimate,
            true,
            CostCoverageCounts {
                priced: 1,
                unpriced: 0,
                unmetered: 1,
                estimated: 0,
            },
            SpendTokenMix {
                input_tokens: Some(10),
                output_tokens: Some(2),
                ..SpendTokenMix::default()
            },
            native_models,
            vec![SpendDailyPoint {
                day: "2026-08-01".to_string(),
                cost_usd: Some(1.0),
                total_tokens: Some(12),
            }],
            vec![SpendActivityCell {
                weekday: 1,
                hour: 2,
                conversations: 3,
            }],
            Some(&imported),
            false,
        );

        assert_eq!(resolved.known_cost_usd, Some(3.0));
        assert_eq!(resolved.provenance, CostProvenance::Mixed);
        assert_eq!(resolved.token_mix.input_tokens, Some(15));
        assert_eq!(resolved.token_mix.output_tokens, Some(2));
        assert_eq!(resolved.token_mix.cache_read_tokens, Some(4));
        assert_eq!(resolved.token_mix.reasoning_tokens, Some(1));
        assert_eq!(resolved.price_coverage.priced, 3);
        assert_eq!(resolved.price_coverage.unpriced, 1);
        assert_eq!(resolved.price_coverage.unmetered, 1);
        assert_eq!(resolved.price_coverage.estimated, 1);
        assert!(resolved.price_coverage_exact);

        let model = &resolved.models[0];
        assert_eq!(model.cost_usd, Some(3.0));
        assert_eq!(model.input_tokens, 15);
        assert_eq!(model.cache_read_tokens, 4);
        assert!(model.custom_pricing);
        assert_eq!(resolved.daily[0].cost_usd, Some(3.0));
        assert_eq!(resolved.daily[0].total_tokens, Some(21));
        assert_eq!(resolved.daily[1].cost_usd, None);
        assert_eq!(resolved.daily[1].total_tokens, None);
        assert_eq!(resolved.hourly_activity[0].conversations, 7);
    }

    #[test]
    fn cost_provenance_for_window_matches_upstream_truth_table() {
        let cases = [
            (
                CostProvenance::ListPriceEstimate,
                false,
                false,
                CostProvenance::Unknown,
            ),
            (
                CostProvenance::ListPriceEstimate,
                true,
                false,
                CostProvenance::ListPriceEstimate,
            ),
            (
                CostProvenance::VendorMetered,
                false,
                false,
                CostProvenance::Unknown,
            ),
            (
                CostProvenance::VendorMetered,
                true,
                false,
                CostProvenance::VendorMetered,
            ),
            (
                CostProvenance::VendorMetered,
                false,
                true,
                CostProvenance::VendorMetered,
            ),
            (CostProvenance::Mixed, false, false, CostProvenance::Unknown),
            (
                CostProvenance::Mixed,
                true,
                false,
                CostProvenance::ListPriceEstimate,
            ),
            (
                CostProvenance::Mixed,
                false,
                true,
                CostProvenance::VendorMetered,
            ),
            (CostProvenance::Mixed, true, true, CostProvenance::Mixed),
            (CostProvenance::Unknown, true, true, CostProvenance::Unknown),
        ];

        for (snapshot, has_window_costs, includes_metered, expected) in cases {
            assert_eq!(
                CostProvenance::for_window(snapshot, has_window_costs, includes_metered),
                expected,
                "snapshot={snapshot:?}, has_window_costs={has_window_costs}, includes_metered={includes_metered}"
            );
        }
    }

    #[test]
    fn zero_cost_authoritative_sources_use_presence_not_positive_value() {
        assert_eq!(
            CostProvenance::for_window(CostProvenance::ListPriceEstimate, true, false),
            CostProvenance::ListPriceEstimate
        );
        assert_eq!(
            CostProvenance::for_window(CostProvenance::VendorMetered, true, false),
            CostProvenance::VendorMetered
        );

        let resolved = resolve_spend(
            Some(0.0),
            CostProvenance::ListPriceEstimate,
            true,
            CostCoverageCounts::default(),
            SpendTokenMix::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            false,
        );
        assert_eq!(resolved.known_cost_usd, Some(0.0));
        assert_eq!(resolved.provenance, CostProvenance::ListPriceEstimate);
    }

    #[test]
    fn provenance_merge_keeps_unknown_conservative_and_mixes_vendor_with_list() {
        assert_eq!(
            merge_provenance(
                CostProvenance::ListPriceEstimate,
                true,
                CostProvenance::VendorMetered,
                true,
            ),
            CostProvenance::Mixed
        );
        assert_eq!(
            merge_provenance(
                CostProvenance::ListPriceEstimate,
                true,
                CostProvenance::ListPriceEstimate,
                true,
            ),
            CostProvenance::ListPriceEstimate
        );
        assert_eq!(
            merge_provenance(
                CostProvenance::VendorMetered,
                true,
                CostProvenance::VendorMetered,
                true,
            ),
            CostProvenance::VendorMetered
        );
        assert_eq!(
            merge_provenance(
                CostProvenance::Unknown,
                true,
                CostProvenance::ListPriceEstimate,
                true,
            ),
            CostProvenance::Unknown
        );
        assert_eq!(
            merge_provenance(
                CostProvenance::Unknown,
                false,
                CostProvenance::ListPriceEstimate,
                true,
            ),
            CostProvenance::ListPriceEstimate
        );
    }

    #[test]
    fn spend_contract_serializes_provenance_for_tauri_and_cli() {
        let contract = SpendContract {
            provider_id: "codex".to_string(),
            history_days: 30,
            known_cost_usd: Some(0.0),
            known_zero: false,
            provenance: CostProvenance::VendorMetered,
            price_coverage: CostCoverageCounts::default(),
            price_coverage_ratio: None,
            history_coverage_established: true,
            token_mix: SpendTokenMix::default(),
            conversation_count: 0,
            models: Vec::new(),
            projects: Vec::new(),
            conversations: Vec::new(),
            daily: Vec::new(),
            hourly_activity: Vec::new(),
            project_source_status: None,
            custom_pricing_active: false,
            imports: Vec::new(),
        };

        let json = serde_json::to_value(contract).expect("spend contract serializes");
        assert_eq!(json["provenance"], "vendorMetered");
    }

    #[test]
    fn known_subtotal_sums_known_costs_only_and_needs_known_zero_for_empty() {
        let make_row = |cost: Option<f64>| SpendModelRow {
            model: String::new(),
            cost_usd: cost,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            custom_pricing: false,
        };
        let mut summary = CostSummary::default();
        assert_eq!(known_subtotal(&[], &summary), None);
        summary.known_zero = true;
        assert_eq!(known_subtotal(&[], &summary), Some(0.0));
        summary.known_zero = false;
        let mixed = [make_row(Some(1.5)), make_row(None), make_row(Some(2.25))];
        assert_eq!(known_subtotal(&mixed, &summary), Some(3.75));
        let all_unknown = [make_row(None)];
        assert_eq!(known_subtotal(&all_unknown, &summary), None);
    }

    #[test]
    fn coverage_for_models_counts_priced_rows_as_estimated() {
        let make_row = |cost: Option<f64>| SpendModelRow {
            model: String::new(),
            cost_usd: cost,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            custom_pricing: false,
        };
        let coverage =
            coverage_for_models(&[make_row(Some(0.0)), make_row(None), make_row(Some(3.0))]);
        assert_eq!(coverage.estimated, 2);
        assert_eq!(coverage.unpriced, 1);
        assert_eq!(coverage.total(), 3);
    }
}
