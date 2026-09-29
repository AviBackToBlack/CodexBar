//! Incremental reader for Codex Priority (Fast) trace evidence.
//!
//! Upstream CodexBar #3820 (v0.65.0). Codex logs every websocket request to
//! `<CODEX_HOME>/logs_2.sqlite`; a `response.create` request with
//! `service_tier == "priority"` marks its turn as Priority. The database is
//! opened read-only (WAL safe, no sidecars created) and scanned incrementally:
//!
//! - the durable [`CodexPriorityTurnsCursor`] records the last examined rowid;
//! - SHA-256 anchors over sampled rows detect a replaced or rewritten
//!   database, and retained source rows are revalidated so retention pruning
//!   inside Codex removes stale turns;
//! - a cold scan runs in rowid chunks so it honors cancellation and keeps its
//!   partial progress.
//!
//! Only ids, model names and timestamps are retained; row bodies contain
//! prompts and are neither stored nor logged.

mod parse;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::core::{
    CODEX_PRIORITY_COMPLETED_MODEL_RETENTION_LIMIT, CodexPriorityCursorAnchor,
    CodexPriorityTurnMetadata, CodexPriorityTurnsCursor, DEFAULT_SQLITE_BUSY_TIMEOUT, JsonlScanner,
    open_readonly_sqlite_connection,
};
use parse::{parse_completed_trace_row, parse_priority_trace_row};

#[cfg(test)]
mod tests;

/// Trace-database file name inside `CODEX_HOME`.
pub(super) const CODEX_TRACE_DATABASE_FILE: &str = "logs_2.sqlite";

/// Rowid span examined per chunk; cancellation is checked between chunks.
const ACCUMULATE_CHUNK_ROWS: i64 = 20_000;
/// Rows per `rowid in (...)` lookup when revalidating retained sources.
const SOURCE_LOOKUP_CHUNK: usize = 500;

const BODY_FILTER: &str = "(feedback_log_body like '%websocket request:%' \
    or feedback_log_body like '%response.completed%' \
    or feedback_log_body like '%service_tier: Some(Some(\"priority\"))%')";

/// Result of one trace resolution. `cursor` is always the best-known state,
/// so a failed or interrupted scan never drops evidence already collected.
#[derive(Debug, Clone)]
pub(super) struct PriorityTraceResolution {
    pub(super) cursor: Option<CodexPriorityTurnsCursor>,
    /// True when the database could not be fully validated this pass; the
    /// previous evidence stays in force and the next scan retries.
    pub(super) validation_pending: bool,
}

impl PriorityTraceResolution {
    fn keep(cursor: Option<CodexPriorityTurnsCursor>, validation_pending: bool) -> Self {
        Self {
            cursor,
            validation_pending,
        }
    }
}

/// Resolve Priority turns from `database_path`, continuing `previous` when it
/// is still valid for this database.
pub(super) fn resolve_priority_turns(
    database_path: &Path,
    previous: Option<CodexPriorityTurnsCursor>,
    coverage_since_epoch: i64,
    cancel: Option<&AtomicBool>,
) -> PriorityTraceResolution {
    let path_key = database_path.to_string_lossy().to_string();
    // A cursor for a different database is never reused.
    let previous = previous.filter(|cursor| cursor.database_path == path_key);

    let Ok(metadata) = std::fs::metadata(database_path) else {
        // A missing optional source is normal until this path has supplied
        // evidence; once it has, the evidence stays but validation is pending.
        let pending = previous.is_some();
        return PriorityTraceResolution::keep(previous, pending);
    };
    let Some(identity) = JsonlScanner::codex_file_identity(database_path, &metadata) else {
        return PriorityTraceResolution::keep(previous, true);
    };
    let conn = match open_readonly_sqlite_connection(database_path, DEFAULT_SQLITE_BUSY_TIMEOUT) {
        Ok(conn) => conn,
        Err(error) => {
            tracing::debug!(%error, "Codex trace database could not be opened");
            return PriorityTraceResolution::keep(previous, true);
        }
    };
    // The file may have been replaced between the identity read and the open.
    let identity_after = std::fs::metadata(database_path)
        .ok()
        .and_then(|metadata| JsonlScanner::codex_file_identity(database_path, &metadata));
    if identity_after.as_deref() != Some(identity.as_str()) {
        return PriorityTraceResolution::keep(previous, true);
    }
    let Some(max_row_id) = max_logs_row_id(&conn) else {
        tracing::debug!("Codex trace database has no readable logs table");
        return PriorityTraceResolution::keep(previous, true);
    };

    let fresh = || CodexPriorityTurnsCursor {
        database_path: path_key.clone(),
        coverage_since_epoch,
        file_identity: Some(identity.clone()),
        ..CodexPriorityTurnsCursor::default()
    };

    let mut state = previous.clone().filter(|cursor| {
        max_row_id >= cursor.last_row_id
            && coverage_since_epoch >= cursor.coverage_since_epoch
            && cursor.file_identity.as_deref() == Some(identity.as_str())
    });
    let mut anchors_need_refresh = false;
    if let Some(cursor) = &state {
        match validate_anchors(&conn, cursor) {
            AnchorValidation::Valid { needs_refresh } => anchors_need_refresh = needs_refresh,
            AnchorValidation::Invalid => state = None,
        }
    }

    let had_state = state.is_some();
    let mut resolved = state.unwrap_or_else(fresh);
    let mut failed_replacement_fallback = None;
    if had_state {
        let mut pruned = resolved.clone();
        if prune_deleted_sources(&conn, &mut pruned).is_some() {
            resolved = pruned;
        } else {
            // A retained source changed or could not be checked. Rebuild
            // rather than publish evidence a cold scan might not reproduce,
            // but keep the prior state as the failure result.
            failed_replacement_fallback = Some(resolved);
            resolved = fresh();
            anchors_need_refresh = false;
        }
    }

    if max_row_id > resolved.last_row_id {
        let mut updated = resolved.clone();
        match accumulate(&conn, &mut updated, max_row_id, cancel) {
            Accumulation::Complete => {}
            Accumulation::Failed => {
                return PriorityTraceResolution::keep(
                    failed_replacement_fallback.or(Some(resolved)),
                    true,
                );
            }
            Accumulation::Cancelled => {
                // Keep partial progress; the next scan continues from it.
                if updated.last_row_id == 0 {
                    return PriorityTraceResolution::keep(
                        failed_replacement_fallback.or(Some(resolved)),
                        true,
                    );
                }
                if !capture_anchors(&conn, &mut updated) {
                    updated.anchors.clear();
                }
                return PriorityTraceResolution::keep(Some(updated), true);
            }
        }
        updated.last_row_id = max_row_id;
        let anchored = capture_anchors(&conn, &mut updated);
        return PriorityTraceResolution::keep(Some(updated), !anchored);
    }
    if anchors_need_refresh {
        let mut updated = resolved.clone();
        let anchored = capture_anchors(&conn, &mut updated);
        return PriorityTraceResolution::keep(
            Some(if anchored { updated } else { resolved }),
            !anchored,
        );
    }
    PriorityTraceResolution::keep(Some(resolved), false)
}

/// Best-effort rollback; the read-only connection has nothing to lose.
fn rollback(conn: &Connection) {
    if let Err(error) = conn.execute_batch("rollback") {
        tracing::debug!(%error, "Codex trace database rollback failed");
    }
}

fn max_logs_row_id(conn: &Connection) -> Option<i64> {
    conn.query_row("select max(rowid) from logs", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .ok()
    .map(|value| value.unwrap_or(0))
}

fn has_timestamp_index(conn: &Connection) -> bool {
    conn.query_row(
        "select 1 from sqlite_master where type = 'index' and tbl_name = 'logs' \
         and name = 'idx_logs_ts' limit 1",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_text(row: &rusqlite::Row<'_>, index: usize) -> Option<String> {
    match row.get_ref(index).ok()? {
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        _ => None,
    }
}

fn column_timestamp(row: &rusqlite::Row<'_>, index: usize) -> Option<i64> {
    match row.get_ref(index).ok()? {
        ValueRef::Integer(value) => Some(value),
        // Truncation to whole seconds is intended for a fractional timestamp.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "whole-second trace timestamp"
        )]
        ValueRef::Real(value) => Some(value as i64),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).ok()?.trim().parse().ok(),
        _ => None,
    }
}

enum Accumulation {
    Complete,
    Cancelled,
    Failed,
}

/// Examine rows in `(state.last_row_id, max_row_id]`, one rowid chunk at a
/// time. `state.last_row_id` advances only after a chunk fully succeeds.
fn accumulate(
    conn: &Connection,
    state: &mut CodexPriorityTurnsCursor,
    max_row_id: i64,
    cancel: Option<&AtomicBool>,
) -> Accumulation {
    let mut from = state.last_row_id;
    if from == 0 && state.coverage_since_epoch > 0 && has_timestamp_index(conn) {
        // No matching row can precede the first row inside the window, so a
        // cold scan skips the older history without reading its bodies.
        let first = conn
            .query_row(
                "select min(rowid) from logs indexed by idx_logs_ts where ts >= ?1",
                [state.coverage_since_epoch],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok();
        match first {
            Some(Some(first)) => from = first.saturating_sub(1).max(0),
            Some(None) => {
                state.last_row_id = max_row_id;
                return Accumulation::Complete;
            }
            None => return Accumulation::Failed,
        }
    }
    let query = format!(
        "select rowid, ts, feedback_log_body from logs \
         where rowid > ?1 and rowid <= ?2 and ts >= ?3 and {BODY_FILTER} order by rowid"
    );
    let Ok(mut statement) = conn.prepare(&query) else {
        return Accumulation::Failed;
    };
    while from < max_row_id {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Accumulation::Cancelled;
        }
        let to = from.saturating_add(ACCUMULATE_CHUNK_ROWS).min(max_row_id);
        let Ok(mut rows) = statement.query(params![from, to, state.coverage_since_epoch]) else {
            return Accumulation::Failed;
        };
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let row_id = row.get::<_, i64>(0).unwrap_or(0);
                    let timestamp = column_timestamp(row, 1);
                    if let Some(body) = column_text(row, 2) {
                        absorb_row(state, row_id, timestamp, &body);
                    }
                }
                Ok(None) => break,
                Err(_) => return Accumulation::Failed,
            }
        }
        state.last_row_id = to;
        from = to;
    }
    Accumulation::Complete
}

/// Fold one trace row into the cursor state.
fn absorb_row(
    state: &mut CodexPriorityTurnsCursor,
    row_id: i64,
    timestamp: Option<i64>,
    body: &str,
) {
    if let Some(completed) = parse_completed_trace_row(body) {
        if state.turns.contains_key(&completed.turn_id) {
            state
                .priority_completed_models
                .entry(completed.turn_id)
                .or_default()
                .insert(row_id, completed.model);
        } else {
            store_pending_completed_models(
                state,
                &completed.turn_id,
                BTreeMap::from([(row_id, completed.model)]),
            );
        }
        return;
    }
    let Some(parsed) = parse_priority_trace_row(timestamp, body) else {
        return;
    };
    let turn_id = parsed.turn_id.clone();
    state.turns.insert(turn_id.clone(), parsed.clone());
    state
        .request_sources
        .entry(turn_id.clone())
        .or_default()
        .insert(row_id, parsed);
    if let Some(models) = state.completed_models.remove(&turn_id) {
        state.completed_order.retain(|id| id != &turn_id);
        state.priority_completed_models.insert(turn_id, models);
    }
}

/// Retain completions that arrived before their request. The map is bounded:
/// once over the limit the oldest turn's models are evicted.
fn store_pending_completed_models(
    state: &mut CodexPriorityTurnsCursor,
    turn_id: &str,
    models: BTreeMap<i64, String>,
) {
    if !state.completed_models.contains_key(turn_id) {
        state.completed_order.push(turn_id.to_string());
        if state.completed_order.len() > CODEX_PRIORITY_COMPLETED_MODEL_RETENTION_LIMIT {
            let evicted = state.completed_order.remove(0);
            state.completed_models.remove(&evicted);
        }
    }
    state
        .completed_models
        .entry(turn_id.to_string())
        .or_default()
        .extend(models);
}

enum AnchorValidation {
    Valid { needs_refresh: bool },
    Invalid,
}

enum AnchorLookup {
    Found(String),
    Missing,
    Failed,
}

fn anchor_digest(timestamp: i64, body: Option<&[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{timestamp}\n").as_bytes());
    if let Some(body) = body {
        hasher.update(body);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn row_anchor_digest(row: &rusqlite::Row<'_>, timestamp_index: usize, body_index: usize) -> String {
    let timestamp = match row.get_ref(timestamp_index) {
        Ok(ValueRef::Integer(value)) => value,
        _ => 0,
    };
    let body = match row.get_ref(body_index) {
        Ok(ValueRef::Text(bytes) | ValueRef::Blob(bytes)) => Some(bytes),
        _ => None,
    };
    anchor_digest(timestamp, body)
}

/// Sample four rows across the scanned range (a quarter, half, three
/// quarters and the last) so a rewritten database cannot look unchanged.
fn capture_anchors(conn: &Connection, state: &mut CodexPriorityTurnsCursor) -> bool {
    if state.last_row_id <= 0 {
        state.anchors.clear();
        return true;
    }
    if conn.execute_batch("begin deferred transaction").is_err() {
        return false;
    }
    let anchors = sample_anchors(conn, state.last_row_id);
    let committed = conn.execute_batch("commit").is_ok();
    if !committed {
        rollback(conn);
    }
    match anchors {
        Some(anchors) if committed && !anchors.is_empty() => {
            state.anchors = anchors;
            true
        }
        _ => false,
    }
}

fn sample_anchors(conn: &Connection, last_row_id: i64) -> Option<Vec<CodexPriorityCursorAnchor>> {
    let minimum = conn
        .query_row(
            "select min(rowid) from logs where rowid <= ?1",
            [last_row_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()??;
    let span = last_row_id - minimum;
    let targets = [
        minimum + span / 4,
        minimum + span / 2,
        minimum + (span / 4) * 3 + (span % 4) * 3 / 4,
        last_row_id,
    ];
    let mut anchors: Vec<CodexPriorityCursorAnchor> = Vec::new();
    for target in targets {
        let anchor = conn
            .query_row(
                "select rowid, ts, feedback_log_body from logs where rowid <= ?1 \
                 order by rowid desc limit 1",
                [target],
                |row| {
                    Ok(CodexPriorityCursorAnchor {
                        row_id: row.get(0)?,
                        digest: row_anchor_digest(row, 1, 2),
                    })
                },
            )
            .ok()?;
        if !anchors.iter().any(|known| known.row_id == anchor.row_id) {
            anchors.push(anchor);
        }
    }
    Some(anchors)
}

fn lookup_anchor(conn: &Connection, row_id: i64) -> AnchorLookup {
    let result = conn.query_row(
        "select ts, feedback_log_body from logs where rowid = ?1",
        [row_id],
        |row| Ok(row_anchor_digest(row, 0, 1)),
    );
    match result {
        Ok(digest) => AnchorLookup::Found(digest),
        Err(rusqlite::Error::QueryReturnedNoRows) => AnchorLookup::Missing,
        Err(_) => AnchorLookup::Failed,
    }
}

/// Check that the sampled rows still hold what was recorded. Codex prunes old
/// rows in place, so a missing anchor only asks for a refresh; a changed one
/// means the database was rewritten.
fn validate_anchors(conn: &Connection, state: &CodexPriorityTurnsCursor) -> AnchorValidation {
    if state.anchors.is_empty() {
        return if state.last_row_id == 0 {
            AnchorValidation::Valid {
                needs_refresh: true,
            }
        } else {
            AnchorValidation::Invalid
        };
    }
    if conn.execute_batch("begin deferred transaction").is_err() {
        return AnchorValidation::Invalid;
    }
    let (mut matching, mut missing) = (0_usize, 0_usize);
    let (mut failed, mut mismatched) = (false, false);
    for anchor in &state.anchors {
        match lookup_anchor(conn, anchor.row_id) {
            AnchorLookup::Found(digest) if digest == anchor.digest => matching += 1,
            AnchorLookup::Found(_) => mismatched = true,
            AnchorLookup::Missing => missing += 1,
            AnchorLookup::Failed => failed = true,
        }
    }
    if conn.execute_batch("commit").is_err() {
        rollback(conn);
        return AnchorValidation::Invalid;
    }
    if failed || mismatched {
        return AnchorValidation::Invalid;
    }
    // With fewer than four distinct rows there is no distributed quorum.
    let required = if state.anchors.len() < 4 {
        state.anchors.len()
    } else {
        2
    };
    if matching < required {
        return AnchorValidation::Invalid;
    }
    AnchorValidation::Valid {
        needs_refresh: missing > 0,
    }
}

/// Drop turns whose source rows Codex has since deleted. Returns `None` when
/// a retained source no longer matches (rebuild required) or cannot be read.
fn prune_deleted_sources(conn: &Connection, state: &mut CodexPriorityTurnsCursor) -> Option<bool> {
    let retained = retained_source_row_ids(conn, state)?;
    let mut pruned = false;

    let turn_ids: Vec<String> = state.request_sources.keys().cloned().collect();
    for turn_id in turn_ids {
        let Some(sources) = state.request_sources.get(&turn_id) else {
            continue;
        };
        let kept: BTreeMap<i64, CodexPriorityTurnMetadata> = sources
            .iter()
            .filter(|(row_id, _)| retained.contains(row_id))
            .map(|(row_id, meta)| (*row_id, meta.clone()))
            .collect();
        if kept.len() == sources.len() {
            continue;
        }
        pruned = true;
        if kept.is_empty() {
            state.request_sources.remove(&turn_id);
            state.turns.remove(&turn_id);
            if let Some(models) = state.priority_completed_models.remove(&turn_id) {
                store_pending_completed_models(state, &turn_id, models);
            }
        } else {
            if let Some((_, latest)) = kept.iter().next_back() {
                state.turns.insert(turn_id.clone(), latest.clone());
            }
            state.request_sources.insert(turn_id, kept);
        }
    }
    pruned |= prune_completed(&retained, &mut state.priority_completed_models);
    pruned |= prune_completed(&retained, &mut state.completed_models);
    let completed = &state.completed_models;
    state
        .completed_order
        .retain(|id| completed.contains_key(id));
    Some(pruned)
}

fn prune_completed(
    retained: &HashSet<i64>,
    models: &mut HashMap<String, BTreeMap<i64, String>>,
) -> bool {
    let mut pruned = false;
    models.retain(|_, by_row| {
        let before = by_row.len();
        by_row.retain(|row_id, _| retained.contains(row_id));
        pruned |= by_row.len() != before;
        !by_row.is_empty()
    });
    pruned
}

/// Row ids of retained sources that still parse to the recorded evidence.
fn retained_source_row_ids(
    conn: &Connection,
    state: &CodexPriorityTurnsCursor,
) -> Option<HashSet<i64>> {
    let mut requests: HashMap<i64, &CodexPriorityTurnMetadata> = HashMap::new();
    for sources in state.request_sources.values() {
        for (row_id, meta) in sources {
            if requests.get(row_id).is_some_and(|known| *known != meta) {
                return None;
            }
            requests.insert(*row_id, meta);
        }
    }
    let mut completions: HashMap<i64, (&str, &str)> = HashMap::new();
    for by_turn in [&state.priority_completed_models, &state.completed_models] {
        for (turn_id, by_row) in by_turn {
            for (row_id, model) in by_row {
                let entry = (turn_id.as_str(), model.as_str());
                if completions.get(row_id).is_some_and(|known| *known != entry) {
                    return None;
                }
                completions.insert(*row_id, entry);
            }
        }
    }
    if requests
        .keys()
        .any(|row_id| completions.contains_key(row_id))
    {
        return None;
    }
    let row_ids: Vec<i64> = requests
        .keys()
        .chain(completions.keys())
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut retained = HashSet::new();
    for chunk in row_ids.chunks(SOURCE_LOOKUP_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let query = format!(
            "select rowid, ts, feedback_log_body from logs where rowid in ({placeholders})"
        );
        let mut statement = conn.prepare(&query).ok()?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(chunk.iter()))
            .ok()?;
        while let Some(row) = rows.next().ok()? {
            let row_id: i64 = row.get(0).ok()?;
            let body = column_text(row, 2)?;
            if let Some(expected) = requests.get(&row_id) {
                let parsed = parse_priority_trace_row(column_timestamp(row, 1), &body)?;
                if &parsed != *expected {
                    return None;
                }
            } else if let Some((turn_id, model)) = completions.get(&row_id) {
                let parsed = parse_completed_trace_row(&body)?;
                if parsed.turn_id != *turn_id || parsed.model != *model {
                    return None;
                }
            } else {
                return None;
            }
            retained.insert(row_id);
        }
    }
    Some(retained)
}
