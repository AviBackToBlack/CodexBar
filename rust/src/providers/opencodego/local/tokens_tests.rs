//! Recorded-token coverage for the OpenCode Go local reader (upstream 0.67.0).

use super::*;

const FULL: &str =
    r#"{"total":1600,"input":100,"output":20,"reasoning":30,"cache":{"read":1400,"write":50}}"#;
/// Older OpenCode rows record no `total`; the five components sum to 20.
const NO_TOTAL: &str = r#"{"input":10,"output":5,"reasoning":1,"cache":{"read":4,"write":0}}"#;

fn iso_ms(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .unwrap()
        .timestamp_millis()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 8, 0, 0, 0).unwrap()
}

fn open_db(path: &Path, with_part_table: bool) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE message (id TEXT PRIMARY KEY, data TEXT, time_created INTEGER);",
    )
    .unwrap();
    if with_part_table {
        conn.execute_batch(
            "CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, data TEXT, time_created INTEGER);",
        )
        .unwrap();
    }
    conn
}

/// `tokens` is spliced in as raw JSON so malformed shapes can be exercised.
fn insert_message(conn: &Connection, id: &str, created_ms: i64, model: &str, tokens: Option<&str>) {
    let tokens_json = tokens.map_or_else(String::new, |t| format!(r#","tokens":{t}"#));
    let data = format!(
        r#"{{"providerID":"opencode-go","role":"assistant","cost":1.5,"modelID":"{model}"{tokens_json},"time":{{"created":{created_ms}}}}}"#
    );
    conn.execute(
        "INSERT INTO message (id, data, time_created) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, data, created_ms],
    )
    .unwrap();
}

fn insert_step_finish(conn: &Connection, id: &str, message_id: &str, ms: i64, tokens: &str) {
    let data = format!(
        r#"{{"type":"step-finish","cost":0.5,"tokens":{tokens},"time":{{"created":{ms}}}}}"#
    );
    conn.execute(
        "INSERT INTO part (id, message_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, message_id, data, ms],
    )
    .unwrap();
}

fn parse(json: &str) -> Option<RowTokens> {
    RowTokens::parse(json)
}

#[test]
fn row_tokens_resolve_total_or_sum_all_five_components() {
    assert_eq!(parse(FULL).unwrap().total, 1600);
    let older = parse(NO_TOTAL).unwrap();
    assert_eq!(older.total, 20);
    assert_eq!(older.cache_write, Some(0));
    // A recorded total is used as-is even when components are absent.
    let total_only = parse(r#"{"total":7}"#).unwrap();
    assert_eq!((total_only.total, total_only.input), (7, None));
}

#[test]
fn row_tokens_reject_unresolvable_negative_malformed_and_overflowing_rows() {
    // No total and a missing component: nothing to resolve.
    assert_eq!(parse(r#"{"input":1,"output":2}"#), None);
    assert_eq!(parse("{}"), None);
    // Any negative count, including the total, makes the row unusable.
    assert_eq!(parse(r#"{"total":-1}"#), None);
    assert_eq!(parse(r#"{"total":5,"input":-1}"#), None);
    assert_eq!(parse(r#"{"total":5,"cache":{"write":-3}}"#), None);
    // Malformed shapes are unknown, never zero.
    assert_eq!(parse(r#"{"total":"12"}"#), None);
    assert_eq!(parse(r#"{"total":1.5}"#), None);
    assert_eq!(parse(r#"{"total":5,"cache":7}"#), None);
    assert_eq!(parse("not json"), None);
    // Checked sum overflow when the total has to be derived.
    let max = i64::MAX;
    let overflow = format!(
        r#"{{"input":{max},"output":{max},"reasoning":{max},"cache":{{"read":{max},"write":{max}}}}}"#
    );
    assert_eq!(parse(&overflow), None);
}

#[test]
fn daily_entries_carry_message_token_counts_per_day_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    let conn = open_db(&db, false);
    let (first, second) = (
        iso_ms("2026-03-06T11:00:00.000Z"),
        iso_ms("2026-03-06T11:30:00.000Z"),
    );
    insert_message(&conn, "m1", first, "kimi-k2", Some(FULL));
    insert_message(&conn, "m2", second, "kimi-k2", Some(NO_TOTAL));
    drop(conn);

    let rows = read_rows(&db).unwrap();
    let daily = daily_model_costs(&rows, now(), 30);
    assert_eq!(daily.len(), 1);
    let tokens = &daily[0].tokens;
    assert_eq!(tokens.complete_total(), Some(1620));
    assert_eq!(tokens.input, 110);
    assert_eq!(tokens.output, 25);
    assert_eq!(tokens.reasoning, 31);
    assert_eq!(tokens.complete_reasoning(), Some(31));
    assert_eq!(tokens.cache_read, 1404);
    assert_eq!(tokens.cache_write, 50);
    // Costs stay the recorded field, never derived from tokens.
    assert!((daily[0].cost - 3.0).abs() < 1e-9);
}

#[test]
fn step_finish_tokens_replace_the_parent_message_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    let conn = open_db(&db, true);
    let created = iso_ms("2026-03-06T11:00:00.000Z");
    // The parent message records the session total; the steps record each call.
    insert_message(&conn, "m1", created, "kimi-k2", Some(FULL));
    let step_a =
        r#"{"total":30,"input":10,"output":5,"reasoning":5,"cache":{"read":10,"write":0}}"#;
    let step_b = r#"{"total":20,"input":8,"output":2,"reasoning":0,"cache":{"read":10,"write":0}}"#;
    insert_step_finish(&conn, "p1", "m1", created, step_a);
    insert_step_finish(&conn, "p2", "m1", created, step_b);
    // A message with no step-finish parts keeps using its own tokens.
    insert_message(&conn, "m2", created, "kimi-k2", Some(NO_TOTAL));
    drop(conn);

    let rows = read_rows(&db).unwrap();
    assert_eq!(rows.len(), 3);
    let daily = daily_model_costs(&rows, now(), 30);
    let tokens = &daily[0].tokens;
    assert_eq!(tokens.complete_total(), Some(30 + 20 + 20));
    assert_eq!(tokens.input, 10 + 8 + 10);
    assert_eq!(tokens.cache_read, 10 + 10 + 4);
}

#[test]
fn rows_without_usable_tokens_leave_the_day_incomplete_not_zero() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    let conn = open_db(&db, false);
    let created = iso_ms("2026-03-06T11:00:00.000Z");
    insert_message(&conn, "m1", created, "kimi-k2", Some(FULL));
    insert_message(&conn, "m2", created, "kimi-k2", None);
    insert_message(&conn, "m3", created, "kimi-k2", Some("5"));
    insert_message(&conn, "m4", created, "kimi-k2", Some(r#"{"total":-4}"#));
    drop(conn);

    let rows = read_rows(&db).unwrap();
    // Costs survive even when tokens do not.
    assert_eq!(rows.len(), 4);
    let daily = daily_model_costs(&rows, now(), 30);
    let tokens = &daily[0].tokens;
    assert_eq!(tokens.complete_total(), None);
    assert_eq!(tokens.complete_reasoning(), None);
    assert!(tokens.has_usable_rows());
    assert_eq!(tokens.total, 1600);
    assert!((daily[0].cost - 6.0).abs() < 1e-9);
}

#[test]
fn model_summary_carries_per_model_tokens_and_marks_partial_models() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    let conn = open_db(&db, false);
    let created = iso_ms("2026-03-06T11:00:00.000Z");
    insert_message(&conn, "m1", created, "kimi-k2", Some(FULL));
    insert_message(&conn, "m2", created, "kimi-k2", Some(NO_TOTAL));
    insert_message(&conn, "m3", created, "glm-5", None);
    insert_message(
        &conn,
        "m4",
        created,
        "mimo",
        Some(r#"{"total":9,"input":4}"#),
    );
    drop(conn);

    let rows = read_rows(&db).unwrap();
    let summary = model_cost_summary_from_rows(&rows, now(), 30);

    let kimi = summary.by_model_tokens["kimi-k2"]
        .to_model_token_counts()
        .unwrap();
    assert_eq!(
        (kimi.input_tokens, kimi.output_tokens, kimi.cached_tokens),
        (110, 25, 1404 + 50)
    );
    assert_eq!(kimi.reasoning_tokens, Some(31));

    // A model with no usable row has no token counts at all (unknown, not zero).
    assert_eq!(
        summary.by_model_tokens["glm-5"].to_model_token_counts(),
        None
    );
    // A row that omits classes keeps reasoning unknown.
    let mimo = summary.by_model_tokens["mimo"]
        .to_model_token_counts()
        .unwrap();
    assert_eq!((mimo.input_tokens, mimo.reasoning_tokens), (4, None));
    assert_eq!(summary.by_model_tokens["mimo"].complete_total(), Some(9));

    // The window total is incomplete because one model has no usable row.
    assert_eq!(summary.tokens.complete_total(), None);
    assert_eq!(summary.tokens.total, 1600 + 20 + 9);
}

#[test]
fn token_sums_treat_overflow_as_an_unusable_row() {
    let big = RowTokens::parse(&format!(r#"{{"total":{},"input":1}}"#, i64::MAX)).unwrap();
    let mut sums = TokenSums::default();
    sums.add(Some(&big));
    sums.add(Some(&big));
    assert_eq!(sums.complete_total(), Some(i64::MAX as u64 * 2));
    sums.add(Some(&big));
    assert_eq!(sums.complete_total(), None);
    assert!(sums.has_usable_rows());
    assert_eq!(sums.total, i64::MAX as u64 * 2);
}
