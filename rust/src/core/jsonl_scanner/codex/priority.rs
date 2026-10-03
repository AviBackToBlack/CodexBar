//! Persisted state for Codex Priority (Fast) trace evidence.
//!
//! Codex records each websocket request in `<CODEX_HOME>/logs_2.sqlite`. A
//! request carrying `service_tier == "priority"` marks its whole turn as a
//! Priority turn, but plain `gpt-5.x` session rows never say so. The scanner
//! reads the trace database incrementally (see `cost_scanner::codex::priority_trace`)
//! and stores only ids, model names and timestamps here; row bodies contain
//! prompts and are never persisted.
//!
//! Priority is applied as an overlay when day totals are rebuilt. It is never
//! baked into the persisted source-row pricing evidence, so cached-price
//! recovery keeps comparing like with like.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::CostUsagePricing;

/// Completions seen before their request may belong to non-priority turns, so
/// that pending map is bounded to keep memory and cache size constant.
pub const CODEX_PRIORITY_COMPLETED_MODEL_RETENTION_LIMIT: usize = 4096;

/// Evidence for one Priority turn read from a trace row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexPriorityTurnMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Trace row timestamp in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

/// Digest of one trace row, used to detect a replaced or rewritten database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexPriorityCursorAnchor {
    pub row_id: i64,
    pub digest: String,
}

/// Durable scan cursor for one trace database.
///
/// Every map is ordered so the serialized cursor is byte-stable across
/// processes; an unchanged cursor must not look like a cache change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexPriorityTurnsCursor {
    pub database_path: String,
    /// Earliest `ts` (Unix seconds) the accumulated evidence covers.
    pub coverage_since_epoch: i64,
    /// Highest `logs.rowid` already examined. Rowids are monotonic.
    pub last_row_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<CodexPriorityCursorAnchor>,
    /// Source trace rows behind each Priority turn, keyed by rowid.
    #[serde(default)]
    pub request_sources: BTreeMap<String, BTreeMap<i64, CodexPriorityTurnMetadata>>,
    /// `response.completed` models for known Priority turns, keyed by rowid.
    #[serde(default)]
    pub priority_completed_models: BTreeMap<String, BTreeMap<i64, String>>,
    /// Completions seen before their request (bounded, oldest evicted first).
    #[serde(default)]
    pub completed_models: BTreeMap<String, BTreeMap<i64, String>>,
    #[serde(default)]
    pub completed_order: VecDeque<String>,
}

/// A borrowed view that prices session rows of one file as Priority.
pub(crate) struct CodexPriorityOverlay<'a> {
    cursor: &'a CodexPriorityTurnsCursor,
}

impl CodexPriorityTurnsCursor {
    /// The overlay for a session file, or `None` when the file lives outside
    /// the Codex home this trace database belongs to. Turn sets are never
    /// shared across `CODEX_HOME` scopes.
    pub(crate) fn overlay_for_file(&self, file_path: &Path) -> Option<CodexPriorityOverlay<'_>> {
        if self.request_sources.is_empty() {
            return None;
        }
        let home = Path::new(&self.database_path).parent()?;
        path_is_under(file_path, home).then_some(CodexPriorityOverlay { cursor: self })
    }

    /// Latest retained Priority request for `turn_id`.
    pub(crate) fn turn(&self, turn_id: &str) -> Option<&CodexPriorityTurnMetadata> {
        self.request_sources.get(turn_id)?.values().next_back()
    }

    /// Model evidence for a Priority turn: the latest completion, else the
    /// request model.
    pub(crate) fn turn_model(&self, turn_id: &str) -> Option<&str> {
        self.priority_completed_models
            .get(turn_id)
            .and_then(|models| models.values().next_back())
            .map(String::as_str)
            .or_else(|| self.turn(turn_id)?.model.as_deref())
    }
}

impl CodexPriorityOverlay<'_> {
    /// The `<model>-priority` name a row prices under, when its turn is a
    /// Priority turn and the model has a Fast lane. A model without a Fast
    /// lane keeps Standard pricing, as upstream does.
    pub(crate) fn priority_model(&self, turn_id: Option<&str>, row_model: &str) -> Option<String> {
        let turn_id = turn_id?;
        self.cursor.turn(turn_id)?;
        if row_model.ends_with("-priority") {
            return None;
        }
        let priced = self
            .cursor
            .turn_model(turn_id)
            .filter(|model| CostUsagePricing::codex_api_fast_multiplier(model).is_some())
            .unwrap_or(row_model);
        CostUsagePricing::codex_api_fast_multiplier(priced)?;
        Some(format!(
            "{}-priority",
            CostUsagePricing::codex_fast_base_model(priced)
        ))
    }
}

fn path_is_under(path: &Path, root: &Path) -> bool {
    let normalize = |value: &Path| value.to_string_lossy().replace('\\', "/").to_lowercase();
    let root = normalize(root);
    let root = root.trim_end_matches('/');
    let path = normalize(path);
    path.strip_prefix(root)
        .is_some_and(|rest| rest.starts_with('/'))
}
