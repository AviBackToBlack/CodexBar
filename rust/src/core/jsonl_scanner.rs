//! JSONL Scanner with Caching
//!
//! Incremental log file parsing for Codex and Claude session logs.
//! Supports file-level caching to avoid re-parsing unchanged files.

#![allow(
    dead_code,
    reason = "scanner types are deserialized from JSONL for parsing but not all are read"
)]

use crate::core::{CostUsagePricing, ProviderId};
use chrono::{DateTime, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Default)]
pub struct CachedCostReadStatus {
    pub has_days: bool,
    pub previous_report: Option<CachedCostReport>,
}

#[derive(Deserialize, Default)]
struct CachedCostReadStatusProjection {
    #[serde(
        default,
        rename = "days",
        deserialize_with = "deserialize_nonempty_object"
    )]
    has_days: bool,
    #[serde(default)]
    previous_report: Option<CachedCostReport>,
}

fn deserialize_nonempty_object<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{IgnoredAny, MapAccess, Visitor};

    struct NonemptyObjectVisitor;

    impl<'de> Visitor<'de> for NonemptyObjectVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut nonempty = false;
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {
                nonempty = true;
            }
            Ok(nonempty)
        }
    }

    deserializer.deserialize_map(NonemptyObjectVisitor)
}
/// Maximum retained Codex JSONL line size (upstream session-metadata bound).
const CODEX_JSONL_MAX_LINE_BYTES: usize = 256 * 1024;

/// Default scanner-side refresh debounce (upstream CostUsageScanner).
pub const DEFAULT_COST_SCAN_REFRESH_MIN_INTERVAL_SECS: u64 = 60;
/// Default number of dirty Codex rollouts inspected in one refresh.
pub const DEFAULT_CODEX_CANDIDATE_LIMIT: usize = 512;
/// Default maximum newly-read bytes from one Codex rollout in one refresh.
pub const DEFAULT_CODEX_MAX_SESSION_FILE_BYTES: i64 = 256 * 1024 * 1024;
/// Default maximum newly-read Codex bytes across one refresh.
pub const DEFAULT_CODEX_MAX_SCAN_BYTES_PER_REFRESH: i64 = 512 * 1024 * 1024;

/// Options for a cost scan pass (disk-cache-backed full inspections).
///
/// Default debounce is 60s between full disk inspections when a
/// [`CostUsageCache`] is present. Pass [`CostScanOptions::app_driven`] (interval 0)
/// for explicit/CLI refreshes. Production [`crate::cost_scanner::CostScanner`]
/// honors these options and persists cache under `{cache}/CodexBar/cost-usage/`.
#[derive(Debug, Clone, Copy)]
pub struct CostScanOptions {
    /// Minimum seconds between disk-cache-backed full inspections.
    /// Set to 0 to force a fresh scan (app-driven / forceRefresh).
    pub refresh_min_interval_secs: u64,
    /// A16 (upstream 0.48.0 --provider-native-only): when false, exclude
    /// pi/OMP-compatible agent session mirrors from Codex/Claude cost history.
    /// Defaults to true (include mirrors) for backward compatibility.
    pub include_pi_sessions: bool,
    /// Maximum bytes newly read from one Codex rollout during a refresh.
    pub codex_max_session_file_bytes: i64,
    /// Maximum Codex JSONL bytes newly read across one refresh.
    pub codex_max_scan_bytes_per_refresh: i64,
    /// Maximum dirty/new Codex rollout candidates processed per refresh.
    pub codex_candidate_limit: usize,
    /// Prefer recent Codex rollouts while historical catch-up is pending.
    pub prefer_newest_codex_sessions_first: bool,
}

impl Default for CostScanOptions {
    fn default() -> Self {
        Self {
            refresh_min_interval_secs: DEFAULT_COST_SCAN_REFRESH_MIN_INTERVAL_SECS,
            include_pi_sessions: true,
            codex_max_session_file_bytes: DEFAULT_CODEX_MAX_SESSION_FILE_BYTES,
            codex_max_scan_bytes_per_refresh: DEFAULT_CODEX_MAX_SCAN_BYTES_PER_REFRESH,
            codex_candidate_limit: DEFAULT_CODEX_CANDIDATE_LIMIT,
            prefer_newest_codex_sessions_first: true,
        }
    }
}

impl CostScanOptions {
    /// App-driven or forced refresh: skip the scanner debounce entirely.
    pub fn app_driven() -> Self {
        Self {
            refresh_min_interval_secs: 0,
            ..Self::default()
        }
    }

    /// Whether a prior scan at `last_scan_unix_ms` is still within the debounce window.
    pub fn should_skip_scan(&self, last_scan_unix_ms: i64, now_unix_ms: i64) -> bool {
        // Debounce intervals are seconds-scale config values, far below i64::MAX.
        #[allow(
            clippy::cast_possible_wrap,
            reason = "debounce interval in seconds is a small config value that cannot exceed i64::MAX"
        )]
        let refresh_ms = (self.refresh_min_interval_secs as i64).saturating_mul(1000);
        refresh_ms > 0
            && last_scan_unix_ms > 0
            && now_unix_ms.saturating_sub(last_scan_unix_ms) <= refresh_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheStamp {
    byte_len: usize,
    content_hash: u64,
}

impl CacheStamp {
    fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self {
            byte_len: bytes.len(),
            content_hash: hasher.finish(),
        }
    }
}

/// Cache for scanned file data
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostUsageCache {
    /// Last scan timestamp in milliseconds
    pub last_scan_unix_ms: i64,
    /// Per-file usage data
    pub files: HashMap<String, CostUsageFileUsage>,
    /// Aggregated daily data: day_key -> model -> [input, cached, output, reasoning?]
    pub days: HashMap<String, HashMap<String, Vec<i32>>>,
    /// Inclusive range covered by the last successful full inspection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_since_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_until_key: Option<String>,
    /// Last validated cost report retained when the persisted cache needs future
    /// catch-up after trimming or expiry. A completed in-memory scan may still
    /// leave this populated when persistence-budget pruning follows; current
    /// publication completeness is carried separately on `CostSummary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_report: Option<CachedCostReport>,
    /// Dirty/incomplete Codex rollouts deferred by the foreground work budget.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codex_pending_paths: Vec<String>,
    /// True while bounded Codex catch-up has not completed for this window.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub codex_scan_incomplete: bool,
    /// Content stamp of the decoded on-disk baseline. This is process-local
    /// and omitted from JSON so a stale reader cannot replace a newer cache.
    #[serde(skip)]
    pub(crate) loaded_stamp: Option<Option<CacheStamp>>,
}

/// Per-file usage tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostUsageFileUsage {
    /// File modification time in milliseconds
    pub mtime_unix_ms: i64,
    /// File size in bytes
    pub size: i64,
    /// Daily usage data extracted from this file
    pub days: HashMap<String, HashMap<String, Vec<i32>>>,
    /// Bytes parsed so far (for incremental parsing)
    pub parsed_bytes: Option<i64>,
    /// Last model seen (for delta calculations)
    pub last_model: Option<String>,
    /// Last token totals (for delta calculations)
    pub last_totals: Option<CodexTotals>,
    /// Whether the parsed Codex token timestamps were non-decreasing.
    ///
    /// `None` is an old cache entry that has never had its timestamp order
    /// validated.  Such an entry must not use the append-only fast path until
    /// a full parse establishes this state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_token_timestamps_monotonic: Option<bool>,
    /// The last parsed Codex token timestamp, used to validate an appended
    /// suffix without replaying the cached prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_last_token_timestamp: Option<String>,
    /// Native Codex session identity from the first authoritative session_meta row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_session_id: Option<String>,
    /// Native Codex parent session identity for forked rollouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_forked_from_id: Option<String>,
    /// Native Codex fork timestamp used for safe parent-baseline validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_fork_timestamp: Option<String>,
    /// True when a fork cannot be billed safely until its parent is available.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub codex_unresolved_fork_parent: bool,
}

/// Lightweight identity metadata read from the first authoritative Codex
/// `session_meta` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CodexSessionMetadata {
    pub session_id: Option<String>,
    pub forked_from_id: Option<String>,
    pub fork_timestamp: Option<String>,
}

/// Running totals for Codex token counting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTotals {
    pub input: i32,
    pub cached: i32,
    pub output: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<i32>,
}

/// Snapshot of the last validated cost report, persisted so spend surfaces keep
/// showing totals while a rescan catches up after the cache is trimmed or the
/// debounce window expires (upstream 0.48.0 #2628). See the cache-budget module
/// for the save/load overshoot contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCostReport {
    /// Total cost in USD for the reported window.
    pub total_cost_usd: f64,
    /// Total input tokens.
    pub input_tokens: i32,
    /// Total cached tokens.
    pub cached_tokens: i32,
    /// Total output tokens.
    pub output_tokens: i32,
    /// Total reasoning output tokens when every contributing packed row knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i32>,
    /// Number of sessions contributing.
    pub sessions_count: i32,
    /// ISO 8601 timestamp when this report was generated.
    pub updated_at: Option<String>,
    /// Whether the report was marked partial (unpriced routing rows retained).
    #[serde(default)]
    pub partial: bool,
}

/// Result of parsing a Codex file
#[derive(Debug)]
pub struct CodexParseResult {
    /// Individual token-count deltas used for per-request pricing.
    pub records: Vec<CodexUsageRecord>,
    /// Bytes parsed
    pub parsed_bytes: i64,
    /// Last model seen
    pub last_model: Option<String>,
    /// Last totals seen
    pub last_totals: Option<CodexTotals>,
    /// Timestamp-order state for the parsed token history.
    pub token_timestamps_monotonic: Option<bool>,
    /// Last token timestamp observed by the parser.
    pub last_token_timestamp: Option<String>,
    /// Number of timestamp comparisons performed while validating this parse.
    pub token_timestamp_comparisons: u64,
    /// Newly consumed bytes in this parse pass.
    pub bytes_read: i64,
    /// Whether this pass reached the file's current EOF without cancellation/budget deferral.
    pub is_complete: bool,
    /// A fork-baseline parse observed a cumulative component below the inherited
    /// parent baseline. The child must be discarded rather than billed as fresh.
    pub fork_baseline_ambiguous: bool,
}

/// A billable Codex token-count delta.
#[derive(Debug, Clone)]
pub struct CodexUsageRecord {
    pub day_key: String,
    pub model: String,
    pub input: i32,
    pub cached: i32,
    pub output: i32,
    pub reasoning: Option<i32>,
}

/// Day range for scanning
pub struct CostUsageDayRange {
    pub since_key: String,
    pub until_key: String,
    pub scan_since_key: String,
    pub scan_until_key: String,
}

impl CostUsageDayRange {
    pub fn new(since: NaiveDate, until: NaiveDate) -> Self {
        let since_minus_one = since - chrono::Duration::days(1);
        let until_plus_one = until + chrono::Duration::days(1);

        Self {
            since_key: Self::day_key(since),
            until_key: Self::day_key(until),
            scan_since_key: Self::day_key(since_minus_one),
            scan_until_key: Self::day_key(until_plus_one),
        }
    }

    pub fn day_key(date: NaiveDate) -> String {
        date.format("%Y-%m-%d").to_string()
    }

    pub fn is_in_range(day_key: &str, since: &str, until: &str) -> bool {
        day_key >= since && day_key <= until
    }

    pub fn parse_day_key(key: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(key, "%Y-%m-%d").ok()
    }
}

/// JSONL Scanner for cost/usage logs
pub struct JsonlScanner;

struct CodexParserState {
    current_model: Option<String>,
    previous_totals: Option<CodexTotals>,
    /// High watermark of observed cumulative totals (never lowered). Used for
    /// Ultra interleaved-lineage containment (issue #2037 Phase 1).
    totals_watermark: Option<CodexTotals>,
    /// Latched once any cumulative component drops below the watermark.
    saw_interleaved_totals: bool,
    records: Vec<CodexUsageRecord>,
    previous_token_timestamp: Option<String>,
    previous_token_timestamp_parsed: Option<DateTime<chrono::FixedOffset>>,
    token_timestamps_monotonic: Option<bool>,
    token_timestamp_comparisons: u64,
    fork_baseline: Option<CodexTotals>,
    fork_baseline_ambiguous: bool,
}

#[derive(Debug, Deserialize)]
struct CodexFastLine<'a> {
    #[serde(rename = "type", borrow)]
    event_type: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
    #[serde(default, borrow)]
    payload: Option<CodexFastPayload<'a>>,
    #[serde(default, borrow)]
    event_msg: Option<CodexFastPayload<'a>>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct CodexFastPayload<'a> {
    #[serde(rename = "type", borrow)]
    payload_type: Option<&'a str>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default, borrow)]
    model_name: Option<&'a str>,
    #[serde(default, borrow)]
    info: Option<CodexFastInfo<'a>>,
    #[serde(default)]
    input_tokens: Option<i32>,
    #[serde(default)]
    cached_input_tokens: Option<i32>,
    #[serde(default)]
    cache_read_input_tokens: Option<i32>,
    #[serde(default)]
    output_tokens: Option<i32>,
    #[serde(default)]
    reasoning_output_tokens: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct CodexFastInfo<'a> {
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default, borrow)]
    model_name: Option<&'a str>,
    #[serde(default)]
    total_token_usage: Option<CodexFastTotals>,
    #[serde(default)]
    last_token_usage: Option<CodexFastTotals>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct CodexFastTotals {
    #[serde(default)]
    input_tokens: i32,
    #[serde(default)]
    cached_input_tokens: Option<i32>,
    #[serde(default)]
    cache_read_input_tokens: Option<i32>,
    #[serde(default)]
    output_tokens: i32,
    #[serde(default)]
    reasoning_output_tokens: Option<i32>,
}

enum CodexFastEvent<'a> {
    TurnContext {
        model: Option<&'a str>,
    },
    TokenCount {
        timestamp: &'a str,
        payload: CodexFastPayload<'a>,
    },
}

impl CodexParserState {
    fn new(initial_model: Option<String>, initial_totals: Option<CodexTotals>) -> Self {
        Self::with_timestamp_state(initial_model, initial_totals, None, None)
    }

    fn with_timestamp_state(
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
    ) -> Self {
        Self::with_timestamp_state_and_fork_mode(
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            false,
        )
    }

    fn with_timestamp_state_and_fork_mode(
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        fork_baseline_mode: bool,
    ) -> Self {
        let previous_token_timestamp_parsed = previous_token_timestamp
            .as_deref()
            .and_then(parse_rfc3339_timestamp);
        let fork_baseline = fork_baseline_mode.then(|| initial_totals.clone()).flatten();
        Self {
            current_model: initial_model,
            previous_totals: initial_totals.clone(),
            totals_watermark: initial_totals,
            saw_interleaved_totals: false,
            records: Vec::new(),
            previous_token_timestamp,
            previous_token_timestamp_parsed,
            // A parser always validates a fresh prefix.  `None` is only an
            // input marker for the legacy-cache path, not an output state.
            token_timestamps_monotonic: Some(token_timestamps_monotonic.unwrap_or(true)),
            token_timestamp_comparisons: 0,
            fork_baseline,
            fork_baseline_ambiguous: false,
        }
    }

    fn process_line(&mut self, line: &str, range: &CostUsageDayRange) {
        let event_candidate = is_candidate_codex_line(line);
        let bare_candidate = !event_candidate && line.contains("\"usage\"");
        if !event_candidate && !bare_candidate {
            return;
        }

        if event_candidate && let Some(event) = parse_codex_fast_event(line) {
            self.process_fast_event(event, range);
            return;
        }

        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            return;
        };

        if bare_candidate {
            if obj.get("type").is_some() {
                return;
            }
            let parsed_timestamp = obj
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_codex_timestamp);
            if let Some(timestamp) = obj.get("timestamp").and_then(Value::as_str) {
                // Timestamp order is a property of the whole native file,
                // including usage records outside the requested day window.
                self.observe_token_timestamp(timestamp, parsed_timestamp.as_ref());
            }
            let day_key = parsed_timestamp
                .as_ref()
                .map(ParsedCodexTimestamp::day_key)
                .filter(|day_key| {
                    CostUsageDayRange::is_in_range(
                        day_key,
                        &range.scan_since_key,
                        &range.scan_until_key,
                    )
                })
                .or_else(|| self.records.last().map(|record| record.day_key.clone()));
            let Some(day_key) = day_key else {
                return;
            };
            if let Some((totals, model)) = bare_usage_totals(&obj) {
                let model = self
                    .current_model
                    .as_deref()
                    .and_then(model_evidence)
                    .or(model.as_deref().and_then(model_evidence))
                    .unwrap_or(CostUsagePricing::CODEX_UNATTRIBUTED_MODEL)
                    .to_string();
                self.record_usage(
                    day_key,
                    &model,
                    totals.input,
                    totals.cached,
                    totals.output,
                    totals.reasoning,
                );
            }
            return;
        }

        let is_token_count = token_count_payload(&obj).is_some();
        let parsed_timestamp = obj
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_codex_timestamp);
        if is_token_count && let Some(timestamp) = obj.get("timestamp").and_then(Value::as_str) {
            // Timestamp order is a property of the whole native file, not
            // only the requested display window. Validate it before the
            // range filter so a cached prefix remains safe to extend.
            self.observe_token_timestamp(timestamp, parsed_timestamp.as_ref());
        }
        let Some(day_key) = parsed_timestamp
            .as_ref()
            .map(ParsedCodexTimestamp::day_key)
            .filter(|day_key| {
                CostUsageDayRange::is_in_range(
                    day_key,
                    &range.scan_since_key,
                    &range.scan_until_key,
                )
            })
        else {
            return;
        };
        if obj.get("type").and_then(|v| v.as_str()) == Some("turn_context") {
            self.update_current_model(&obj);
        }

        if is_token_count {
            self.record_token_count(&obj, day_key);
        }
    }

    fn process_fast_event(&mut self, event: CodexFastEvent<'_>, range: &CostUsageDayRange) {
        match event {
            CodexFastEvent::TurnContext { model } => {
                // Explicit blank model evidence clears stale turn context.
                if let Some(raw) = model {
                    self.current_model = model_evidence(raw).map(str::to_string);
                }
            }
            CodexFastEvent::TokenCount { timestamp, payload } => {
                let parsed_timestamp = parse_codex_timestamp(timestamp);
                self.observe_token_timestamp(timestamp, parsed_timestamp.as_ref());
                let Some(parsed_timestamp) = parsed_timestamp else {
                    return;
                };
                let day_key = parsed_timestamp.day_key();
                if !CostUsageDayRange::is_in_range(
                    &day_key,
                    &range.scan_since_key,
                    &range.scan_until_key,
                ) {
                    return;
                }
                self.record_fast_token_count(payload, day_key);
            }
        }
    }

    fn update_current_model(&mut self, obj: &Value) {
        let candidates = [
            obj.get("model").and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("model"))
                .and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("model_name"))
                .and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("info"))
                .and_then(|info| info.get("model"))
                .and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("info"))
                .and_then(|info| info.get("model_name"))
                .and_then(|v| v.as_str()),
        ];
        // Only rewrite current_model when the turn_context actually carries a
        // model field (including blank, which clears stale attribution).
        let has_key = candidates.iter().any(|c| c.is_some());
        if !has_key {
            return;
        }
        self.current_model = candidates
            .into_iter()
            .flatten()
            .find_map(model_evidence)
            .map(str::to_string);
    }

    fn record_token_count(&mut self, obj: &Value, day_key: String) {
        let Some(payload) = token_count_payload(obj) else {
            return;
        };
        let Some((delta_input, delta_cached, delta_output, reasoning)) = self.token_deltas(payload)
        else {
            return;
        };
        if delta_input == 0 && delta_cached == 0 && delta_output == 0 {
            return;
        }

        let info = payload.get("info");
        let model = self.resolve_token_model(info, payload, obj);
        self.record_usage(
            day_key,
            &model,
            delta_input,
            delta_cached,
            delta_output,
            reasoning,
        );
    }

    fn record_fast_token_count(&mut self, payload: CodexFastPayload<'_>, day_key: String) {
        let Some((delta_input, delta_cached, delta_output, reasoning)) =
            self.fast_token_deltas(&payload)
        else {
            return;
        };
        if delta_input == 0 && delta_cached == 0 && delta_output == 0 {
            return;
        }

        let event_model = payload
            .info
            .as_ref()
            .and_then(|info| info.model.or(info.model_name))
            .or(payload.model)
            .and_then(model_evidence);
        // Prefer current turn_context model over a conflicting event model,
        // matching upstream precedence. Fall back to unattributed (not gpt-5).
        let model = self
            .current_model
            .as_deref()
            .and_then(model_evidence)
            .or(event_model)
            .unwrap_or(CostUsagePricing::CODEX_UNATTRIBUTED_MODEL)
            .to_string();
        self.record_usage(
            day_key,
            &model,
            delta_input,
            delta_cached,
            delta_output,
            reasoning,
        );
    }

    fn record_usage(
        &mut self,
        day_key: String,
        model: &str,
        input: i32,
        cached: i32,
        output: i32,
        reasoning: Option<i32>,
    ) {
        self.records.push(CodexUsageRecord {
            day_key,
            model: CostUsagePricing::normalize_codex_model(model),
            input,
            cached: cached.min(input),
            output,
            reasoning: clamp_reasoning(reasoning, output),
        });
    }

    fn resolve_token_model(&self, info: Option<&Value>, payload: &Value, obj: &Value) -> String {
        let event_model = info
            .and_then(|i| i.get("model").or(i.get("model_name")))
            .or_else(|| payload.get("model"))
            .or_else(|| obj.get("model"))
            .and_then(|v| v.as_str())
            .and_then(model_evidence);
        self.current_model
            .as_deref()
            .and_then(model_evidence)
            .or(event_model)
            .unwrap_or(CostUsagePricing::CODEX_UNATTRIBUTED_MODEL)
            .to_string()
    }

    fn token_deltas(&mut self, payload: &Value) -> Option<(i32, i32, i32, Option<i32>)> {
        let info = payload.get("info");
        if let Some(total) = info.and_then(|i| i.get("total_token_usage")) {
            return Some(self.total_usage_delta(total));
        }

        if let Some(last) = info.and_then(|i| i.get("last_token_usage")) {
            return Some(last_usage_delta(last));
        }

        let direct = read_token_totals(payload);
        (direct.input != 0 || direct.cached != 0 || direct.output != 0).then_some((
            direct.input.max(0),
            direct.cached.max(0),
            direct.output.max(0),
            direct.reasoning,
        ))
    }

    fn fast_token_deltas(
        &mut self,
        payload: &CodexFastPayload<'_>,
    ) -> Option<(i32, i32, i32, Option<i32>)> {
        if let Some(total) = payload
            .info
            .as_ref()
            .and_then(|info| info.total_token_usage)
        {
            return Some(self.fast_total_usage_delta(total));
        }

        if let Some(last) = payload.info.as_ref().and_then(|info| info.last_token_usage) {
            return Some(fast_last_usage_delta(last));
        }

        let direct = fast_totals_from_payload(payload);
        (direct.input != 0 || direct.cached != 0 || direct.output != 0).then_some((
            direct.input.max(0),
            direct.cached.max(0),
            direct.output.max(0),
            direct.reasoning,
        ))
    }

    fn total_usage_delta(&mut self, total: &Value) -> (i32, i32, i32, Option<i32>) {
        let totals = read_token_totals(total);
        self.apply_totals_delta(totals)
    }

    fn fast_total_usage_delta(&mut self, total: CodexFastTotals) -> (i32, i32, i32, Option<i32>) {
        let totals = codex_totals_from_fast(total);
        self.apply_totals_delta(totals)
    }

    fn apply_totals_delta(&mut self, totals: CodexTotals) -> (i32, i32, i32, Option<i32>) {
        self.latch_if_below_watermark(&totals);

        let delta = if self.saw_interleaved_totals {
            contained_total_delta(
                self.totals_watermark.as_ref(),
                self.previous_totals.as_ref(),
                &totals,
            )
        } else {
            let previous = self.previous_totals.as_ref();
            let input = (totals.input - previous.map_or(0, |t| t.input)).max(0);
            let cached = (totals.cached - previous.map_or(0, |t| t.cached)).max(0);
            let output = (totals.output - previous.map_or(0, |t| t.output)).max(0);
            CodexTotals {
                input,
                cached,
                output,
                reasoning: cumulative_reasoning_delta(previous, totals.reasoning, output),
            }
        };

        self.previous_totals = Some(totals.clone());
        self.raise_watermark(&totals);
        (delta.input, delta.cached, delta.output, delta.reasoning)
    }

    fn observe_token_timestamp(
        &mut self,
        timestamp: &str,
        parsed_timestamp: Option<&ParsedCodexTimestamp>,
    ) {
        let current_parsed = parsed_timestamp
            .map(|parsed| parsed.parsed)
            .unwrap_or_else(|| parse_rfc3339_timestamp(timestamp));
        if let Some(previous) = self.previous_token_timestamp.as_deref()
            && self.token_timestamps_monotonic != Some(false)
        {
            self.token_timestamp_comparisons = self.token_timestamp_comparisons.saturating_add(1);
            let ordered = match (
                self.previous_token_timestamp_parsed.as_ref(),
                current_parsed.as_ref(),
            ) {
                (Some(previous), Some(current)) => previous <= current,
                // A malformed historical timestamp keeps the scanner's
                // existing lexical fallback semantics.  The current parsed
                // value is deliberately not reparsed here.
                _ => previous <= timestamp,
            };
            if !ordered {
                self.token_timestamps_monotonic = Some(false);
            }
        }
        self.previous_token_timestamp = Some(timestamp.to_string());
        self.previous_token_timestamp_parsed = current_parsed;
    }

    fn latch_if_below_watermark(&mut self, totals: &CodexTotals) {
        if let Some(baseline) = self.fork_baseline.as_ref()
            && (totals.input < baseline.input
                || totals.cached < baseline.cached
                || totals.output < baseline.output)
        {
            self.fork_baseline_ambiguous = true;
        }
        let Some(water) = self.totals_watermark.as_ref() else {
            return;
        };
        if totals.input < water.input
            || totals.cached < water.cached
            || totals.output < water.output
        {
            self.saw_interleaved_totals = true;
        }
    }

    fn raise_watermark(&mut self, totals: &CodexTotals) {
        self.totals_watermark = Some(match self.totals_watermark.as_ref() {
            Some(water) => CodexTotals {
                input: water.input.max(totals.input),
                cached: water.cached.max(totals.cached),
                output: water.output.max(totals.output),
                reasoning: match (water.reasoning, totals.reasoning) {
                    (Some(water), Some(current)) => Some(water.max(current)),
                    (Some(water), None) => Some(water),
                    (None, Some(current)) => Some(current),
                    (None, None) => None,
                },
            },
            None => totals.clone(),
        });
    }
}

fn model_evidence(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// When interleaved Ultra lineages reset cumulative counters, only count growth
/// above the historical high watermark so rewound branches do not re-add work.
fn contained_total_delta(
    watermark: Option<&CodexTotals>,
    counted: Option<&CodexTotals>,
    current: &CodexTotals,
) -> CodexTotals {
    let water = watermark.cloned().unwrap_or(CodexTotals {
        input: 0,
        cached: 0,
        output: 0,
        reasoning: None,
    });
    let counted = counted.cloned().unwrap_or(CodexTotals {
        input: 0,
        cached: 0,
        output: 0,
        reasoning: None,
    });

    let component = |water: i32, counted: i32, current: i32| -> i32 {
        if current >= water {
            // Only growth above the historical high watermark counts.
            (current - water.max(counted)).max(0)
        } else {
            // Below watermark: rewind / interleaved lineage — do not re-add
            // mid-range climbs that would inflate totals after a fork reset.
            0
        }
    };

    CodexTotals {
        input: component(water.input, counted.input, current.input),
        cached: component(water.cached, counted.cached, current.cached),
        output: component(water.output, counted.output, current.output),
        reasoning: cumulative_reasoning_delta(
            Some(&counted),
            current.reasoning,
            component(water.output, counted.output, current.output),
        ),
    }
}

fn cumulative_reasoning_delta(
    previous: Option<&CodexTotals>,
    current: Option<i32>,
    output_delta: i32,
) -> Option<i32> {
    let current = current?;
    let previous = match previous {
        Some(previous) => previous.reasoning?,
        None => 0,
    };
    Some(
        current
            .saturating_sub(previous)
            .max(0)
            .min(output_delta.max(0)),
    )
}

/// Read one JSONL line, discarding content when it exceeds `max_bytes`.
/// Returns `(line_without_newline, bytes_consumed_including_newline)`.
fn read_bounded_jsonl_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<(Vec<u8>, usize)>> {
    let mut line = Vec::new();
    let mut saw_bytes = false;
    let mut discarding = false;
    let mut consumed_total = 0;

    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(
                saw_bytes.then_some((if discarding { Vec::new() } else { line }, consumed_total))
            );
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let segment_end = newline.unwrap_or(chunk.len());
        let segment = &chunk[..segment_end];
        saw_bytes = true;

        if !discarding {
            let remaining = max_bytes.saturating_sub(line.len());
            if segment.len() <= remaining {
                line.extend_from_slice(segment);
            } else {
                line.clear();
                discarding = true;
            }
        }

        let consumed = segment_end + usize::from(newline.is_some());
        reader.consume(consumed);
        consumed_total += consumed;
        if newline.is_some() {
            return Ok(Some((
                if discarding { Vec::new() } else { line },
                consumed_total,
            )));
        }
    }
}

fn parse_codex_fast_event(line: &str) -> Option<CodexFastEvent<'_>> {
    let parsed: CodexFastLine<'_> = serde_json::from_str(line).ok()?;
    match parsed.event_type? {
        "turn_context" => {
            let model = parsed
                .payload
                .as_ref()
                .and_then(|payload| {
                    payload.model.or(payload.model_name).or_else(|| {
                        payload
                            .info
                            .as_ref()
                            .and_then(|info| info.model.or(info.model_name))
                    })
                })
                .or(parsed.model);
            Some(CodexFastEvent::TurnContext { model })
        }
        "event_msg" => {
            let payload = parsed.payload.or(parsed.event_msg)?;
            (payload.payload_type == Some("token_count")).then_some(CodexFastEvent::TokenCount {
                timestamp: parsed.timestamp?,
                payload,
            })
        }
        _ => None,
    }
}

fn is_candidate_codex_line(line: &str) -> bool {
    if !line.contains("\"type\":\"event_msg\"")
        && !line.contains("\"type\":\"turn_context\"")
        && !line.contains("\"event_msg\"")
    {
        return false;
    }

    !line.contains("\"type\":\"event_msg\"") || line.contains("\"token_count\"")
}

fn codex_timestamp_day_key(timestamp: &str) -> Option<String> {
    parse_codex_timestamp(timestamp).map(|parsed| parsed.day_key())
}

#[derive(Debug, Clone)]
struct ParsedCodexTimestamp {
    parsed: Option<DateTime<FixedOffset>>,
    fallback_day_key: String,
}

impl ParsedCodexTimestamp {
    fn day_key(&self) -> String {
        self.parsed
            .as_ref()
            .map(|timestamp| {
                timestamp
                    .with_timezone(&Local)
                    .date_naive()
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_else(|| self.fallback_day_key.clone())
    }
}

fn parse_codex_timestamp(timestamp: &str) -> Option<ParsedCodexTimestamp> {
    let bytes = timestamp.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let fallback_day_key = timestamp.get(..10)?;
    NaiveDate::parse_from_str(fallback_day_key, "%Y-%m-%d").ok()?;
    Some(ParsedCodexTimestamp {
        parsed: parse_rfc3339_timestamp(timestamp),
        fallback_day_key: fallback_day_key.to_string(),
    })
}

fn parse_rfc3339_timestamp(timestamp: &str) -> Option<DateTime<FixedOffset>> {
    parse_native_rfc3339(timestamp).or_else(|| DateTime::parse_from_rfc3339(timestamp).ok())
}

fn nonempty_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn session_meta_field(root: &Value, payload: Option<&Value>, keys: &[&str]) -> Option<String> {
    payload
        .and_then(|payload| {
            keys.iter()
                .find_map(|key| nonempty_json_string(payload.get(*key)))
        })
        .or_else(|| {
            keys.iter()
                .find_map(|key| nonempty_json_string(root.get(*key)))
        })
}

/// Fast path for the RFC3339 spelling emitted by native Codex logs. Historical
/// spellings still fall through to chrono's parser, preserving old behavior.
fn parse_native_rfc3339(timestamp: &str) -> Option<DateTime<FixedOffset>> {
    let bytes = timestamp.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let year = parse_ascii_number(bytes, 0, 4)?;
    if year < 1900 {
        return None;
    }
    let month = parse_ascii_number(bytes, 5, 2)?;
    let day = parse_ascii_number(bytes, 8, 2)?;
    let hour = parse_ascii_number(bytes, 11, 2)?;
    let minute = parse_ascii_number(bytes, 14, 2)?;
    let second = parse_ascii_number(bytes, 17, 2)?;
    if !(1..=12).contains(&month) || hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }

    let mut zone_index = 19;
    let mut nanoseconds = 0_u32;
    if bytes.get(zone_index) == Some(&b'.') {
        zone_index += 1;
        let fraction_start = zone_index;
        while bytes
            .get(zone_index)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            let digits = zone_index - fraction_start;
            if digits >= 9 {
                return None;
            }
            if digits < 3 {
                nanoseconds = nanoseconds * 10 + u32::from(bytes[zone_index] - b'0');
            }
            zone_index += 1;
        }
        let digits = zone_index - fraction_start;
        if digits == 0 {
            return None;
        }
        for _ in digits.min(3)..3 {
            nanoseconds *= 10;
        }
        for _ in 0..6 {
            nanoseconds *= 10;
        }
    }

    let offset_seconds = match bytes.get(zone_index) {
        Some(b'Z') if zone_index + 1 == bytes.len() => 0,
        Some(sign) if (*sign == b'+' || *sign == b'-') && zone_index + 6 == bytes.len() => {
            if bytes[zone_index + 3] != b':' {
                return None;
            }
            let hours = parse_ascii_number(bytes, zone_index + 1, 2)?;
            let minutes = parse_ascii_number(bytes, zone_index + 4, 2)?;
            if hours >= 24 || minutes >= 60 {
                return None;
            }
            let seconds = i32::try_from((hours * 60 + minutes) * 60).ok()?;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ => return None,
    };

    let year = i32::try_from(year).ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let local = date.and_hms_nano_opt(hour, minute, second, nanoseconds)?;
    FixedOffset::east_opt(offset_seconds)
        .and_then(|offset| offset.from_local_datetime(&local).single())
}

fn parse_ascii_number(bytes: &[u8], start: usize, count: usize) -> Option<u32> {
    let slice = bytes.get(start..start.checked_add(count)?)?;
    let mut value = 0_u32;
    for byte in slice {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(*byte - b'0');
    }
    Some(value)
}

fn bare_usage_totals(obj: &Value) -> Option<(CodexTotals, Option<String>)> {
    let usage = obj
        .get("usage")
        .or_else(|| obj.get("data").and_then(|v| v.get("usage")))
        .or_else(|| obj.get("result").and_then(|v| v.get("usage")))
        .or_else(|| obj.get("response").and_then(|v| v.get("usage")))?;
    // Token counts come from usage records and fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "usage token counts fit i32, the canonical totals storage type"
    )]
    let input = ["input_tokens", "prompt_tokens", "input"]
        .into_iter()
        .find_map(|key| usage.get(key).and_then(Value::as_i64))
        .unwrap_or(0)
        .max(0) as i32;
    // Token counts come from usage records and fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "usage token counts fit i32, the canonical totals storage type"
    )]
    let output = ["output_tokens", "completion_tokens", "output"]
        .into_iter()
        .find_map(|key| usage.get(key).and_then(Value::as_i64))
        .unwrap_or(0)
        .max(0) as i32;
    // Token counts come from usage records and fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "usage token counts fit i32, the canonical totals storage type"
    )]
    let cached = [
        "cached_input_tokens",
        "cache_read_input_tokens",
        "cached_tokens",
    ]
    .into_iter()
    .filter_map(|key| usage.get(key).and_then(Value::as_i64))
    .max()
    .unwrap_or(0)
    .max(0) as i32;
    let reasoning = clamp_reasoning(optional_token_i32(usage, "reasoning_output_tokens"), output);
    if input == 0 && output == 0 && cached == 0 {
        return None;
    }
    let model = obj
        .get("model")
        .or_else(|| obj.get("data").and_then(|v| v.get("model")))
        .or_else(|| obj.get("result").and_then(|v| v.get("model")))
        .or_else(|| obj.get("response").and_then(|v| v.get("model")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    Some((
        CodexTotals {
            input,
            cached,
            output,
            reasoning,
        },
        model,
    ))
}

fn token_count_payload(obj: &Value) -> Option<&Value> {
    if let Some(payload) = obj.get("payload")
        && payload.get("type").and_then(|v| v.as_str()) == Some("token_count")
    {
        return Some(payload);
    }

    let event_msg = obj.get("event_msg")?;
    (event_msg.get("type").and_then(|v| v.as_str()) == Some("token_count")).then_some(event_msg)
}

fn read_token_totals(value: &Value) -> CodexTotals {
    // Token counts come from Codex usage records and fit within i32, which is
    // the canonical storage type of the totals table.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "token counts from usage records fit i32"
    )]
    let cached = value
        .get("cached_input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(
            value
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        ) as i32;
    CodexTotals {
        input: token_i32(value, "input_tokens"),
        cached,
        output: token_i32(value, "output_tokens"),
        reasoning: clamp_reasoning(
            optional_token_i32(value, "reasoning_output_tokens"),
            token_i32(value, "output_tokens"),
        ),
    }
}

fn codex_totals_from_fast(value: CodexFastTotals) -> CodexTotals {
    CodexTotals {
        input: value.input_tokens,
        cached: value
            .cached_input_tokens
            .unwrap_or(0)
            .max(value.cache_read_input_tokens.unwrap_or(0)),
        output: value.output_tokens,
        reasoning: clamp_reasoning(value.reasoning_output_tokens, value.output_tokens),
    }
}

fn fast_totals_from_payload(value: &CodexFastPayload<'_>) -> CodexTotals {
    CodexTotals {
        input: value.input_tokens.unwrap_or(0),
        cached: value
            .cached_input_tokens
            .unwrap_or(0)
            .max(value.cache_read_input_tokens.unwrap_or(0)),
        output: value.output_tokens.unwrap_or(0),
        reasoning: clamp_reasoning(
            value.reasoning_output_tokens,
            value.output_tokens.unwrap_or(0),
        ),
    }
}

fn token_i32(value: &Value, key: &str) -> i32 {
    // Token counts from usage records fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "token counts from usage records fit i32"
    )]
    let tokens = value.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    tokens
}

fn optional_token_i32(value: &Value, key: &str) -> Option<i32> {
    // Token counts from usage records fit i32, the canonical storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "token counts from usage records fit i32"
    )]
    value
        .get(key)
        .and_then(Value::as_i64)
        .map(|tokens| tokens as i32)
}

fn clamp_reasoning(reasoning: Option<i32>, output: i32) -> Option<i32> {
    reasoning.map(|tokens| tokens.max(0).min(output.max(0)))
}

fn last_usage_delta(last: &Value) -> (i32, i32, i32, Option<i32>) {
    let totals = read_token_totals(last);
    (
        totals.input.max(0),
        totals.cached.max(0),
        totals.output.max(0),
        totals.reasoning,
    )
}

fn fast_last_usage_delta(last: CodexFastTotals) -> (i32, i32, i32, Option<i32>) {
    let totals = codex_totals_from_fast(last);
    (
        totals.input.max(0),
        totals.cached.max(0),
        totals.output.max(0),
        totals.reasoning,
    )
}

impl JsonlScanner {
    /// Get default Codex sessions root directory
    pub fn default_codex_sessions_root() -> Option<PathBuf> {
        // Check CODEX_HOME environment variable
        if let Ok(home) = std::env::var("CODEX_HOME") {
            let home = home.trim();
            if !home.is_empty() {
                return Some(PathBuf::from(home).join("sessions"));
            }
        }

        // Default to ~/.codex/sessions
        dirs::home_dir().map(|h| h.join(".codex").join("sessions"))
    }

    /// Get default Claude projects roots
    pub fn default_claude_projects_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();

        // Check CLAUDE_CONFIG_DIR
        if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            let path = PathBuf::from(config_dir.trim()).join("projects");
            if path.exists() {
                roots.push(path);
            }
        }

        // Default locations
        if let Some(home) = dirs::home_dir() {
            let default_path = home.join(".claude").join("projects");
            if default_path.exists() && !roots.contains(&default_path) {
                roots.push(default_path);
            }
        }

        roots
    }

    /// List Codex session files in the given date range
    pub fn list_codex_session_files(
        root: &Path,
        scan_since_key: &str,
        scan_until_key: &str,
    ) -> Vec<PathBuf> {
        let mut files = Vec::new();

        let Some(mut date) = CostUsageDayRange::parse_day_key(scan_since_key) else {
            return files;
        };
        let Some(until_date) = CostUsageDayRange::parse_day_key(scan_until_key) else {
            return files;
        };

        while date <= until_date {
            let year = format!("{:04}", date.year());
            let month = format!("{:02}", date.month());
            let day = format!("{:02}", date.day());

            let day_dir = root.join(&year).join(&month).join(&day);

            if let Ok(entries) = fs::read_dir(&day_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
                    {
                        files.push(path);
                    }
                }
            }

            date += chrono::Duration::days(1);
        }

        files
    }

    /// Read only a bounded prefix until the first authoritative `session_meta`
    /// row is found. Fork decisions must not require parsing the child usage
    /// stream before a safe parent baseline is selected.
    pub(crate) fn read_codex_session_metadata(
        file_path: &Path,
    ) -> std::io::Result<CodexSessionMetadata> {
        let file = File::open(file_path)?;
        let mut reader = BufReader::new(file);
        let mut bytes_examined = 0_usize;

        while bytes_examined < CODEX_JSONL_MAX_LINE_BYTES {
            let Some((line_bytes, consumed)) =
                read_bounded_jsonl_line(&mut reader, CODEX_JSONL_MAX_LINE_BYTES)?
            else {
                break;
            };
            bytes_examined = bytes_examined.saturating_add(consumed);
            if line_bytes.is_empty() {
                continue;
            }
            let Ok(line) = std::str::from_utf8(&line_bytes) else {
                continue;
            };
            let line = line.strip_suffix('\r').unwrap_or(line);
            let Ok(obj) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if obj.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }

            let payload = obj.get("payload").filter(|value| value.is_object());
            return Ok(CodexSessionMetadata {
                session_id: session_meta_field(&obj, payload, &["id", "session_id", "sessionId"]),
                forked_from_id: session_meta_field(
                    &obj,
                    payload,
                    &[
                        "forked_from_id",
                        "forkedFromId",
                        "parent_session_id",
                        "parentSessionId",
                    ],
                ),
                fork_timestamp: nonempty_json_string(obj.get("timestamp")).or_else(|| {
                    payload.and_then(|value| nonempty_json_string(value.get("timestamp")))
                }),
            });
        }

        Ok(CodexSessionMetadata::default())
    }

    /// Compare RFC3339 timestamps using parsed instants. Malformed timestamps
    /// are unsafe for fork-baseline reconciliation and therefore fail closed.
    pub(crate) fn codex_timestamp_at_or_before(earlier: &str, later: &str) -> bool {
        match (
            parse_rfc3339_timestamp(earlier),
            parse_rfc3339_timestamp(later),
        ) {
            (Some(earlier), Some(later)) => earlier <= later,
            _ => false,
        }
    }

    /// Parse a Codex JSONL file
    pub fn parse_codex_file(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state(
            file_path,
            range,
            start_offset,
            initial_model,
            initial_totals,
            None,
            None,
            None,
        )
    }

    /// Parse a Codex file while retaining the timestamp-order state of an
    /// already decoded prefix.  A known prefix only pays for the append
    /// boundary and newly read token events; an unknown legacy prefix is
    /// intentionally rejected by the caller and should be parsed from zero.
    pub fn parse_codex_file_with_state(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        cancel: Option<&AtomicBool>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state_bounded(
            file_path,
            range,
            start_offset,
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            cancel,
            None,
        )
    }

    /// Parse a Codex file with an optional cap on bytes newly consumed this pass.
    /// The reader may finish the current bounded JSONL line before yielding.
    #[allow(
        clippy::too_many_arguments,
        reason = "resume state mirrors the persisted parser cache"
    )]
    pub fn parse_codex_file_with_state_bounded(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        cancel: Option<&AtomicBool>,
        max_bytes_to_read: Option<i64>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state_bounded_internal(
            file_path,
            range,
            start_offset,
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            cancel,
            false,
            max_bytes_to_read,
        )
    }

    /// Parse a forked Codex child from byte zero with a parent cumulative
    /// baseline. This is intentionally separate from ordinary append-resume
    /// parsing so existing non-fork semantics remain unchanged.
    #[allow(
        clippy::too_many_arguments,
        reason = "fork parse state mirrors the persisted parser cache"
    )]
    pub(crate) fn parse_codex_file_with_state_bounded_fork(
        file_path: &Path,
        range: &CostUsageDayRange,
        initial_totals: CodexTotals,
        cancel: Option<&AtomicBool>,
        max_bytes_to_read: Option<i64>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state_bounded_internal(
            file_path,
            range,
            0,
            None,
            Some(initial_totals),
            None,
            None,
            cancel,
            true,
            max_bytes_to_read,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "resume state mirrors the persisted parser cache"
    )]
    fn parse_codex_file_with_state_bounded_internal(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        cancel: Option<&AtomicBool>,
        fork_baseline_mode: bool,
        max_bytes_to_read: Option<i64>,
    ) -> std::io::Result<CodexParseResult> {
        let file = File::open(file_path)?;
        // Session JSONL files are bounded by the cache budget; sizes fit i64.
        #[allow(
            clippy::cast_possible_wrap,
            reason = "session JSONL file sizes fit i64"
        )]
        let file_size = file.metadata()?.len() as i64;

        let mut reader = BufReader::new(file);
        if start_offset > 0 {
            reader.seek(SeekFrom::Start(start_offset as u64))?;
        }

        let mut parser = CodexParserState::with_timestamp_state_and_fork_mode(
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            fork_baseline_mode,
        );
        let mut parsed_bytes = start_offset;
        let mut cancelled = false;
        let mut budget_exhausted = false;

        loop {
            if max_bytes_to_read.is_some_and(|limit| {
                parsed_bytes.saturating_sub(start_offset) >= limit.max(0)
                    && parsed_bytes < file_size
            }) {
                budget_exhausted = true;
                break;
            }
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                break;
            }
            let Some((line_bytes, consumed)) =
                read_bounded_jsonl_line(&mut reader, CODEX_JSONL_MAX_LINE_BYTES)?
            else {
                break;
            };
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                break;
            }
            // Per-line byte counts are capped at 256 KiB, far inside i64::MAX.
            #[allow(
                clippy::cast_possible_wrap,
                reason = "per-line consumed bytes are capped at CODEX_JSONL_MAX_LINE_BYTES"
            )]
            let consumed_i64 = consumed as i64;
            parsed_bytes += consumed_i64;
            if line_bytes.is_empty() {
                continue;
            }
            let Ok(line) = std::str::from_utf8(&line_bytes) else {
                continue;
            };
            let line = line.strip_suffix('\r').unwrap_or(line);
            parser.process_line(line, range);
        }

        let bytes_read = parsed_bytes.saturating_sub(start_offset).max(0);
        let is_complete = !cancelled && !budget_exhausted && parsed_bytes >= file_size;
        Ok(CodexParseResult {
            records: parser.records,
            parsed_bytes: if is_complete {
                file_size.max(parsed_bytes)
            } else {
                parsed_bytes
            },
            last_model: parser.current_model,
            last_totals: parser.previous_totals,
            token_timestamps_monotonic: parser.token_timestamps_monotonic,
            last_token_timestamp: parser.previous_token_timestamp,
            token_timestamp_comparisons: parser.token_timestamp_comparisons,
            bytes_read,
            is_complete,
            fork_baseline_ambiguous: parser.fork_baseline_ambiguous,
        })
    }

    /// F2 (upstream 0.48.0 #2648): whether a cached resume offset sits on a real
    /// line boundary. A partial trailing-line write leaves the cached offset
    /// mid-line; resuming there re-parses from mid-line and corrupts the first
    /// resumed record. Returns  when the byte just before  is
    /// not a newline (or the probe fails), signalling the caller to fall back
    /// to a full re-parse from zero.
    pub fn is_line_boundary_offset(file_path: &Path, offset: i64) -> bool {
        use std::io::{Read, Seek};
        if offset <= 0 {
            return true;
        }
        // Session JSONL file sizes fit i64; metadata feeds only boundary probes.
        #[allow(
            clippy::cast_possible_wrap,
            reason = "session JSONL file sizes fit i64"
        )]
        let file_size_i64 = fs::metadata(file_path).map(|m| m.len() as i64);
        let Ok(file_size) = file_size_i64 else {
            return false;
        };
        if offset >= file_size {
            return true;
        }
        let Ok(mut probe) = File::open(file_path) else {
            return false;
        };
        if probe.seek(SeekFrom::Start((offset - 1) as u64)).is_err() {
            return false;
        }
        let mut prev_byte = [0u8; 1];
        probe.read_exact(&mut prev_byte).is_ok() && prev_byte[0] == b'\n'
    }

    /// Whether a cached scan should be reused under `options` (issue #2089).
    pub fn should_skip_cached_scan(
        cache: &CostUsageCache,
        options: CostScanOptions,
        now_unix_ms: i64,
    ) -> bool {
        options.should_skip_scan(cache.last_scan_unix_ms, now_unix_ms)
    }

    /// Load cache from disk.
    ///
    /// Refuses to decode artifacts larger than the load cap
    /// (`crate::core::CostUsageCacheBudget::MAX_LOAD_BYTES`); an oversized artifact is
    /// cheaper to rebuild bounded than to decode in one shot, so the caller
    /// gets a fresh empty cache instead (upstream 0.48.0 overshoot contract).
    /// Only Codex persistence is bounded; other providers load unbounded.
    pub fn load_cache(provider: ProviderId, cache_root: Option<&Path>) -> CostUsageCache {
        let cache_path = Self::cache_path(provider, cache_root);

        if crate::core::is_bounded_provider(provider) {
            // Artifacts are bounded by MAX_LOAD_BYTES (320 MiB), fitting usize on
            // any supported target even before the budget comparison below.
            #[allow(
                clippy::cast_possible_truncation,
                reason = "bounded artifacts fit usize on any supported target"
            )]
            let file_bytes = crate::core::artifact_file_size(&cache_path) as usize;
            if file_bytes > crate::core::CostUsageCacheBudget::MAX_LOAD_BYTES {
                return CostUsageCache::default();
            }
        }

        if let Ok(contents) = fs::read_to_string(&cache_path)
            && let Ok(mut cache) = serde_json::from_str::<CostUsageCache>(&contents)
        {
            cache.loaded_stamp = Some(Some(CacheStamp::from_bytes(contents.as_bytes())));
            return cache;
        }

        let mut cache = CostUsageCache::default();
        // Track a missing or unreadable baseline separately from a manually
        // constructed cache so a concurrent first writer can invalidate it.
        cache.loaded_stamp = Some(Self::cache_stamp(&cache_path));
        cache
    }

    /// Read only the cache metadata needed by presentation surfaces.
    ///
    /// v0.56.0 performance parity: skip raw per-file scanner state and day
    /// payloads when callers only need stale/catch-up status.
    pub fn load_cache_status(
        provider: ProviderId,
        cache_root: Option<&Path>,
    ) -> CachedCostReadStatus {
        let cache_path = Self::cache_path(provider, cache_root);
        if crate::core::is_bounded_provider(provider) {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "bounded artifacts fit usize on any supported target"
            )]
            let file_bytes = crate::core::artifact_file_size(&cache_path) as usize;
            if file_bytes > crate::core::CostUsageCacheBudget::MAX_LOAD_BYTES {
                return CachedCostReadStatus::default();
            }
        }

        let Ok(file) = File::open(cache_path) else {
            return CachedCostReadStatus::default();
        };
        let Ok(projection) =
            serde_json::from_reader::<_, CachedCostReadStatusProjection>(BufReader::new(file))
        else {
            return CachedCostReadStatus::default();
        };
        CachedCostReadStatus {
            has_days: projection.has_days,
            previous_report: projection.previous_report,
        }
    }
    pub(crate) fn cached_cost_report_from_days(cache: &CostUsageCache) -> CachedCostReport {
        let mut total_cost_usd = 0.0;
        let mut input_tokens = 0_i32;
        let mut cached_tokens = 0_i32;
        let mut output_tokens = 0_i32;
        let mut reasoning_tokens = 0_i32;
        let mut reasoning_known = true;
        let mut partial = false;

        for (day_key, models) in &cache.days {
            let pricing_day = NaiveDate::parse_from_str(day_key, "%Y-%m-%d").ok();
            for (model, values) in models {
                let input = values.first().copied().unwrap_or(0).max(0);
                let cached = values.get(1).copied().unwrap_or(0).max(0);
                let output = values.get(2).copied().unwrap_or(0).max(0);
                input_tokens = input_tokens.saturating_add(input);
                cached_tokens = cached_tokens.saturating_add(cached);
                output_tokens = output_tokens.saturating_add(output);
                if input > 0 || cached > 0 || output > 0 {
                    if let Some(reasoning) = values.get(3).copied() {
                        reasoning_tokens =
                            reasoning_tokens.saturating_add(reasoning.max(0).min(output));
                    } else {
                        reasoning_known = false;
                    }
                }

                if CostUsagePricing::is_codex_unattributed_model(model) {
                    partial = true;
                    continue;
                }
                if !CostUsagePricing::counts_toward_codex_subscription(model) {
                    continue;
                }
                let priced = pricing_day
                    .and_then(|day| {
                        CostUsagePricing::codex_cost_usd_at_date(
                            model,
                            u64::try_from(input).unwrap_or(0),
                            u64::try_from(cached).unwrap_or(0),
                            u64::try_from(output).unwrap_or(0),
                            day,
                        )
                    })
                    .or_else(|| {
                        CostUsagePricing::codex_cost_usd(
                            model,
                            u64::try_from(input).unwrap_or(0),
                            u64::try_from(cached).unwrap_or(0),
                            u64::try_from(output).unwrap_or(0),
                        )
                    });
                if let Some(cost) = priced {
                    total_cost_usd += cost;
                } else {
                    partial = true;
                }
            }
        }

        let sessions_count = i32::try_from(
            cache
                .files
                .values()
                .filter(|usage| !usage.days.is_empty())
                .count(),
        )
        .unwrap_or(i32::MAX);
        CachedCostReport {
            total_cost_usd,
            input_tokens,
            cached_tokens,
            output_tokens,
            reasoning_tokens: reasoning_known.then_some(reasoning_tokens),
            sessions_count,
            updated_at: Some(Utc::now().to_rfc3339()),
            partial,
        }
    }

    /// Merge one Codex record into a packed day/model row. A three-slot row is
    /// deliberately treated as reasoning-unknown, including when a known row
    /// is merged into an existing legacy row.
    pub(crate) fn merge_codex_record_into_packed(packed: &mut Vec<i32>, record: &CodexUsageRecord) {
        let was_empty = packed.is_empty();
        if packed.len() < 3 {
            packed.resize(3, 0);
        }
        packed[0] = packed[0].saturating_add(record.input.max(0));
        packed[1] = packed[1].saturating_add(record.cached.max(0));
        packed[2] = packed[2].saturating_add(record.output.max(0));

        match record.reasoning {
            Some(reasoning) if was_empty => packed.push(reasoning.max(0).min(record.output.max(0))),
            Some(reasoning) if packed.len() >= 4 => {
                packed[3] = packed[3].saturating_add(reasoning.max(0).min(record.output.max(0)));
            }
            Some(_) => {}
            None => packed.truncate(3),
        }
    }

    /// Save cache to disk (temp sibling + copy into place).
    ///
    /// Before encoding, prunes the cache to the persistence budget so the
    /// artifact stays small enough to decode in one shot (upstream 0.48.0
    /// #2637). Only Codex persistence is bounded; the overshoot contract lets
    /// the encoded size exceed `MAX_FILE_BYTES` up to `MAX_LOAD_BYTES`
    /// when protected (partially parsed) entries cannot be trimmed further.
    pub fn save_cache(provider: ProviderId, cache: &mut CostUsageCache, cache_root: Option<&Path>) {
        Self::save_cache_with_limit(
            provider,
            cache,
            cache_root,
            crate::core::CostUsageCacheBudget::MAX_LOAD_BYTES,
        );
    }

    /// Save with an explicit post-encode refusal limit, injected by tests.
    ///
    /// Identical to `save_cache` except the post-encode oversize check uses
    /// `max_load_bytes` rather than the production `MAX_LOAD_BYTES` const.
    /// Production callers MUST use `save_cache`; this helper exists so the
    /// refusal / stale-destination removal can be exercised without encoding a
    /// ~320 MiB test artifact.
    fn save_cache_with_limit(
        provider: ProviderId,
        cache: &mut CostUsageCache,
        cache_root: Option<&Path>,
        max_load_bytes: usize,
    ) {
        let cache_path = Self::cache_path(provider, cache_root);

        // A decoded baseline is only valid for the file contents that produced
        // it. Refuse a stale writer before pruning or creating directories so a
        // concurrent scan remains authoritative.
        if let Some(expected) = cache.loaded_stamp.as_ref()
            && Self::cache_stamp(&cache_path).as_ref() != expected.as_ref()
        {
            return;
        }

        let Some(parent) = cache_path.parent() else {
            return;
        };
        // Best-effort cache dir creation; a missing dir surfaces as the write error below.
        let _dir_created = fs::create_dir_all(parent);

        if crate::core::is_bounded_provider(provider) {
            // v0.55.1 #3051: snapshot the fully validated report BEFORE persistence
            // pruning. If budget trimming creates a catch-up cycle, this is the
            // established spend/tokens users should keep seeing until replacement
            // history finishes, not a zero-cost reconstruction of the trimmed cache.
            let established_report = cache
                .previous_report
                .clone()
                .unwrap_or_else(|| Self::cached_cost_report_from_days(cache));
            let pruned = crate::core::prune_out_of_window_for_budget(
                &mut cache.files,
                &mut cache.days,
                cache.scan_since_key.as_deref(),
                cache.scan_until_key.as_deref(),
                false,
            );
            let estimate = crate::core::estimated_cache_bytes(&cache.files, &cache.days);
            let trimmed = if estimate > crate::core::CostUsageCacheBudget::MAX_FILE_BYTES {
                crate::core::trim_in_window_for_budget(
                    &mut cache.files,
                    &mut cache.days,
                    cache.scan_since_key.as_deref(),
                    cache.scan_until_key.as_deref(),
                    crate::core::CostUsageCacheBudget::MAX_FILE_BYTES,
                )
            } else {
                Vec::new()
            };
            // A16 (upstream 0.48.0): when entries were trimmed for budget, the persisted
            // artifact no longer covers the full window — set previous_report so the
            // next refresh can signal catch-up is pending (and spend surfaces can show
            // the last-validated snapshot during the rescan).
            if (!pruned.is_empty() || !trimmed.is_empty()) && cache.previous_report.is_none() {
                cache.previous_report = Some(established_report);
            }
        }

        let Ok(json) = serde_json::to_string(cache) else {
            return;
        };

        // F19 (upstream 0.48.0): after bounded encode, if the artifact still
        // exceeds MAX_LOAD_BYTES, refuse persistence. Also remove any existing
        // destination artifact so a stale/oversized file cannot persist and
        // trip the load-refusal path on the next scan (which would force an
        // unnecessary full rebuild from a poisoned artifact). This is a
        // one-shot refusal (not a persist/refuse/rebuild loop): the budget
        // enforcement above already pruned and trimmed; if the result is still
        // too large (e.g. a single protected entry exceeds the limit), the
        // artifact is dropped and the next scan rebuilds from scratch.
        if crate::core::is_bounded_provider(provider)
            && crate::core::CostUsageCacheBudget::should_refuse_persistence(
                json.len(),
                max_load_bytes,
            )
        {
            // Best-effort removal; ignore errors (file may not exist).
            let _cleared = fs::remove_file(&cache_path);
            return;
        }

        let tmp_name = format!(
            ".{}.{}-{}.tmp",
            provider.cli_name(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmp_path = parent.join(tmp_name);
        if fs::write(&tmp_path, json.as_bytes()).is_err() {
            return;
        }
        // Recheck after encoding/pruning: another scan may have replaced the
        // destination while this writer was preparing its payload.
        if let Some(expected) = cache.loaded_stamp.as_ref()
            && Self::cache_stamp(&cache_path).as_ref() != expected.as_ref()
        {
            let _removed_tmp = fs::remove_file(&tmp_path);
            return;
        }
        // `copy` replaces an existing target on Windows; prefer it over rename.
        let wrote = if fs::copy(&tmp_path, &cache_path).is_ok() {
            true
        } else {
            // Fallback direct write when copy fails; the copy error already surfaced.
            fs::write(&cache_path, json.as_bytes()).is_ok()
        };
        if wrote {
            cache.loaded_stamp = Some(Some(CacheStamp::from_bytes(json.as_bytes())));
        }
        // Best-effort temp cleanup (ignore errors — unique name avoids clashes).
        let _truncated_tmp = fs::File::create(&tmp_path).and_then(|f| f.set_len(0));
    }

    /// Default on-disk cache root: `%LOCALAPPDATA%\CodexBar` (via `dirs::cache_dir`).
    pub fn default_cache_root() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("CodexBar"))
    }

    fn cache_path(provider: ProviderId, cache_root: Option<&Path>) -> PathBuf {
        let root = cache_root
            .map(|p| p.to_path_buf())
            .or_else(Self::default_cache_root)
            .unwrap_or_else(|| PathBuf::from("."));

        // Mirror upstream layout: {cacheRoot}/cost-usage/{provider}-v1.json
        root.join("cost-usage")
            .join(format!("{}-v1.json", provider.cli_name()))
    }

    fn cache_stamp(cache_path: &Path) -> Option<CacheStamp> {
        fs::read(cache_path)
            .ok()
            .map(|contents| CacheStamp::from_bytes(&contents))
    }

    /// Whether `cache` covers the requested day window (for debounce short-circuit).
    pub fn cache_covers_range(cache: &CostUsageCache, range: &CostUsageDayRange) -> bool {
        match (&cache.scan_since_key, &cache.scan_until_key) {
            (Some(since), Some(until)) => {
                since.as_str() <= range.since_key.as_str()
                    && until.as_str() >= range.until_key.as_str()
            }
            _ => !cache.days.is_empty() || !cache.files.is_empty(),
        }
    }
}

use chrono::Datelike;

#[cfg(test)]
mod tests;
