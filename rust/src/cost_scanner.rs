//! Local cost-usage scanner for Codex and Claude
//!
//! Scans local JSONL log files to aggregate token usage and calculate costs.
//!
//! Codex production path loads/saves [`crate::core::CostUsageCache`] under
//! `{cache}/CodexBar/cost-usage/`, skips unchanged files by mtime+size, resumes
//! partial files from `parsed_bytes`, honors [`crate::core::CostScanOptions`]
//! debounce (default 60s; `app_driven` forces a fresh inspection), and checks
//! cancel flags between files.

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use crate::codex_costs::scan_codex_file_cost;
use crate::codex_costs::{
    add_codex_days_map_to_summary, add_codex_records_to_summary, codex_period_start,
    codex_scan_dates, merge_codex_records_into_days,
};
use crate::codex_sessions::{codex_sessions_dir_candidates, default_wsl_roots};
use crate::core::{
    CachedCostReport, CostScanOptions, CostUsageCache, CostUsageDayRange, CostUsageFileUsage,
    CostUsagePricing, JsonlScanner, ProviderId,
};
use crate::providers::opencodego::local as opencodego_local;
use crate::settings::Settings;

/// Completeness of the pricing coverage in a [`CostSummary`] (upstream 0.48.0 F18).
///
/// `Complete` means every billed model resolved a canonical or fast-rate price.
/// `Partial` means at least one model was deliberately unpriced (routing rows like
/// `codex-auto-review`) or fell back to a legacy default; the breakdown is still
/// shown but the total is labeled partial.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ModelPricingCompleteness {
    /// Every model resolved a canonical price.
    #[default]
    Complete,
    /// At least one model was unpriced or used a fallback rate.
    Partial {
        /// Model IDs that were deliberately unpriced (routing rows).
        unpriced_models: Vec<String>,
    },
}

impl ModelPricingCompleteness {
    pub fn is_partial(&self) -> bool {
        matches!(self, Self::Partial { .. })
    }
}

/// Cost summary from scanning local logs
#[derive(Debug, Clone, Default)]
pub struct CostSummary {
    /// Total cost in USD for the period
    pub total_cost_usd: f64,
    /// Total input tokens
    pub input_tokens: u64,
    /// Total output tokens
    pub output_tokens: u64,
    /// Total cached input tokens
    pub cached_tokens: u64,
    /// Total reasoning tokens when every contributing row reports them
    pub reasoning_tokens: Option<u64>,
    /// Number of sessions/conversations scanned
    pub sessions_count: u32,
    /// Cost breakdown by model
    pub by_model: HashMap<String, f64>,
    /// Token breakdown by model
    pub by_model_tokens: HashMap<String, ModelTokenCounts>,
    /// Codex cost split by speed/tier when local logs expose it.
    pub by_speed: HashMap<String, f64>,
    /// Codex token split by speed/tier when local logs expose it.
    pub by_speed_tokens: HashMap<String, ModelTokenCounts>,
    /// Model IDs that were priced with fallback rates because no canonical rate is available.
    pub unknown_models: HashSet<String>,
    /// Completeness of pricing coverage (Complete vs Partial). Surfaced in the CLI
    /// cost JSON so callers can label a partial breakdown (upstream 0.48.0 F18).
    pub model_pricing_completeness: ModelPricingCompleteness,
    /// Whether the scan's coverage of the requested history window is established
    /// (not pending a catch-up re-scan). `true` when the cache is fresh (within the
    /// debounce window) or the scan just completed; `false` when the cache is stale
    /// or empty and a re-scan would be required (upstream 0.48.0 A16).
    pub history_coverage_established: bool,
    /// True when the scan completed with zero results — a *known* zero, not a
    /// missing scan. Set only when `history_coverage_established` is true and
    /// the scan found no sessions/tokens (upstream 0.50.1 #2932). Never
    /// fabricated on incomplete scans.
    pub known_zero: bool,
    /// Period start date
    pub period_start: Option<NaiveDate>,
    /// Period end date
    pub period_end: Option<NaiveDate>,
}

/// Per-model token counts
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelTokenCounts {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: Option<u64>,
}

impl ModelTokenCounts {
    pub fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

impl CostSummary {
    pub fn format_total(&self) -> String {
        format!("${:.2}", self.total_cost_usd)
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

/// Fallback Claude model used when a scanned model isn't in the canonical
/// pricing table (unknown or retired IDs). Prices as Sonnet 4.6.
const FALLBACK_CLAUDE_MODEL: &str = "claude-sonnet-4-6";

fn unix_now_ms() -> i64 {
    // Duration is clamped to i64::MAX before casting, so the value fits i64.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to i64::MAX before casting"
    )]
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    millis
}

fn system_time_to_unix_ms(modified: Option<SystemTime>) -> i64 {
    // Duration is clamped to i64::MAX before casting, so the value fits i64.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to i64::MAX before casting"
    )]
    let millis = modified
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    millis
}

fn rebuild_cache_days(cache: &mut CostUsageCache) {
    cache.days.clear();
    for usage in cache.files.values() {
        for (day, models) in &usage.days {
            let day_entry = cache.days.entry(day.clone()).or_default();
            for (model, packed) in models {
                let dest = day_entry
                    .entry(model.clone())
                    .or_insert_with(|| vec![0, 0, 0]);
                if dest.len() < 3 {
                    dest.resize(3, 0);
                }

                let had_core_tokens = dest[0] != 0 || dest[1] != 0 || dest[2] != 0;
                let source_input = packed.first().copied().unwrap_or(0);
                let source_cached = packed.get(1).copied().unwrap_or(0);
                let source_output = packed.get(2).copied().unwrap_or(0);
                let source_has_tokens =
                    source_input != 0 || source_cached != 0 || source_output != 0;
                let source_reasoning = packed
                    .get(3)
                    .copied()
                    .map(|reasoning| reasoning.max(0).min(source_output.max(0)));

                dest[0] = dest[0].saturating_add(source_input);
                dest[1] = dest[1].saturating_add(source_cached);
                dest[2] = dest[2].saturating_add(source_output);

                if !source_has_tokens {
                    continue;
                }

                if !had_core_tokens {
                    match source_reasoning {
                        Some(reasoning) => {
                            if dest.len() >= 4 {
                                dest[3] = reasoning.min(dest[2].max(0));
                            } else {
                                dest.push(reasoning.min(dest[2].max(0)));
                            }
                        }
                        None => dest.truncate(3),
                    }
                    continue;
                }

                match (dest.get(3).copied(), source_reasoning) {
                    (Some(previous), Some(reasoning)) => {
                        let merged = previous.saturating_add(reasoning).min(dest[2].max(0));
                        dest[3] = merged;
                    }
                    _ => dest.truncate(3),
                }
            }
        }
    }
}

fn summary_from_cached_report(
    report: &CachedCostReport,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> CostSummary {
    CostSummary {
        total_cost_usd: report.total_cost_usd,
        input_tokens: u64::try_from(report.input_tokens.max(0)).unwrap_or(0),
        cached_tokens: u64::try_from(report.cached_tokens.max(0)).unwrap_or(0),
        output_tokens: u64::try_from(report.output_tokens.max(0)).unwrap_or(0),
        reasoning_tokens: report
            .reasoning_tokens
            .map(|reasoning| u64::try_from(reasoning.max(0)).unwrap_or(0)),
        sessions_count: u32::try_from(report.sessions_count.max(0)).unwrap_or(0),
        // The persisted report has no model-level breakdown. A catch-up
        // summary must not claim that its newly rebuilt partial breakdown is
        // complete, even when the validated report itself was fully priced.
        model_pricing_completeness: ModelPricingCompleteness::Partial {
            unpriced_models: Vec::new(),
        },
        history_coverage_established: false,
        known_zero: false,
        period_start: Some(period_start),
        period_end: Some(period_end),
        ..CostSummary::default()
    }
}

/// Remove cached Codex files that are provably gone from the portion of the
/// sessions tree covered by this scan. Entries outside the current roots or
/// date directories are intentionally retained for a later scan.
fn reconcile_missing_codex_cache_files(
    cache: &mut CostUsageCache,
    sessions_dirs: &[PathBuf],
    range: &CostUsageDayRange,
) {
    let scanned_date_dirs: Vec<PathBuf> = sessions_dirs
        .iter()
        .flat_map(|sessions_dir| {
            codex_scan_dates(range).into_iter().map(|date| {
                sessions_dir
                    .join(date.format("%Y").to_string())
                    .join(date.format("%m").to_string())
                    .join(date.format("%d").to_string())
            })
        })
        .collect();

    cache.files.retain(|path_key, _| {
        let path = Path::new(path_key);
        let in_scanned_root = sessions_dirs
            .iter()
            .any(|sessions_dir| path.starts_with(sessions_dir));
        let in_scanned_date = scanned_date_dirs
            .iter()
            .any(|date_dir| path.starts_with(date_dir));

        !(in_scanned_root && in_scanned_date && !path.exists())
    });
    cache
        .codex_pending_paths
        .retain(|path| Path::new(path).exists());
}

fn cached_codex_file_is_complete_for_range(
    cache: &CostUsageCache,
    path_key: &str,
    range: &CostUsageDayRange,
) -> bool {
    JsonlScanner::cache_covers_range(cache, range)
        && cache.files.get(path_key).is_some_and(|usage| {
            let Ok(metadata) = fs::metadata(path_key) else {
                return false;
            };
            #[allow(clippy::cast_possible_wrap, reason = "session file sizes fit i64")]
            let size = metadata.len().min(i64::MAX as u64) as i64;
            usage.mtime_unix_ms == system_time_to_unix_ms(metadata.modified().ok())
                && usage.size == size
                && usage.parsed_bytes.unwrap_or(0) >= size
                && !usage.codex_unresolved_fork_parent
                && codex_fork_parent_is_safe(cache, usage)
        })
}

fn codex_fork_parent_is_safe(cache: &CostUsageCache, usage: &CostUsageFileUsage) -> bool {
    usage.codex_forked_from_id.as_deref().is_none()
        || codex_parent_baseline(
            cache,
            usage.codex_forked_from_id.as_deref().unwrap_or_default(),
            usage.codex_fork_timestamp.as_deref(),
        )
        .is_some()
}

/// Return a parent cumulative baseline only when exactly one cached session
/// identity is current, complete, timestamp-ordered, and safe to trust.
fn codex_parent_baseline(
    cache: &CostUsageCache,
    parent_session_id: &str,
    child_fork_timestamp: Option<&str>,
) -> Option<crate::core::CodexTotals> {
    let mut baseline = None;
    for (path_key, usage) in &cache.files {
        if usage.codex_session_id.as_deref() != Some(parent_session_id) {
            continue;
        }
        if usage.codex_unresolved_fork_parent
            || usage.codex_token_timestamps_monotonic != Some(true)
        {
            return None;
        }
        let metadata = fs::metadata(path_key).ok()?;
        #[allow(clippy::cast_possible_wrap, reason = "session file sizes fit i64")]
        let size = metadata.len().min(i64::MAX as u64) as i64;
        if usage.mtime_unix_ms != system_time_to_unix_ms(metadata.modified().ok())
            || usage.size != size
            || usage.parsed_bytes.unwrap_or(0) < size
        {
            return None;
        }
        let last_totals = usage.last_totals.clone()?;
        let last_token_timestamp = usage.codex_last_token_timestamp.as_deref()?;
        let child_fork_timestamp = child_fork_timestamp?;
        if !JsonlScanner::codex_timestamp_at_or_before(last_token_timestamp, child_fork_timestamp) {
            return None;
        }
        if baseline.replace(last_totals).is_some() {
            // Duplicate identities make the dependency ambiguous.
            return None;
        }
    }
    baseline
}

fn is_codex_path_in_scan_window(
    path: &Path,
    sessions_dirs: &[PathBuf],
    range: &CostUsageDayRange,
) -> bool {
    sessions_dirs.iter().any(|sessions_dir| {
        codex_scan_dates(range).into_iter().any(|date| {
            let date_dir = sessions_dir
                .join(date.format("%Y").to_string())
                .join(date.format("%m").to_string())
                .join(date.format("%d").to_string());
            path.starts_with(date_dir)
        })
    })
}

/// Claude cost calculation for the usage scanner.
///
/// Per-token rates come from the canonical `CostUsagePricing::claude_cost_usd`
/// table (the single source of truth for Claude pricing). The only
/// scanner-specific piece is the one-hour cache-write premium, which the
/// canonical cost function doesn't model: one-hour cache writes bill at 2x the
/// input rate.
struct ClaudePricing;

impl ClaudePricing {
    fn cost_usd_with_cache_ttl(
        model: &str,
        input: u64,
        cache_create: u64,
        cache_create_1h: u64,
        cache_read: u64,
        output: u64,
    ) -> f64 {
        let cache_create_1h = cache_create_1h.min(cache_create);
        let cache_create_5m = cache_create.saturating_sub(cache_create_1h);

        // Standard buckets (input, cache-read, 5-minute cache-write, output),
        // including any long-context tiering, come from the canonical table.
        // Unknown/retired models fall back to Sonnet pricing.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "clamped to i32::MAX before casting"
        )]
        let clamp = |v: u64| v.min(i32::MAX as u64) as i32;
        let base = CostUsagePricing::claude_cost_usd(
            model,
            clamp(input),
            clamp(cache_read),
            clamp(cache_create_5m),
            clamp(output),
        )
        .or_else(|| {
            CostUsagePricing::claude_cost_usd(
                FALLBACK_CLAUDE_MODEL,
                clamp(input),
                clamp(cache_read),
                clamp(cache_create_5m),
                clamp(output),
            )
        })
        .unwrap_or(0.0);

        // Scanner-specific: one-hour cache writes bill at 2x the input rate.
        let input_rate = CostUsagePricing::claude_input_cost_per_token(model)
            .or_else(|| CostUsagePricing::claude_input_cost_per_token(FALLBACK_CLAUDE_MODEL))
            .unwrap_or(0.0);

        base + (cache_create_1h as f64) * input_rate * 2.0
    }
}

/// JSONL event structures for Codex
#[allow(
    dead_code,
    reason = "JSONL event fields are deserialized for parsing but not all are read"
)]
#[derive(Debug, Deserialize)]
struct CodexEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    event_msg: Option<CodexEventMsg>,
}

#[allow(
    dead_code,
    reason = "event message fields are deserialized for parsing but not all are read"
)]
#[derive(Debug, Deserialize)]
struct CodexEventMsg {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

/// JSONL event structures for Claude transcripts.
///
/// The flattened values retain otherwise-unknown metadata long enough to
/// distinguish Anthropic rows from Vertex AI rows. Claude's local transcript
/// format can contain both shapes, and counting Vertex rows with Anthropic
/// pricing would misstate both cost and token history.
#[derive(Debug, Deserialize)]
struct ClaudeEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId", alias = "request_id")]
    request_id: Option<String>,
    message: Option<ClaudeMessage>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl ClaudeEvent {
    fn parsed_timestamp(&self) -> Option<DateTime<Utc>> {
        let timestamp = self.timestamp.as_deref()?;
        DateTime::parse_from_rfc3339(timestamp)
            .ok()
            .map(|ts| ts.with_timezone(&Utc))
    }

    fn is_vertex_ai_usage_entry(&self) -> bool {
        // Vertex AI message/request identifiers use the `_vrtx_` marker.
        if self
            .message
            .as_ref()
            .and_then(|message| message.id.as_deref())
            .is_some_and(|id| id.contains("_vrtx_"))
            || self
                .request_id
                .as_deref()
                .is_some_and(|request_id| request_id.contains("_vrtx_"))
        {
            return true;
        }

        // Vertex AI model names use `@` as the version separator.
        if self
            .message
            .as_ref()
            .and_then(|message| message.model.as_deref())
            .is_some_and(model_name_looks_vertex)
        {
            return true;
        }

        if contains_claude_vertex_metadata_entries(self.extra.iter()) {
            return true;
        }
        self.message
            .as_ref()
            .is_some_and(ClaudeMessage::contains_vertex_metadata)
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl ClaudeMessage {
    fn contains_vertex_metadata(&self) -> bool {
        if contains_claude_vertex_metadata_entries(self.extra.iter()) {
            return true;
        }
        self.usage
            .as_ref()
            .is_some_and(ClaudeUsage::contains_vertex_metadata)
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation: Option<ClaudeCacheCreation>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl ClaudeUsage {
    fn contains_vertex_metadata(&self) -> bool {
        if contains_claude_vertex_metadata_entries(self.extra.iter()) {
            return true;
        }
        self.cache_creation
            .as_ref()
            .is_some_and(ClaudeCacheCreation::contains_vertex_metadata)
    }
}

impl ClaudeUsage {
    /// One-hour cache-write tokens, clamped to the total cache-write count.
    fn one_hour_cache_creation_tokens(&self, total: u64) -> u64 {
        self.cache_creation
            .as_ref()
            .and_then(|cache_creation| cache_creation.ephemeral_1h_input_tokens)
            .unwrap_or(0)
            .min(total)
    }
}

/// TTL breakdown of cache writes reported by the API.
#[derive(Debug, Deserialize)]
struct ClaudeCacheCreation {
    ephemeral_1h_input_tokens: Option<u64>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl ClaudeCacheCreation {
    fn contains_vertex_metadata(&self) -> bool {
        contains_claude_vertex_metadata_entries(self.extra.iter())
    }
}

const CLAUDE_VERTEX_PROVIDER_KEYS: &[&str] = &[
    "provider",
    "platform",
    "backend",
    "api_provider",
    "apiprovider",
    "api_type",
    "apitype",
    "source",
    "vendor",
    "client",
];

fn model_name_looks_vertex(model: &str) -> bool {
    model.starts_with("claude-") && model.contains('@')
}

/// Match the upstream Claude classifier's recursive metadata rules. Marker
/// keys (`vertex`/`gcp`) classify regardless of value; provider-key values
/// classify only when their text contains `vertex` (not merely `gcp`).
fn contains_claude_vertex_metadata(value: &Value) -> bool {
    match value {
        Value::Object(object) => contains_claude_vertex_metadata_entries(object.iter()),
        Value::Array(array) => array.iter().any(contains_claude_vertex_metadata),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn contains_claude_vertex_metadata_entries<'a, I>(entries: I) -> bool
where
    I: IntoIterator<Item = (&'a String, &'a Value)>,
{
    entries.into_iter().any(|(key, value)| {
        contains_claude_vertex_marker(key, true)
            || (CLAUDE_VERTEX_PROVIDER_KEYS
                .iter()
                .any(|candidate| key.eq_ignore_ascii_case(candidate))
                && value
                    .as_str()
                    .is_some_and(|text| contains_claude_vertex_marker(text, false)))
            || contains_claude_vertex_metadata(value)
    })
}

fn contains_claude_vertex_marker(value: &str, include_gcp: bool) -> bool {
    let bytes = value.as_bytes();
    let has_marker = |marker: &[u8]| {
        bytes.windows(marker.len()).any(|window| {
            window
                .iter()
                .zip(marker)
                .all(|(byte, expected)| byte.to_ascii_lowercase() == *expected)
        })
    };

    if has_marker(b"vertex") || (include_gcp && has_marker(b"gcp")) {
        return true;
    }

    // ASCII folding above is enough for the common path. Unicode lowercasing
    // preserves the historical classifier's behavior for non-ASCII strings.
    if value.is_ascii() {
        return false;
    }
    let lower = value.to_lowercase();
    lower.contains("vertex") || (include_gcp && lower.contains("gcp"))
}

#[derive(Debug)]
struct ClaudeUsageRecord {
    model: String,
    timestamp: Option<DateTime<Utc>>,
    dedup_key: Option<String>,
    input: u64,
    output: u64,
    cache_create: u64,
    cache_read: u64,
    cost: f64,
}

/// Per-pass counters for cache/resume behavior (tests + diagnostics).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CostScanStats {
    pub files_seen: u32,
    pub files_parsed: u32,
    pub files_skipped: u32,
    pub files_resumed: u32,
    /// Files deferred to a later bounded Codex catch-up pass.
    pub files_deferred: u32,
    /// Newly consumed Codex JSONL bytes in this refresh.
    pub codex_bytes_read: u64,
    /// Timestamp comparisons performed while validating Codex append history.
    pub token_timestamp_comparisons: u64,
    pub used_cache_debounce: bool,
}

#[derive(Debug, Clone)]
struct CodexScanCandidate {
    path: PathBuf,
    mtime_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, Default)]
struct CodexFileScanOutcome {
    bytes_read: i64,
    is_complete: bool,
}

/// Cost usage scanner
pub struct CostScanner {
    days: u32,
    options: CostScanOptions,
    cache_root: Option<PathBuf>,
    /// When set, bypass normal sessions-dir discovery (tests / inject roots).
    sessions_dirs_override: Option<Vec<PathBuf>>,
}

impl CostScanner {
    /// Create a new scanner for the last N days (default 60s cache debounce).
    pub fn new(days: u32) -> Self {
        Self {
            days,
            options: CostScanOptions::default(),
            cache_root: None,
            sessions_dirs_override: None,
        }
    }

    /// Override scan options (e.g. [`CostScanOptions::app_driven`] for force refresh).
    pub fn with_options(mut self, options: CostScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Override on-disk cache root (`{root}/cost-usage/…`).
    pub fn with_cache_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.cache_root = Some(root.into());
        self
    }

    /// Override Codex sessions roots (primarily for tests).
    pub fn with_sessions_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.sessions_dirs_override = Some(dirs);
        self
    }

    /// Scan Codex local logs
    pub fn scan_codex(&self) -> CostSummary {
        self.scan_codex_with_cancel(None)
    }

    /// Scan Codex local logs, stopping early when the caller cancels the scan.
    pub fn scan_codex_with_cancel(&self, cancel: Option<&AtomicBool>) -> CostSummary {
        self.scan_codex_detailed(cancel).0
    }

    /// Scan Codex and return cache/resume stats alongside the summary.
    pub fn scan_codex_detailed(&self, cancel: Option<&AtomicBool>) -> (CostSummary, CostScanStats) {
        let (summary, stats, _cache) = self.scan_codex_detailed_with_cache(cancel);
        (summary, stats)
    }

    /// Scan Codex and retain the decoded cache baseline for same-cycle readers.
    ///
    /// The returned cache is the exact in-memory value used for publication,
    /// including any persistence-budget pruning.  Callers that only need the
    /// summary should use [`Self::scan_codex_detailed`]; daily history readers
    /// use this seam to avoid decoding the same native cache a second time.
    pub(crate) fn scan_codex_detailed_with_cache(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> (CostSummary, CostScanStats, CostUsageCache) {
        let mut summary = CostSummary::default();
        let mut stats = CostScanStats::default();
        let today = Local::now().date_naive();
        let start_date = codex_period_start(today, self.days);
        let range = CostUsageDayRange::new(start_date, today);
        let now_ms = unix_now_ms();

        summary.period_start = Some(start_date);
        summary.period_end = Some(today);

        let cache_root = self.cache_root.as_deref();
        let mut cache = JsonlScanner::load_cache(ProviderId::Codex, cache_root);

        // Debounce: rebuild from disk cache without re-walking session files.
        if JsonlScanner::should_skip_cached_scan(&cache, self.options, now_ms)
            && !cache.codex_scan_incomplete
            && JsonlScanner::cache_covers_range(&cache, &range)
            && (!cache.days.is_empty() || !cache.files.is_empty())
        {
            stats.used_cache_debounce = true;
            // A16 (upstream 0.48.0): cache hit within debounce = coverage established
            // when the cache has data and no catch-up is pending. Final publication
            // also waits for the cancellable Pi/OMP scan below.
            let cached_history_coverage_established = !cache.codex_scan_incomplete
                && cache.previous_report.is_none()
                && JsonlScanner::cache_covers_range(&cache, &range);
            let (cost, _) = add_codex_days_map_to_summary(&mut summary, &cache.days, &range);
            summary.total_cost_usd += cost;
            // Session count is a display field; the cache holds far fewer files than u32::MAX.
            #[allow(clippy::cast_possible_truncation, reason = "cache file counts fit u32")]
            let sessions_count = cache
                .files
                .values()
                .filter(|usage| {
                    usage.days.keys().any(|day| {
                        CostUsageDayRange::is_in_range(day, &range.since_key, &range.until_key)
                    })
                })
                .count() as u32;
            summary.sessions_count = sessions_count;

            // Pi-compatible sessions are outside the Codex JSONL cache.
            // Skip when tests inject sessions roots — avoid scanning the real home tree.
            if self.sessions_dirs_override.is_none() {
                let mut seen_pi = HashSet::new();
                crate::pi_session_cost::scan_pi_compatible_into(
                    &mut summary,
                    crate::pi_session_cost::PiMappedProvider::Codex,
                    self.days,
                    cancel,
                    &mut seen_pi,
                );
            }
            summary.history_coverage_established =
                cached_history_coverage_established && !is_cancelled(cancel);
            // Upstream 0.50.1 #2932: debounce cache hit with coverage
            // established but zero sessions in-range is a known-zero.
            summary.known_zero =
                summary.history_coverage_established && summary.sessions_count == 0;
            return (summary, stats, cache);
        }

        let sessions_dirs = self.get_codex_sessions_dirs();
        let established_report_before_scan = (!cache.codex_scan_incomplete
            && cache.previous_report.is_none()
            && (cache.scan_since_key.is_some()
                || !cache.days.is_empty()
                || !cache.files.is_empty()))
        .then(|| JsonlScanner::cached_cost_report_from_days(&cache));

        let (candidates, discovery_complete) =
            self.collect_codex_candidates(&sessions_dirs, &range, &cache, cancel, &mut stats);
        let candidate_limit = if self.options.codex_candidate_limit == 0 {
            usize::MAX
        } else {
            self.options.codex_candidate_limit
        };
        let refresh_byte_limit = if self.options.codex_max_scan_bytes_per_refresh <= 0 {
            i64::MAX
        } else {
            self.options.codex_max_scan_bytes_per_refresh
        };
        let per_file_limit = if self.options.codex_max_session_file_bytes <= 0 {
            i64::MAX
        } else {
            self.options.codex_max_session_file_bytes
        };
        let mut bytes_read_this_refresh = 0_i64;
        let mut pending_next = cache.codex_pending_paths.clone();
        let pending_paths_before_pass = cache.codex_pending_paths.clone();
        if discovery_complete && !is_cancelled(cancel) {
            pending_next
                .retain(|path| !cached_codex_file_is_complete_for_range(&cache, path, &range));
        }

        for (index, candidate) in candidates.iter().enumerate() {
            if is_cancelled(cancel)
                || index >= candidate_limit
                || bytes_read_this_refresh >= refresh_byte_limit
            {
                for deferred in &candidates[index..] {
                    let key = deferred.path.to_string_lossy().to_string();
                    if !pending_next.contains(&key) {
                        pending_next.push(key);
                    }
                }
                stats.files_deferred = stats
                    .files_deferred
                    .saturating_add((candidates.len() - index).min(u32::MAX as usize) as u32);
                break;
            }

            let refresh_remaining = refresh_byte_limit.saturating_sub(bytes_read_this_refresh);
            let allowance = per_file_limit.min(refresh_remaining);
            if allowance <= 0 {
                for deferred in &candidates[index..] {
                    let key = deferred.path.to_string_lossy().to_string();
                    if !pending_next.contains(&key) {
                        pending_next.push(key);
                    }
                }
                stats.files_deferred = stats
                    .files_deferred
                    .saturating_add((candidates.len() - index).min(u32::MAX as usize) as u32);
                break;
            }

            let outcome = self.parse_codex_file_bounded(
                &candidate.path,
                &range,
                &mut summary,
                &mut cache,
                cancel,
                &mut stats,
                Some(allowance),
            );
            bytes_read_this_refresh =
                bytes_read_this_refresh.saturating_add(outcome.bytes_read.max(0));
            stats.codex_bytes_read = stats
                .codex_bytes_read
                .saturating_add(u64::try_from(outcome.bytes_read.max(0)).unwrap_or(u64::MAX));
            let key = candidate.path.to_string_lossy().to_string();
            pending_next.retain(|pending| pending != &key);
            if !outcome.is_complete {
                pending_next.push(key);
                stats.files_deferred = stats.files_deferred.saturating_add(1);
            }
        }

        if discovery_complete && !is_cancelled(cancel) {
            reconcile_missing_codex_cache_files(&mut cache, &sessions_dirs, &range);
            for path in &pending_paths_before_pass {
                if !Path::new(path).exists() {
                    cache.files.remove(path);
                }
            }
        }
        pending_next.retain(|path| {
            Path::new(path).exists()
                && (!discovery_complete
                    || is_cancelled(cancel)
                    || is_codex_path_in_scan_window(Path::new(path), &sessions_dirs, &range))
        });
        pending_next.sort();
        pending_next.dedup();
        cache.codex_pending_paths = pending_next;
        cache.codex_scan_incomplete =
            !discovery_complete || is_cancelled(cancel) || !cache.codex_pending_paths.is_empty();
        rebuild_cache_days(&mut cache);
        cache.last_scan_unix_ms = now_ms;
        if cache.codex_scan_incomplete {
            if cache.previous_report.is_none() {
                cache.previous_report = established_report_before_scan;
            }
        } else {
            cache.scan_since_key = Some(range.since_key.clone());
            cache.scan_until_key = Some(range.until_key.clone());
            cache.previous_report = None;
        }
        JsonlScanner::save_cache(ProviderId::Codex, &mut cache, cache_root);

        // Build the current native summary from the complete decoded cache view,
        // including prior cached files that were not reread in this bounded pass.
        // A cancelled pass retains missing rows on disk for deletion
        // reconciliation, but must not publish those stale rows in its summary.
        let mut summary_cache = cache.clone();
        if is_cancelled(cancel) {
            summary_cache
                .files
                .retain(|path, _| Path::new(path).exists());
            rebuild_cache_days(&mut summary_cache);
        }
        let mut rebuilt = CostSummary {
            period_start: Some(start_date),
            period_end: Some(today),
            ..CostSummary::default()
        };
        let (native_cost, _) =
            add_codex_days_map_to_summary(&mut rebuilt, &summary_cache.days, &range);
        rebuilt.total_cost_usd += native_cost;
        #[allow(clippy::cast_possible_truncation, reason = "cache file counts fit u32")]
        {
            rebuilt.sessions_count = summary_cache
                .files
                .values()
                .filter(|usage| {
                    usage.days.keys().any(|day| {
                        CostUsageDayRange::is_in_range(day, &range.since_key, &range.until_key)
                    })
                })
                .count() as u32;
        }
        let cancelled_with_missing_cache_rows =
            is_cancelled(cancel) && cache.files.keys().any(|path| !Path::new(path).exists());
        let preserving_previous_report = cache.codex_scan_incomplete
            && cache.previous_report.is_some()
            && !cancelled_with_missing_cache_rows;
        summary = if preserving_previous_report {
            cache
                .previous_report
                .as_ref()
                .map(|report| summary_from_cached_report(report, start_date, today))
                .unwrap_or(rebuilt)
        } else {
            rebuilt
        };

        // OMP / pi-compatible agent sessions (upstream #2269). Dedup by entry id.
        // Skip when tests inject sessions roots — avoid scanning the real home tree.
        // A16 --provider-native-only: skip pi/OMP mirrors when disabled.
        if !preserving_previous_report
            && self.sessions_dirs_override.is_none()
            && self.options.include_pi_sessions
        {
            let mut seen_pi = HashSet::new();
            crate::pi_session_cost::scan_pi_compatible_into(
                &mut summary,
                crate::pi_session_cost::PiMappedProvider::Codex,
                self.days,
                cancel,
                &mut seen_pi,
            );
        }

        // v0.56.1 #3279: only publish authoritative coverage after all
        // cancellable scan work, including Pi/OMP, has completed. Persistence
        // pruning may retain `previous_report`, but that must not make a
        // completed in-memory scan stale or make a cancelled partial scan look
        // complete.
        summary.history_coverage_established =
            !is_cancelled(cancel) && !cache.codex_scan_incomplete;
        // Upstream 0.50.1 #2932: a completed scan with zero results is a
        // *known* zero. Only set when coverage is established; an incomplete
        // scan must NOT fabricate a zero.
        summary.known_zero = summary.history_coverage_established && summary.sessions_count == 0;

        (summary, stats, cache)
    }

    /// Scan Claude local logs
    pub fn scan_claude(&self) -> CostSummary {
        self.scan_claude_with_cancel(None)
    }

    /// Scan Claude local logs, stopping early when the caller cancels the scan.
    pub fn scan_claude_with_cancel(&self, cancel: Option<&AtomicBool>) -> CostSummary {
        let projects_dir = self.get_claude_projects_dir();
        let mut summary = CostSummary::default();
        let today = Utc::now().date_naive();
        let start_date = today - Duration::days(self.days as i64);
        let cutoff = Utc::now() - Duration::days(self.days as i64);

        summary.period_start = Some(start_date);
        summary.period_end = Some(today);

        // Walk through projects directory, de-duplicating usage records
        // that appear across multiple files.
        if projects_dir.exists() {
            let mut seen = HashSet::new();
            let mut handle_file = |path: &Path| {
                let counted =
                    for_each_claude_usage_record(path, &cutoff, &mut seen, cancel, |record| {
                        add_claude_record_to_summary(&mut summary, record);
                    });
                if counted > 0 {
                    summary.sessions_count += 1;
                }
            };
            self.walk_claude_files(&projects_dir, &cutoff, cancel, &mut handle_file);
        }

        // OMP / pi-compatible anthropic rows, deduped across shared files.
        let mut seen_pi = HashSet::new();
        crate::pi_session_cost::scan_pi_compatible_into(
            &mut summary,
            crate::pi_session_cost::PiMappedProvider::Claude,
            self.days,
            cancel,
            &mut seen_pi,
        );

        summary
    }

    /// Scan OpenCode Go local SQLite usage (upstream #2649 per-model cost breakdown).
    ///
    /// Reads the local `opencode.db` and maps rows onto the shared `CostSummary`
    /// (`total_cost_usd`, `by_model`, `sessions_count`, period) so the chart's
    /// local-usage summary treats OpenCode Go like Codex/Claude. No token counts
    /// are available from the SQLite reader, so token fields stay zero.
    pub fn scan_opencodego_with_cancel(&self, cancel: Option<&AtomicBool>) -> CostSummary {
        if is_cancelled(cancel) {
            return CostSummary::default();
        }
        let now = Utc::now();
        let Some(local) = opencodego_local::model_cost_summary_scan(now, self.days) else {
            return CostSummary::default();
        };
        CostSummary {
            total_cost_usd: local.total_cost_usd,
            by_model: local.by_model,
            sessions_count: local.request_count,
            period_start: local.period_start,
            period_end: local.period_end,
            ..CostSummary::default()
        }
    }

    fn get_codex_sessions_dirs(&self) -> Vec<PathBuf> {
        if let Some(dirs) = &self.sessions_dirs_override {
            return dirs.clone();
        }
        let settings = Settings::load();
        let codex_home = std::env::var("CODEX_HOME").ok();
        codex_sessions_dir_candidates(
            dirs::home_dir(),
            codex_home,
            &settings.codex_custom_sessions_dirs,
            &default_wsl_roots(),
        )
    }

    fn collect_codex_candidates(
        &self,
        sessions_dirs: &[PathBuf],
        range: &CostUsageDayRange,
        cache: &CostUsageCache,
        cancel: Option<&AtomicBool>,
        stats: &mut CostScanStats,
    ) -> (Vec<CodexScanCandidate>, bool) {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        let mut dates = codex_scan_dates(range);
        if self.options.prefer_newest_codex_sessions_first {
            dates.reverse();
        }

        for sessions_dir in sessions_dirs {
            for date in &dates {
                if is_cancelled(cancel) {
                    return (candidates, false);
                }
                let day_dir = sessions_dir
                    .join(date.format("%Y").to_string())
                    .join(date.format("%m").to_string())
                    .join(date.format("%d").to_string());
                let Ok(entries) = fs::read_dir(day_dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    if is_cancelled(cancel) {
                        return (candidates, false);
                    }
                    let path = entry.path();
                    if !path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
                    {
                        continue;
                    }
                    let path_key = path.to_string_lossy().to_string();
                    if !seen.insert(path_key.clone()) {
                        continue;
                    }
                    let Ok(metadata) = entry.metadata() else {
                        continue;
                    };
                    let mtime_unix_ms = system_time_to_unix_ms(metadata.modified().ok());
                    let unchanged_complete =
                        cached_codex_file_is_complete_for_range(cache, &path_key, range);
                    if unchanged_complete {
                        stats.files_seen = stats.files_seen.saturating_add(1);
                        stats.files_skipped = stats.files_skipped.saturating_add(1);
                        continue;
                    }
                    candidates.push(CodexScanCandidate {
                        path,
                        mtime_unix_ms,
                    });
                }
            }
        }

        // Persisted paths are retried even if their directory partition was not
        // rediscovered this pass, as long as they remain in the requested scan
        // window. Missing paths are pruned after a complete discovery pass.
        for path_key in &cache.codex_pending_paths {
            if seen.contains(path_key) {
                continue;
            }
            let path = PathBuf::from(path_key);
            if !is_codex_path_in_scan_window(&path, sessions_dirs, range) {
                continue;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            candidates.push(CodexScanCandidate {
                path,
                mtime_unix_ms: system_time_to_unix_ms(metadata.modified().ok()),
            });
        }

        if self.options.prefer_newest_codex_sessions_first {
            candidates.sort_by(|lhs, rhs| {
                rhs.mtime_unix_ms
                    .cmp(&lhs.mtime_unix_ms)
                    .then_with(|| rhs.path.cmp(&lhs.path))
            });
        } else {
            candidates.sort_by(|lhs, rhs| {
                lhs.mtime_unix_ms
                    .cmp(&rhs.mtime_unix_ms)
                    .then_with(|| lhs.path.cmp(&rhs.path))
            });
        }
        (candidates, true)
    }

    fn get_claude_projects_dir(&self) -> PathBuf {
        if let Ok(claude_config) = std::env::var("CLAUDE_CONFIG_DIR") {
            let trimmed = claude_config.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join("projects");
            }
        }

        // Try ~/.claude/projects first
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let claude_dir = home.join(".claude").join("projects");
        if claude_dir.exists() {
            return claude_dir;
        }

        // Fallback to ~/.config/claude/projects
        home.join(".config").join("claude").join("projects")
    }

    fn parse_codex_file(
        &self,
        path: &Path,
        range: &CostUsageDayRange,
        summary: &mut CostSummary,
        cache: &mut CostUsageCache,
        cancel: Option<&AtomicBool>,
        stats: &mut CostScanStats,
    ) {
        let _ = self.parse_codex_file_bounded(path, range, summary, cache, cancel, stats, None);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "bounded file scan carries shared scan state"
    )]
    fn parse_codex_file_bounded(
        &self,
        path: &Path,
        range: &CostUsageDayRange,
        summary: &mut CostSummary,
        cache: &mut CostUsageCache,
        cancel: Option<&AtomicBool>,
        stats: &mut CostScanStats,
        max_bytes_to_read: Option<i64>,
    ) -> CodexFileScanOutcome {
        if is_cancelled(cancel) {
            return CodexFileScanOutcome::default();
        }
        stats.files_seen = stats.files_seen.saturating_add(1);

        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => return CodexFileScanOutcome::default(),
        };
        #[allow(
            clippy::cast_possible_wrap,
            reason = "file sizes are clamped to i64::MAX"
        )]
        let size = metadata.len().min(i64::MAX as u64) as i64;
        let mtime_ms = system_time_to_unix_ms(metadata.modified().ok());
        let path_key = path.to_string_lossy().to_string();
        let cached = cache.files.get(&path_key).cloned();
        let cache_covers_range = JsonlScanner::cache_covers_range(cache, range);
        let session_metadata = JsonlScanner::read_codex_session_metadata(path).unwrap_or_default();
        let cached_identity_matches = cached
            .as_ref()
            .is_some_and(|entry| entry.mtime_unix_ms == mtime_ms && entry.size == size);
        let codex_session_id = session_metadata.session_id.clone().or_else(|| {
            cached_identity_matches
                .then(|| cached.as_ref()?.codex_session_id.clone())
                .flatten()
        });
        let codex_forked_from_id = session_metadata.forked_from_id.clone().or_else(|| {
            cached_identity_matches
                .then(|| cached.as_ref()?.codex_forked_from_id.clone())
                .flatten()
        });
        let codex_fork_timestamp = session_metadata.fork_timestamp.clone().or_else(|| {
            cached_identity_matches
                .then(|| cached.as_ref()?.codex_fork_timestamp.clone())
                .flatten()
        });
        let is_fork = codex_forked_from_id.is_some();
        let fork_baseline = codex_forked_from_id.as_deref().and_then(|parent_id| {
            codex_parent_baseline(cache, parent_id, codex_fork_timestamp.as_deref())
        });

        if is_fork && fork_baseline.is_none() {
            cache.files.insert(
                path_key,
                CostUsageFileUsage {
                    mtime_unix_ms: mtime_ms,
                    size,
                    days: HashMap::new(),
                    parsed_bytes: Some(0),
                    last_model: None,
                    last_totals: None,
                    codex_token_timestamps_monotonic: None,
                    codex_last_token_timestamp: None,
                    codex_session_id,
                    codex_forked_from_id,
                    codex_fork_timestamp,
                    codex_unresolved_fork_parent: true,
                },
            );
            stats.files_parsed = stats.files_parsed.saturating_add(1);
            return CodexFileScanOutcome {
                bytes_read: 0,
                is_complete: false,
            };
        }

        if let Some(entry) = &cached
            && cache_covers_range
            && !entry.codex_unresolved_fork_parent
            && entry.mtime_unix_ms == mtime_ms
            && entry.size == size
            && entry.parsed_bytes.unwrap_or(0) >= size
        {
            let (session_cost, has_tokens) =
                add_codex_days_map_to_summary(summary, &entry.days, range);
            if has_tokens {
                summary.total_cost_usd += session_cost;
                summary.sessions_count += 1;
            }
            stats.files_skipped = stats.files_skipped.saturating_add(1);
            return CodexFileScanOutcome {
                bytes_read: 0,
                is_complete: true,
            };
        }

        if !is_fork && let Some(entry) = &cached {
            let start_offset = entry.parsed_bytes.unwrap_or(0);
            let same_partial =
                size == entry.size && mtime_ms == entry.mtime_unix_ms && start_offset < size;
            let growing = size > entry.size;
            let parser_state_safe = entry.codex_token_timestamps_monotonic.is_some();
            if cache_covers_range
                && (same_partial || growing)
                && start_offset > 0
                && start_offset <= size
                && parser_state_safe
                && JsonlScanner::is_line_boundary_offset(path, start_offset)
            {
                let parse_result = match JsonlScanner::parse_codex_file_with_state_bounded(
                    path,
                    range,
                    start_offset,
                    entry.last_model.clone(),
                    entry.last_totals.clone(),
                    entry.codex_last_token_timestamp.clone(),
                    entry.codex_token_timestamps_monotonic,
                    cancel,
                    max_bytes_to_read,
                ) {
                    Ok(result) => result,
                    Err(_) => return CodexFileScanOutcome::default(),
                };
                stats.token_timestamp_comparisons = stats
                    .token_timestamp_comparisons
                    .saturating_add(parse_result.token_timestamp_comparisons);
                let mut days = entry.days.clone();
                merge_codex_records_into_days(&mut days, &parse_result.records);
                let (session_cost, has_tokens) =
                    add_codex_days_map_to_summary(summary, &days, range);
                if has_tokens {
                    summary.total_cost_usd += session_cost;
                    summary.sessions_count += 1;
                }
                let outcome = CodexFileScanOutcome {
                    bytes_read: parse_result.bytes_read,
                    is_complete: parse_result.is_complete,
                };
                cache.files.insert(
                    path_key,
                    CostUsageFileUsage {
                        mtime_unix_ms: mtime_ms,
                        size,
                        days,
                        parsed_bytes: Some(parse_result.parsed_bytes),
                        last_model: parse_result.last_model.or_else(|| entry.last_model.clone()),
                        last_totals: parse_result
                            .last_totals
                            .or_else(|| entry.last_totals.clone()),
                        codex_token_timestamps_monotonic: parse_result
                            .token_timestamps_monotonic
                            .or(entry.codex_token_timestamps_monotonic),
                        codex_last_token_timestamp: parse_result
                            .last_token_timestamp
                            .or_else(|| entry.codex_last_token_timestamp.clone()),
                        codex_session_id: codex_session_id.clone(),
                        codex_forked_from_id: codex_forked_from_id.clone(),
                        codex_fork_timestamp: codex_fork_timestamp.clone(),
                        codex_unresolved_fork_parent: false,
                    },
                );
                stats.files_resumed = stats.files_resumed.saturating_add(1);
                return outcome;
            }
        }

        let parse_result = match if let Some(baseline) = fork_baseline.clone() {
            JsonlScanner::parse_codex_file_with_state_bounded_fork(
                path,
                range,
                baseline,
                cancel,
                max_bytes_to_read,
            )
        } else {
            JsonlScanner::parse_codex_file_with_state_bounded(
                path,
                range,
                0,
                None,
                None,
                None,
                None,
                cancel,
                max_bytes_to_read,
            )
        } {
            Ok(result) => result,
            Err(_) => return CodexFileScanOutcome::default(),
        };
        stats.token_timestamp_comparisons = stats
            .token_timestamp_comparisons
            .saturating_add(parse_result.token_timestamp_comparisons);
        if parse_result.fork_baseline_ambiguous {
            cache.files.insert(
                path_key,
                CostUsageFileUsage {
                    mtime_unix_ms: mtime_ms,
                    size,
                    days: HashMap::new(),
                    parsed_bytes: Some(0),
                    last_model: None,
                    last_totals: None,
                    codex_token_timestamps_monotonic: None,
                    codex_last_token_timestamp: None,
                    codex_session_id,
                    codex_forked_from_id,
                    codex_fork_timestamp,
                    codex_unresolved_fork_parent: true,
                },
            );
            stats.files_parsed = stats.files_parsed.saturating_add(1);
            return CodexFileScanOutcome {
                bytes_read: parse_result.bytes_read,
                is_complete: false,
            };
        }
        let mut days = HashMap::new();
        merge_codex_records_into_days(&mut days, &parse_result.records);
        let (session_cost, has_tokens) =
            add_codex_records_to_summary(summary, &parse_result.records, range);
        if has_tokens {
            summary.total_cost_usd += session_cost;
            summary.sessions_count += 1;
        }
        let outcome = CodexFileScanOutcome {
            bytes_read: parse_result.bytes_read,
            is_complete: parse_result.is_complete,
        };
        cache.files.insert(
            path_key,
            CostUsageFileUsage {
                mtime_unix_ms: mtime_ms,
                size,
                days,
                parsed_bytes: Some(parse_result.parsed_bytes),
                last_model: parse_result.last_model,
                last_totals: parse_result.last_totals,
                codex_token_timestamps_monotonic: parse_result.token_timestamps_monotonic,
                codex_last_token_timestamp: parse_result.last_token_timestamp,
                codex_session_id,
                codex_forked_from_id,
                codex_fork_timestamp,
                codex_unresolved_fork_parent: false,
            },
        );
        stats.files_parsed = stats.files_parsed.saturating_add(1);
        outcome
    }

    fn walk_claude_files<F>(
        &self,
        dir: &Path,
        cutoff: &DateTime<Utc>,
        cancel: Option<&AtomicBool>,
        on_file: &mut F,
    ) where
        F: FnMut(&Path),
    {
        if is_cancelled(cancel) {
            return;
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            if is_cancelled(cancel) {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                self.walk_claude_files(&path, cutoff, cancel, on_file);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                // Check file modification time
                if let Ok(metadata) = fs::metadata(&path)
                    && let Ok(modified) = metadata.modified()
                {
                    let modified_dt: DateTime<Utc> = modified.into();
                    if modified_dt >= *cutoff {
                        on_file(&path);
                    }
                }
            }
        }
    }
}

/// Stream the de-duplicated, in-window usage records from one transcript
/// file into `on_record`. Both the summary scan and the daily-history scan
/// consume this single reader, so Claude log semantics live in one place.
/// Returns the number of records consumed, so callers can tell whether the
/// file contributed anything.
fn for_each_claude_usage_record<F>(
    path: &Path,
    cutoff: &DateTime<Utc>,
    seen: &mut HashSet<String>,
    cancel: Option<&AtomicBool>,
    mut on_record: F,
) -> usize
where
    F: FnMut(&ClaudeUsageRecord),
{
    let Ok(file) = File::open(path) else {
        return 0;
    };

    let mut counted = 0;
    // Use read_until so a final incomplete line (no trailing newline) is still
    // processed when it is valid UTF-8 JSON, and so a single bad line does not
    // stop the walk the way `lines().map_while(Result::ok)` would.
    for_each_jsonl_text_line(BufReader::new(file), |line| {
        if is_cancelled(cancel) {
            return false;
        }
        if let Ok(event) = serde_json::from_str::<ClaudeEvent>(line)
            && !event.is_vertex_ai_usage_entry()
            && let Some(record) = claude_usage_record_from_event(&event)
            && should_count_claude_record(&record, cutoff, seen)
        {
            counted += 1;
            on_record(&record);
        }
        true
    });
    counted
}

/// Walk JSONL text lines from `reader`, including a final incomplete line at EOF.
/// Continues past invalid UTF-8 segments. `on_line` returns `false` to stop early.
fn for_each_jsonl_text_line<R, F>(mut reader: R, mut on_line: F)
where
    R: BufRead,
    F: FnMut(&str) -> bool,
{
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        let Ok(line) = std::str::from_utf8(&buf) else {
            continue;
        };
        if !on_line(line) {
            break;
        }
    }
}

fn claude_usage_record_from_event(event: &ClaudeEvent) -> Option<ClaudeUsageRecord> {
    if event.event_type.as_deref() != Some("assistant") {
        return None;
    }

    let message = event.message.as_ref()?;
    let usage = message.usage.as_ref()?;
    let model = message.model.as_deref().unwrap_or("claude-3-5-sonnet");

    let input = usage.input_tokens.unwrap_or(0);
    let output = usage.output_tokens.unwrap_or(0);
    let cache_create = usage.cache_creation_input_tokens.unwrap_or(0);
    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);

    if input == 0 && output == 0 && cache_create == 0 && cache_read == 0 {
        return None;
    }

    let cache_create_1h = usage.one_hour_cache_creation_tokens(cache_create);
    let cost = ClaudePricing::cost_usd_with_cache_ttl(
        model,
        input,
        cache_create,
        cache_create_1h,
        cache_read,
        output,
    );

    Some(ClaudeUsageRecord {
        model: model.to_string(),
        timestamp: event.parsed_timestamp(),
        dedup_key: claude_usage_dedup_key(message.id.as_deref(), event.request_id.as_deref()),
        input,
        output,
        cache_create,
        cache_read,
        cost,
    })
}

fn claude_usage_dedup_key(message_id: Option<&str>, request_id: Option<&str>) -> Option<String> {
    match (message_id, request_id) {
        (Some(message_id), Some(request_id)) => Some(format!("{message_id}:{request_id}")),
        (Some(message_id), None) => Some(format!("message:{message_id}")),
        (None, Some(request_id)) => Some(format!("request:{request_id}")),
        (None, None) => None,
    }
}

fn should_count_claude_record(
    record: &ClaudeUsageRecord,
    cutoff: &DateTime<Utc>,
    seen: &mut HashSet<String>,
) -> bool {
    if let Some(timestamp) = record.timestamp
        && timestamp < *cutoff
    {
        return false;
    }

    if let Some(key) = &record.dedup_key
        && !seen.insert(key.clone())
    {
        return false;
    }

    true
}

fn add_claude_record_to_summary(summary: &mut CostSummary, record: &ClaudeUsageRecord) {
    if CostUsagePricing::claude_cost_usd(&record.model, 0, 0, 0, 0).is_none() {
        summary.unknown_models.insert(record.model.clone());
    }

    summary.input_tokens += record.input;
    summary.output_tokens += record.output;
    summary.cached_tokens += record.cache_create + record.cache_read;
    summary.total_cost_usd += record.cost;

    *summary.by_model.entry(record.model.clone()).or_insert(0.0) += record.cost;

    let model_tokens = summary
        .by_model_tokens
        .entry(record.model.clone())
        .or_default();
    model_tokens.input_tokens += record.input;
    model_tokens.output_tokens += record.output;
    model_tokens.cached_tokens += record.cache_create + record.cache_read;
}

/// Add one usage record to the per-day cost buckets, keyed by the record's
/// own timestamp in the local timezone. Records outside the initialized
/// date range (or without a timestamp) are ignored.
fn add_claude_record_to_daily_costs(
    daily_costs: &mut HashMap<String, Option<f64>>,
    record: &ClaudeUsageRecord,
) {
    let Some(timestamp) = record.timestamp else {
        return;
    };
    let date_str = timestamp
        .with_timezone(&Local)
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    if let Some(cost) = daily_costs.get_mut(&date_str) {
        *cost = Some(cost.unwrap_or(0.0) + record.cost);
    }
}

/// Check if any cost usage sources are available
#[allow(
    dead_code,
    reason = "utility probe for cost-usage availability; not yet wired into all call sites"
)]
pub fn has_cost_usage_sources() -> bool {
    let scanner = CostScanner::new(1);
    scanner
        .get_codex_sessions_dirs()
        .iter()
        .any(|dir| dir.exists())
        || scanner.get_claude_projects_dir().exists()
        || crate::pi_session_cost::pi_compatible_session_roots(dirs::home_dir())
            .iter()
            .any(|dir| dir.exists())
}

/// Get daily cost history for the last N days
/// Returns calendar-preserving daily costs sorted by date. `None` means the day
/// is unscanned or contains unpriced Codex usage; `Some(0)` is a known zero.
pub fn get_daily_cost_history(provider: &str, days: u32) -> Vec<(String, Option<f64>)> {
    let scanner = CostScanner::new(days);
    let today = Local::now().date_naive();
    let mut daily_costs: HashMap<String, Option<f64>> = HashMap::new();

    // Initialize all days with 0
    for days_ago in 0..days {
        let date = today - Duration::days(days_ago as i64);
        let date_str = date.format("%Y-%m-%d").to_string();
        daily_costs.insert(date_str, (provider != "codex").then_some(0.0));
    }

    match provider {
        "codex" => {
            // Warm/refresh the disk cache, then price from packed days. v0.56.1
            // preserves every calendar slot and distinguishes covered zero from
            // unscanned/unpriced history.
            let (_summary, _stats, cache) = scanner.scan_codex_detailed_with_cache(None);
            if cache.previous_report.is_none() && !cache.codex_scan_incomplete {
                for (day_key, slot) in &mut daily_costs {
                    if cache
                        .scan_since_key
                        .as_deref()
                        .is_some_and(|since| day_key.as_str() >= since)
                        && cache
                            .scan_until_key
                            .as_deref()
                            .is_some_and(|until| day_key.as_str() <= until)
                    {
                        *slot = Some(0.0);
                    }
                }
            }
            for (day_key, models) in &cache.days {
                let Some(slot) = daily_costs.get_mut(day_key) else {
                    continue;
                };
                let Some(day) = CostUsageDayRange::parse_day_key(day_key) else {
                    continue;
                };
                let day_range = CostUsageDayRange::new(day, day);
                let mut one_day = HashMap::new();
                one_day.insert(day_key.clone(), models.clone());
                let mut scratch = CostSummary::default();
                let (cost, _) = add_codex_days_map_to_summary(&mut scratch, &one_day, &day_range);
                *slot = (!scratch.model_pricing_completeness.is_partial()).then_some(cost);
            }
        }
        "claude" => {
            // Real per-day breakdown: walk the project logs once,
            // de-duplicating records across files.
            let projects_dir = scanner.get_claude_projects_dir();
            if projects_dir.exists() {
                let cutoff = Utc::now() - Duration::days(days as i64);
                let mut seen = HashSet::new();
                let mut handle_file = |path: &Path| {
                    for_each_claude_usage_record(path, &cutoff, &mut seen, None, |record| {
                        add_claude_record_to_daily_costs(&mut daily_costs, record);
                    });
                };
                scanner.walk_claude_files(&projects_dir, &cutoff, None, &mut handle_file);
            }
        }
        "opencodego" => {
            // Per-day cost from the local OpenCode SQLite reader (upstream #2649).
            // Rows are grouped by local calendar day to match Codex/Claude keying.
            for (day_key, cost) in opencodego_local::daily_cost_series(Utc::now(), days) {
                if let Some(slot) = daily_costs.get_mut(&day_key) {
                    *slot = Some(slot.unwrap_or(0.0) + cost);
                }
            }
        }
        _ => {}
    }

    // Convert to sorted vector
    let mut result: Vec<(String, Option<f64>)> = daily_costs.into_iter().collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// Daily token totals (input + output) for the Tokens chart mode, plus
/// whether local history looks incomplete at the old edge of the window
/// (Codex backfill still in progress → the chart shows a "Refreshing"
/// marker; upstream 0.50.0 #2930).
pub fn get_daily_token_history(provider: &str, days: u32) -> (Vec<(String, u64)>, bool) {
    let scanner = CostScanner::new(days);
    let today = Local::now().date_naive();
    let mut daily_tokens: HashMap<String, u64> = HashMap::new();
    let mut covered_days: HashSet<String> = HashSet::new();

    // Initialize all days with 0
    for days_ago in 0..days {
        let date = today - Duration::days(days_ago as i64);
        let date_str = date.format("%Y-%m-%d").to_string();
        daily_tokens.insert(date_str, 0);
    }

    match provider {
        "codex" => {
            // Warm/refresh the disk cache, then read exact local token totals
            // from packed days through the same summary path the cost chart
            // uses.
            let (_summary, _stats, cache) = scanner.scan_codex_detailed_with_cache(None);
            for (day_key, models) in &cache.days {
                if !daily_tokens.contains_key(day_key) {
                    continue;
                }
                let Some(day) = CostUsageDayRange::parse_day_key(day_key) else {
                    continue;
                };
                let day_range = CostUsageDayRange::new(day, day);
                let mut one_day = HashMap::new();
                one_day.insert(day_key.clone(), models.clone());
                let mut scratch = CostSummary::default();
                add_codex_days_map_to_summary(&mut scratch, &one_day, &day_range);
                if let Some(slot) = daily_tokens.get_mut(day_key) {
                    *slot = scratch.input_tokens + scratch.output_tokens;
                }
                covered_days.insert(day_key.clone());
            }
        }
        "claude" => {
            // Per-day token breakdown from the same de-duplicated record walk
            // as the cost chart. The full walk is authoritative, so the
            // Refreshing marker never applies here.
            let projects_dir = scanner.get_claude_projects_dir();
            if projects_dir.exists() {
                let cutoff = Utc::now() - Duration::days(days as i64);
                let mut seen = HashSet::new();
                let mut handle_file = |path: &Path| {
                    for_each_claude_usage_record(path, &cutoff, &mut seen, None, |record| {
                        add_claude_record_to_daily_tokens(&mut daily_tokens, record);
                    });
                };
                scanner.walk_claude_files(&projects_dir, &cutoff, None, &mut handle_file);
            }
        }
        _ => {}
    }

    // Convert to sorted vector
    let mut result: Vec<(String, u64)> = daily_tokens.into_iter().collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));

    // Codex only: the bounded catch-up may not have reached the requested
    // depth yet. Incomplete = history exists but the oldest quarter of the
    // window has no scanned day.
    let incomplete = provider == "codex"
        && !covered_days.is_empty()
        && covered_days.len() < days as usize
        && result[..(result.len() / 4).max(1)]
            .iter()
            .any(|(date, _)| !covered_days.contains(date));

    (result, incomplete)
}

fn add_claude_record_to_daily_tokens(
    daily_tokens: &mut HashMap<String, u64>,
    record: &ClaudeUsageRecord,
) {
    let Some(timestamp) = record.timestamp else {
        return;
    };
    let date_str = timestamp
        .with_timezone(&Local)
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    if let Some(slot) = daily_tokens.get_mut(&date_str) {
        *slot += record.input + record.output;
    }
}

#[cfg(test)]
mod tests;
