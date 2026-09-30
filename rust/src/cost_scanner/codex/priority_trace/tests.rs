//! Tests for the Priority trace resolver and its scanner integration.

use super::*;
use crate::core::CostUsagePricing;
use crate::cost_scanner::{CostScanOptions, CostScanner};
use chrono::Local;
use std::path::PathBuf;

mod scanner;

fn request_body(turn: &str, tier: &str, model: &str) -> String {
    format!(
        "session_loop{{thread_id=thread-1}}:turn{{turn.id={turn}}}: \
         codex_api::endpoint::responses_websocket: websocket request: \
         {{\"type\":\"response.create\",\"model\":\"{model}\",\"service_tier\":\"{tier}\"}}"
    )
}

fn completed_body(turn: &str, model: &str) -> String {
    format!(
        "session_loop{{thread_id=thread-1}}:turn{{turn.id={turn}}}: \
         codex_api::endpoint::responses_websocket: websocket event: \
         {{\"type\":\"response.completed\",\"response\":{{\"model\":\"{model}\"}}}}"
    )
}

struct TraceDb {
    _dir: Option<tempfile::TempDir>,
    path: PathBuf,
}

impl TraceDb {
    fn new(rows: &[(i64, String)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Self::in_dir(dir.path(), rows);
        db._dir = Some(dir);
        db
    }

    /// A trace database inside an existing Codex home directory.
    fn in_dir(home: &Path, rows: &[(i64, String)]) -> Self {
        let path = home.join(CODEX_TRACE_DATABASE_FILE);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "create table logs (id integer primary key autoincrement, ts integer not null, \
             feedback_log_body text); create index idx_logs_ts on logs(ts);",
        )
        .unwrap();
        let db = Self { _dir: None, path };
        db.insert(rows);
        db
    }

    /// Insert rows in one transaction, like upstream `insertTestLogs`.
    fn insert(&self, rows: &[(i64, String)]) {
        let mut conn = Connection::open(&self.path).unwrap();
        let transaction = conn.transaction().unwrap();
        for (ts, body) in rows {
            transaction
                .execute(
                    "insert into logs (ts, feedback_log_body) values (?1, ?2)",
                    params![ts, body],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    fn execute(&self, sql: &str) {
        Connection::open(&self.path)
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}

fn resolve(db: &TraceDb, previous: Option<CodexPriorityTurnsCursor>) -> PriorityTraceResolution {
    resolve_priority_turns(&db.path, previous, 0, false, None)
}

fn noise(count: usize) -> Vec<(i64, String)> {
    (0..count)
        .map(|index| {
            (
                1_000 + i64::try_from(index).unwrap(),
                format!("unrelated log line {index}"),
            )
        })
        .collect()
}

#[test]
fn parses_priority_request_with_turn_thread_and_model() {
    let parsed =
        parse_priority_trace_row(Some(42), &request_body("turn-1", "priority", "gpt-5.5")).unwrap();
    assert_eq!(parsed.turn_id, "turn-1");
    assert_eq!(parsed.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(parsed.timestamp, Some(42));
}

#[test]
fn ignores_standard_requests_and_other_event_types() {
    assert!(parse_priority_trace_row(None, &request_body("t", "default", "gpt-5.5")).is_none());
    let create = request_body("t", "priority", "gpt-5.5").replace("response.create", "response.x");
    assert!(parse_priority_trace_row(None, &create).is_none());
    assert!(parse_priority_trace_row(None, "plain log").is_none());
}

#[test]
fn request_turn_id_falls_back_to_the_json_body() {
    let body = "websocket request: {\"type\":\"response.create\",\"service_tier\":\"priority\",\
                \"turn_id\":\"json-turn\"}";
    let parsed = parse_priority_trace_row(None, body).unwrap();
    assert_eq!(parsed.turn_id, "json-turn");
    let without = "websocket request: {\"type\":\"response.create\",\"service_tier\":\"priority\"}";
    assert!(parse_priority_trace_row(None, without).is_none());
}

#[test]
fn parses_priority_submission_rows() {
    let body = "thread_id=thread-9: Submission sub=Submission { id: \"sub-turn\", \
                op: UserTurn { service_tier: Some(Some(\"priority\")) } }";
    let parsed = parse_priority_trace_row(Some(7), body).unwrap();
    assert_eq!(parsed.turn_id, "sub-turn");
    assert_eq!(parsed.thread_id.as_deref(), Some("thread-9"));
    assert_eq!(parsed.model, None);
    let standard = body.replace("priority", "flex");
    assert!(parse_priority_trace_row(None, &standard).is_none());
}

#[test]
fn parses_completed_events() {
    let parsed = parse_completed_trace_row(&completed_body("turn-1", "gpt-5.5")).unwrap();
    assert_eq!(parsed.turn_id, "turn-1");
    assert_eq!(parsed.model, "gpt-5.5");
    let other = completed_body("turn-1", "gpt-5.5").replace("response.completed", "response.x");
    assert!(parse_completed_trace_row(&other).is_none());
}

#[test]
fn request_fields_of_another_type_are_absent_not_fatal() {
    // Upstream casts each field with `as? String`: a numeric model is absent,
    // but the turn is still a Priority turn.
    let numeric_model = "turn.id=t1 websocket request: \
         {\"type\":\"response.create\",\"model\":5,\"service_tier\":\"priority\"}";
    let parsed = parse_priority_trace_row(None, numeric_model).unwrap();
    assert_eq!(parsed.turn_id, "t1");
    assert_eq!(parsed.model, None);

    let numeric_tier = "turn.id=t1 websocket request: \
         {\"type\":\"response.create\",\"model\":\"gpt-5.5\",\"service_tier\":1}";
    assert!(parse_priority_trace_row(None, numeric_tier).is_none());

    // Without the object guard serde would read an array positionally.
    let array = "turn.id=t1 websocket request: \
         [\"response.create\",\"priority\",\"t1\",\"gpt-5.5\"]";
    assert!(parse_priority_trace_row(None, array).is_none());
}

#[test]
fn completed_event_without_an_object_response_is_ignored() {
    let string_response = "turn.id=t1 websocket event: \
         {\"type\":\"response.completed\",\"response\":\"gpt-5.5\"}";
    assert!(parse_completed_trace_row(string_response).is_none());

    let numeric_model = "turn.id=t1 websocket event: \
         {\"type\":\"response.completed\",\"response\":{\"model\":5}}";
    assert!(parse_completed_trace_row(numeric_model).is_none());

    let array = "turn.id=t1 websocket event: \
         [\"response.completed\",{\"model\":\"gpt-5.5\"}]";
    assert!(parse_completed_trace_row(array).is_none());
}

fn local_midnight(year: i32, month: u32, day: u32) -> i64 {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
        .map(|midnight| midnight.timestamp())
        .unwrap()
}

#[test]
fn text_timestamps_parse_seconds_or_day_keys() {
    assert_eq!(text_timestamp("42"), Some(42));
    assert_eq!(text_timestamp(" 42 "), Some(42));
    let midnight = local_midnight(2026, 9, 4);
    assert_eq!(text_timestamp("2026-09-04 12:34:56.789"), Some(midnight));
    assert_eq!(text_timestamp("2026-09-04T12:34:56Z"), Some(midnight));
    assert_eq!(text_timestamp("2026-09-04"), Some(midnight));
    assert_eq!(text_timestamp("garbage"), None);
    assert_eq!(text_timestamp("2026-13-40 00:00:00"), None);
    assert_eq!(text_timestamp(""), None);
}

#[test]
fn text_timestamp_rows_count_from_their_local_day() {
    let db = TraceDb::new(&[]);
    db.execute(&format!(
        "insert into logs (ts, feedback_log_body) values ('2026-09-04 12:00:00', '{}')",
        request_body("turn-1", "priority", "gpt-5.5")
    ));
    let cursor = resolve(&db, None).cursor.unwrap();
    assert_eq!(
        cursor.turn("turn-1").and_then(|turn| turn.timestamp),
        Some(local_midnight(2026, 9, 4))
    );
}

#[test]
fn cold_scan_collects_priority_turns_and_completed_model() {
    let mut rows = noise(3);
    rows.push((2_000, request_body("turn-1", "priority", "gpt-5.5")));
    rows.push((2_001, request_body("turn-2", "default", "gpt-5.5")));
    rows.push((2_002, completed_body("turn-1", "gpt-5.4")));
    let db = TraceDb::new(&rows);

    let resolution = resolve(&db, None);
    let cursor = resolution.cursor.unwrap();
    assert!(!resolution.validation_pending);
    assert_eq!(cursor.request_sources.len(), 1);
    assert!(cursor.turn("turn-1").is_some());
    assert_eq!(cursor.last_row_id, 6);
    assert_eq!(cursor.anchors.len(), 4);
    assert_eq!(cursor.turn_model("turn-1"), Some("gpt-5.4"));
}

#[test]
fn incremental_scan_appends_only_new_rows() {
    let db = TraceDb::new(&[(2_000, request_body("turn-1", "priority", "gpt-5.5"))]);
    let first = resolve(&db, None).cursor.unwrap();
    assert_eq!(first.last_row_id, 1);

    db.insert(&[(2_001, request_body("turn-2", "priority", "gpt-5.4"))]);
    let second = resolve(&db, Some(first)).cursor.unwrap();
    assert_eq!(second.last_row_id, 2);
    assert_eq!(second.request_sources.len(), 2);
}

#[test]
fn completion_before_request_is_matched_when_the_request_arrives() {
    let db = TraceDb::new(&[(2_000, completed_body("turn-1", "gpt-5.4"))]);
    let first = resolve(&db, None).cursor.unwrap();
    assert!(first.request_sources.is_empty());
    assert!(first.completed_models.contains_key("turn-1"));

    db.insert(&[(2_001, request_body("turn-1", "priority", "gpt-5.5"))]);
    let second = resolve(&db, Some(first)).cursor.unwrap();
    assert!(second.completed_models.is_empty());
    assert_eq!(second.turn_model("turn-1"), Some("gpt-5.4"));
}

#[test]
fn rewritten_database_rebuilds_instead_of_reusing_evidence() {
    let mut rows = noise(6);
    rows.push((2_000, request_body("old-turn", "priority", "gpt-5.5")));
    let db = TraceDb::new(&rows);
    let first = resolve(&db, None).cursor.unwrap();
    assert!(first.turn("old-turn").is_some());

    db.execute("update logs set feedback_log_body = 'rewritten ' || id");
    db.insert(&[(3_000, request_body("new-turn", "priority", "gpt-5.5"))]);
    let second = resolve(&db, Some(first)).cursor.unwrap();
    assert!(second.turn("old-turn").is_none());
    assert!(second.turn("new-turn").is_some());
}

#[test]
fn deleted_source_rows_drop_their_turns() {
    let db = TraceDb::new(&[
        (2_000, request_body("turn-1", "priority", "gpt-5.5")),
        (2_001, request_body("turn-2", "priority", "gpt-5.5")),
        (2_002, "tail row".to_string()),
    ]);
    let first = resolve(&db, None).cursor.unwrap();
    assert_eq!(first.request_sources.len(), 2);

    db.execute("delete from logs where id = 1");
    let second = resolve(&db, Some(first)).cursor.unwrap();
    assert!(second.turn("turn-1").is_none());
    assert!(second.turn("turn-2").is_some());
}

#[test]
fn cancelled_cold_scan_reports_pending_without_evidence() {
    let db = TraceDb::new(&[(2_000, request_body("turn-1", "priority", "gpt-5.5"))]);
    let cancel = AtomicBool::new(true);
    let resolution = resolve_priority_turns(&db.path, None, 0, false, Some(&cancel));
    assert!(resolution.validation_pending);
    assert!(
        resolution
            .cursor
            .is_none_or(|cursor| cursor.request_sources.is_empty())
    );
}

#[test]
fn missing_database_is_pending_only_after_it_supplied_evidence() {
    let db = TraceDb::new(&[(2_000, request_body("turn-1", "priority", "gpt-5.5"))]);
    let cursor = resolve(&db, None).cursor.unwrap();

    // A path that never held a database is a normal optional source.
    let missing = db.path.with_file_name("absent.sqlite");
    let never = resolve_priority_turns(&missing, None, 0, false, None);
    assert!(never.cursor.is_none());
    assert!(!never.validation_pending);

    // Once the path supplied evidence, its absence keeps that evidence and
    // retries (upstream `expectExistingDatabase`).
    let mut previous = cursor;
    previous.database_path = missing.to_string_lossy().to_string();
    let kept = resolve_priority_turns(&missing, Some(previous.clone()), 0, true, None);
    assert!(kept.validation_pending);
    assert_eq!(kept.cursor, Some(previous));
}

#[test]
fn unreadable_database_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(CODEX_TRACE_DATABASE_FILE);
    std::fs::write(&path, b"not a sqlite database").unwrap();
    let resolution = resolve_priority_turns(&path, None, 0, false, None);
    assert!(resolution.cursor.is_none());
    assert!(resolution.validation_pending);
}

#[test]
fn cursor_for_another_database_is_discarded() {
    let db = TraceDb::new(&[(2_000, request_body("turn-1", "priority", "gpt-5.5"))]);
    let mut stale = resolve(&db, None).cursor.unwrap();
    stale.database_path = "elsewhere".to_string();
    stale.request_sources.insert(
        "ghost".to_string(),
        BTreeMap::from([(
            2,
            CodexPriorityTurnMetadata {
                turn_id: "ghost".to_string(),
                ..CodexPriorityTurnMetadata::default()
            },
        )]),
    );
    let cursor = resolve(&db, Some(stale)).cursor.unwrap();
    assert!(cursor.turn("ghost").is_none());
    assert!(cursor.turn("turn-1").is_some());
}

#[test]
fn coverage_window_skips_older_history() {
    let db = TraceDb::new(&[
        (100, request_body("ancient", "priority", "gpt-5.5")),
        (5_000, request_body("recent", "priority", "gpt-5.5")),
    ]);
    let cursor = resolve_priority_turns(&db.path, None, 1_000, false, None)
        .cursor
        .unwrap();
    assert!(cursor.turn("ancient").is_none());
    assert!(cursor.turn("recent").is_some());
}

#[test]
fn advancing_coverage_prunes_expired_turns_without_restarting_the_cursor() {
    let db = TraceDb::new(&[
        (1_000, request_body("expired", "priority", "gpt-5.5")),
        (2_000, request_body("current", "priority", "gpt-5.5")),
    ]);
    let first = resolve(&db, None).cursor.unwrap();
    let last_row_id = first.last_row_id;
    assert!(first.turn("expired").is_some());

    let updated = resolve_priority_turns(&db.path, Some(first), 1_500, false, None)
        .cursor
        .unwrap();

    assert_eq!(updated.last_row_id, last_row_id);
    assert_eq!(updated.coverage_since_epoch, 1_500);
    assert!(updated.turn("expired").is_none());
    assert!(updated.turn("current").is_some());
}

#[test]
fn anchor_digest_tracks_fractional_sqlite_timestamps() {
    let db = TraceDb::new(&noise(6));
    let first = resolve(&db, None).cursor.unwrap();
    let anchor = first.anchors[0].clone();

    db.execute(&format!(
        "update logs set ts = ts + 0.25 where id = {}",
        anchor.row_id
    ));

    let updated = resolve(&db, Some(first)).cursor.unwrap();
    let updated_anchor = updated
        .anchors
        .iter()
        .find(|candidate| candidate.row_id == anchor.row_id)
        .unwrap();
    assert_ne!(updated_anchor.digest, anchor.digest);
}

#[test]
fn pending_completions_are_bounded() {
    let mut state = CodexPriorityTurnsCursor::default();
    for index in 0..(CODEX_PRIORITY_COMPLETED_MODEL_RETENTION_LIMIT + 10) {
        let row_id = i64::try_from(index).unwrap() + 1;
        absorb_row(
            &mut state,
            row_id,
            Some(1),
            &completed_body(&format!("turn-{index}"), "gpt-5.5"),
        );
    }
    assert_eq!(
        state.completed_models.len(),
        CODEX_PRIORITY_COMPLETED_MODEL_RETENTION_LIMIT
    );
    assert!(!state.completed_models.contains_key("turn-0"));
}

#[test]
fn overlay_is_scoped_to_the_codex_home_of_the_database() {
    let db = TraceDb::new(&[(2_000, request_body("turn-1", "priority", "gpt-5.5"))]);
    let cursor = resolve(&db, None).cursor.unwrap();
    let home = db.path.parent().unwrap();
    let inside = home.join("sessions").join("2026").join("a.jsonl");
    let outside = home
        .parent()
        .unwrap()
        .join("other-home")
        .join("sessions")
        .join("a.jsonl");
    let overlay = cursor.overlay_for_file(&inside).unwrap();
    assert!(cursor.overlay_for_file(&outside).is_none());

    assert_eq!(
        overlay.priority_model(Some("turn-1"), "gpt-5.5").as_deref(),
        Some("gpt-5.5-priority")
    );
    assert_eq!(overlay.priority_model(Some("turn-2"), "gpt-5.5"), None);
    assert_eq!(overlay.priority_model(None, "gpt-5.5"), None);
    assert_eq!(
        overlay.priority_model(Some("turn-1"), "gpt-5.5-priority"),
        None
    );
}

#[test]
fn model_without_a_fast_lane_stays_standard() {
    let db = TraceDb::new(&[(2_000, request_body("turn-1", "priority", "gpt-5-mini"))]);
    let cursor = resolve(&db, None).cursor.unwrap();
    let inside = db.path.parent().unwrap().join("sessions").join("a.jsonl");
    let overlay = cursor.overlay_for_file(&inside).unwrap();
    assert_eq!(overlay.priority_model(Some("turn-1"), "gpt-5-mini"), None);
}

fn write_session(sessions: &Path, lines: &[serde_json::Value]) {
    let today = Local::now().date_naive();
    let day_dir = sessions
        .join(today.format("%Y").to_string())
        .join(today.format("%m").to_string())
        .join(today.format("%d").to_string());
    std::fs::create_dir_all(&day_dir).unwrap();
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(day_dir.join("priority.jsonl"), body).unwrap();
}

fn turn_lines() -> Vec<serde_json::Value> {
    let now = Local::now().to_rfc3339();
    let turn = |turn_id: &str, input: i64, output: i64| {
        [
            serde_json::json!({
                "timestamp": now, "type": "event_msg",
                "payload": {"type": "task_started", "turn_id": turn_id}
            }),
            serde_json::json!({
                "timestamp": now, "type": "event_msg",
                "payload": {"type": "token_count", "info": {
                    "model": "gpt-5.5",
                    "total_token_usage": {
                        "input_tokens": input, "cached_input_tokens": 0,
                        "output_tokens": output, "reasoning_output_tokens": 0
                    }
                }}
            }),
        ]
    };
    // Cumulative totals: turn-fast adds 1000/100, turn-std adds 400/40.
    let mut lines = turn("turn-fast", 1000, 100).to_vec();
    lines.extend(turn("turn-std", 1400, 140));
    lines
}

#[test]
fn scan_prices_priority_turns_as_priority_and_others_as_standard() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    write_session(&sessions, &turn_lines());
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let now = Local::now().timestamp();
    let trace = TraceDb::in_dir(
        root.path(),
        &[(now, request_body("turn-fast", "priority", "gpt-5.5"))],
    );

    let scan = |cache: &str, trace_path: Option<&Path>| {
        let mut scanner = CostScanner::new(7)
            .with_options(CostScanOptions::app_driven())
            .with_cache_root(root.path().join(cache))
            .with_sessions_dirs(vec![sessions.clone()]);
        if let Some(path) = trace_path {
            scanner = scanner.with_codex_trace_database(path);
        }
        scanner.scan_codex_detailed_with_cache(None)
    };

    let (baseline, _, _) = scan("cache-plain", None);
    let (summary, _, cache) = scan("cache-priority", Some(&trace.path));

    let models = &cache.days[&day];
    assert_eq!(models["gpt-5.5-priority"][0], 1000);
    assert_eq!(models["gpt-5.5"][0], 400);
    assert_eq!(summary.input_tokens, baseline.input_tokens);

    let standard = CostUsagePricing::codex_cost_usd("gpt-5.5", 1000, 0, 100).unwrap();
    let fast = CostUsagePricing::codex_fast_cost_usd("gpt-5.5", 1000, 0, 100).unwrap();
    assert!(fast > standard);
    assert!((summary.total_cost_usd - baseline.total_cost_usd - (fast - standard)).abs() < 1e-9);
}

#[test]
fn scan_without_trace_database_keeps_standard_pricing() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    write_session(&sessions, &turn_lines());
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let scanner = CostScanner::new(7)
        .with_options(CostScanOptions::app_driven())
        .with_cache_root(root.path().join("cache"))
        .with_sessions_dirs(vec![sessions])
        .with_codex_trace_database(root.path().join("missing").join(CODEX_TRACE_DATABASE_FILE));
    let (_, _, cache) = scanner.scan_codex_detailed_with_cache(None);
    assert!(cache.days[&day].contains_key("gpt-5.5"));
    assert!(!cache.days[&day].contains_key("gpt-5.5-priority"));
}
