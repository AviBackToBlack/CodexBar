use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};

use crate::core::RateWindow;

use super::info::{
    KeyBinding, KeyInfoResponse, TeamInfoResponse, UserInfoResponse, bind_key, result_from_team,
    result_from_user,
};
use super::*;

fn binding(info: Value) -> KeyBinding {
    bind_key(serde_json::from_value::<KeyInfoResponse>(json!({ "info": info })).unwrap()).unwrap()
}

fn user_result(key: &KeyBinding, body: Value) -> Result<ProviderFetchResult, ProviderError> {
    let user_id = key.user_id.clone().unwrap();
    result_from_user(
        key,
        &user_id,
        serde_json::from_value::<UserInfoResponse>(body).unwrap(),
    )
}

fn team_result(key: &KeyBinding, body: Value) -> Result<ProviderFetchResult, ProviderError> {
    let team_id = key.team_id.clone().unwrap();
    result_from_team(
        key,
        &team_id,
        serde_json::from_value::<TeamInfoResponse>(body).unwrap(),
    )
}

fn personal_budget(spend: f64, max_budget: f64) -> ProviderFetchResult {
    let key = binding(json!({"user_id": "user-1"}));
    user_result(
        &key,
        json!({"user_info": {"spend": spend, "max_budget": max_budget}}),
    )
    .unwrap()
}

fn personal_reset(budget_reset_at: &str) -> Option<chrono::DateTime<Utc>> {
    let key = binding(json!({"user_id": "user-1"}));
    user_result(
        &key,
        json!({"user_info": {"spend": 1.0, "max_budget": 10.0, "budget_reset_at": budget_reset_at}}),
    )
    .unwrap()
    .usage
    .primary
    .resets_at
}

fn assert_parse_error(result: Result<ProviderFetchResult, ProviderError>, expected: &str) {
    match result {
        Err(ProviderError::Parse(message)) => {
            assert!(message.contains(expected), "unexpected message: {message}")
        }
        Err(other) => panic!("expected parse error, got {other}"),
        Ok(_) => panic!("expected parse error"),
    }
}

fn assert_no_budget(window: &RateWindow) {
    assert!(window.is_informational);
    assert!(!window.usage_known());
    assert_eq!(window.reset_description.as_deref(), Some("No budget set"));
    assert_eq!(window.resets_at, None);
}

#[test]
fn key_info_without_user_or_team_id_fails() {
    let response: KeyInfoResponse =
        serde_json::from_value(json!({"info": {"user_id": " ", "spend": 1.0}})).unwrap();
    match bind_key(response) {
        Err(error @ ProviderError::Parse(_)) => assert_eq!(
            error.to_string(),
            "Parse error: LiteLLM key info did not include a user_id or team_id."
        ),
        _ => panic!("expected parse error"),
    }
}

#[test]
fn key_info_requires_info_object() {
    assert!(serde_json::from_value::<KeyInfoResponse>(json!({"user_id": "u"})).is_err());
}

#[test]
fn metadata_names_budget_lanes_and_prefers_the_team_lane() {
    let provider = LiteLLMProvider::new();
    let metadata = provider.metadata();
    assert_eq!(metadata.session_label, "Personal budget");
    assert_eq!(metadata.weekly_label, "Team budget");
    assert!(!metadata.supports_credits);
    assert!(provider.automatic_metric_prefers_secondary_window());
}

#[test]
fn personal_budget_is_primary_with_identity() {
    let key = binding(json!({"user_id": "user-1", "expires": "2026-12-31T00:00:00Z"}));
    let result = user_result(
        &key,
        json!({
            "user_id": "user-1",
            "user_info": {
                "user_id": "user-1",
                "user_email": "dev@example.com",
                "spend": 25.0,
                "max_budget": 100.0,
                "budget_reset_at": "2026-10-01T00:00:00Z"
            },
            "teams": []
        }),
    )
    .unwrap();
    let primary = &result.usage.primary;
    assert_eq!(primary.used_percent, 25.0);
    assert_eq!(
        primary.reset_description.as_deref(),
        Some("$25.00 / $100.00")
    );
    assert!(primary.description_is_detail);
    assert!(!primary.is_informational);
    assert_eq!(
        primary.resets_at,
        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap())
    );
    assert_eq!(result.usage.primary_label, None);
    assert!(result.usage.secondary.is_none());
    assert!(result.usage.extra_rate_windows.is_empty());
    assert_eq!(
        result.usage.account_email.as_deref(),
        Some("dev@example.com")
    );
    assert_eq!(result.usage.login_method.as_deref(), Some("api"));
    assert!(
        result
            .usage
            .subscription
            .as_ref()
            .is_some_and(|sub| sub.expires_at.is_some())
    );
    assert!(!result.pace_authoritative);
    let cost = result.cost.expect("personal cost");
    assert_eq!(cost.used, 25.0);
    assert_eq!(cost.limit, Some(100.0));
    assert_eq!(cost.period, "Personal budget");
    assert!(!cost.always_visible);
}

#[test]
fn identity_falls_back_to_alias_then_preferred_username() {
    let key = binding(json!({"user_id": "user-1"}));
    let alias = user_result(
        &key,
        json!({"user_info": {"user_alias": "alias", "metadata": {"preferred_username": "pref"}}}),
    )
    .unwrap();
    assert_eq!(alias.usage.account_email.as_deref(), Some("alias"));
    let pref = user_result(
        &key,
        json!({"user_info": {"user_email": " ", "metadata": {"preferred_username": "pref"}}}),
    )
    .unwrap();
    assert_eq!(pref.usage.account_email.as_deref(), Some("pref"));
    let non_string = user_result(
        &key,
        json!({"user_info": {"metadata": {"preferred_username": 7}}}),
    )
    .unwrap();
    assert_eq!(non_string.usage.account_email, None);
}

#[test]
fn personal_and_team_budgets_fill_primary_and_secondary() {
    let key = binding(json!({"user_id": "user-1", "team_id": "team-b"}));
    let result = user_result(
        &key,
        json!({
            "user_info": {"user_id": "user-1", "spend": 25.0, "max_budget": 100.0},
            "teams": [
                {"team_id": "team-a", "team_alias": "Other", "spend": 1.0, "max_budget": 10.0},
                {
                    "team_id": "team-b",
                    "team_alias": "Platform",
                    "spend": 70.0,
                    "max_budget": 1000.0,
                    "budget_reset_at": "2026-11-01",
                    "budget_duration": "30d"
                }
            ]
        }),
    )
    .unwrap();
    assert_eq!(result.usage.primary.used_percent, 25.0);
    assert_eq!(
        result.usage.primary.reset_description.as_deref(),
        Some("$25.00 / $100.00")
    );
    let team = result.usage.secondary.as_ref().expect("team lane");
    assert!((team.used_percent - 7.0).abs() < 1e-9);
    assert_eq!(
        team.reset_description.as_deref(),
        Some("Team Platform: $70.00 / $1,000.00")
    );
    assert!(team.description_is_detail);
    assert_eq!(
        team.resets_at,
        Some(Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap())
    );
    // The metadata labels ("Personal budget" / "Team budget") apply.
    assert_eq!(result.usage.primary_label, None);
    assert_eq!(result.usage.secondary_label, None);
    assert!(result.usage.extra_rate_windows.is_empty());
    assert_eq!(
        result.usage.account_organization.as_deref(),
        Some("Platform")
    );
    let cost = result.cost.expect("personal cost");
    assert_eq!(cost.period, "Personal budget");
    assert_eq!(cost.limit, Some(100.0));
}

#[test]
fn team_budget_fills_the_primary_lane_without_a_personal_budget() {
    let key = binding(json!({"user_id": "user-1", "team_id": "team-b"}));
    let result = user_result(
        &key,
        json!({
            "user_info": {"user_id": "user-1", "spend": 3.0},
            "teams": [
                {"team_id": "team-a", "team_alias": "Other", "spend": 1.0, "max_budget": 10.0},
                {"team_id": "team-b", "team_alias": "Platform", "spend": 70.0, "max_budget": 1000.0}
            ]
        }),
    )
    .unwrap();
    let primary = &result.usage.primary;
    assert!((primary.used_percent - 7.0).abs() < 1e-9);
    assert_eq!(
        primary.reset_description.as_deref(),
        Some("Team Platform: $70.00 / $1,000.00")
    );
    assert!(primary.description_is_detail);
    assert_eq!(result.usage.primary_label.as_deref(), Some("Team budget"));
    assert!(result.usage.secondary.is_none());
    assert_eq!(
        result.usage.account_organization.as_deref(),
        Some("Platform")
    );
    // The cost stays scoped to the key's own (personal) spend.
    let cost = result.cost.expect("spend-only cost");
    assert_eq!(cost.used, 3.0);
    assert_eq!(cost.period, "Personal spend");
    assert_eq!(cost.limit, None);
    assert!(cost.always_visible);
}

#[test]
fn team_without_a_matching_entry_is_omitted() {
    let key = binding(json!({"user_id": "user-1", "team_id": "team-x"}));
    let result = user_result(
        &key,
        json!({
            "user_info": {"spend": 1.0},
            "teams": [{"team_id": "team-a", "spend": 1.0, "max_budget": 10.0}]
        }),
    )
    .unwrap();
    assert_no_budget(&result.usage.primary);
    assert_eq!(result.usage.primary_label, None);
    assert!(result.usage.secondary.is_none());
    assert!(result.usage.extra_rate_windows.is_empty());
    assert_eq!(result.usage.account_organization, None);
}

#[test]
fn team_entries_match_the_key_team_id_exactly() {
    let key = binding(json!({"user_id": "user-1", "team_id": "team-a"}));
    let result = user_result(
        &key,
        json!({
            "user_info": {"spend": 5.0, "max_budget": 50.0},
            "teams": [{"team_id": " team-a ", "team_alias": "Padded", "spend": 1.0, "max_budget": 10.0}]
        }),
    )
    .unwrap();
    assert!(result.usage.secondary.is_none());
    assert_eq!(result.usage.account_organization, None);
}

#[test]
fn team_without_budget_or_alias_adds_no_lane_or_organization() {
    let key = binding(json!({"user_id": "user-1", "team_id": "team-a"}));
    let unbudgeted = user_result(
        &key,
        json!({
            "user_info": {"spend": 5.0, "max_budget": 50.0},
            "teams": [{"team_id": "team-a", "team_alias": "Platform", "spend": 9.0}]
        }),
    )
    .unwrap();
    assert!(unbudgeted.usage.secondary.is_none());
    assert_eq!(
        unbudgeted.usage.account_organization.as_deref(),
        Some("Platform")
    );

    let blank_alias = user_result(
        &key,
        json!({
            "user_info": {"spend": 5.0, "max_budget": 50.0},
            "teams": [{"team_id": "team-a", "team_alias": " ", "spend": 9.0, "max_budget": 90.0}]
        }),
    )
    .unwrap();
    let team = blank_alias.usage.secondary.as_ref().expect("team lane");
    assert_eq!(
        team.reset_description.as_deref(),
        Some("Team: $9.00 / $90.00")
    );
    assert_eq!(blank_alias.usage.account_organization, None);
}

#[test]
fn mismatched_user_id_is_rejected() {
    let key = binding(json!({"user_id": "user-1"}));
    assert_parse_error(
        user_result(&key, json!({"user_info": {"user_id": "user-2"}})),
        "user_id did not match /key/info",
    );
    assert_parse_error(
        user_result(&key, json!({"user_id": "user-2", "user_info": {}})),
        "user_id did not match /key/info",
    );
    // The card shows the error's Display text, so "parse error" appears once.
    let Err(error) = user_result(&key, json!({"user_info": {"user_id": "user-2"}})) else {
        panic!("expected parse error");
    };
    assert_eq!(
        error.to_string(),
        "Parse error: LiteLLM user_id did not match /key/info"
    );
}

#[test]
fn team_entries_without_team_id_are_rejected() {
    let key = binding(json!({"user_id": "user-1", "team_id": "team-a"}));
    assert_parse_error(
        user_result(&key, json!({"user_info": {}, "teams": [{"spend": 1.0}]})),
        "missing team_id",
    );
}

#[test]
fn wrongly_typed_fields_fail_to_parse() {
    assert!(
        serde_json::from_value::<UserInfoResponse>(json!({"user_info": {"spend": "12"}})).is_err()
    );
    assert!(serde_json::from_value::<UserInfoResponse>(json!({"teams": []})).is_err());
    assert!(
        serde_json::from_value::<UserInfoResponse>(json!({"user_info": {}, "teams": {}})).is_err()
    );
    assert!(
        serde_json::from_value::<UserInfoResponse>(json!({
            "user_info": {},
            "teams": [{"team_id": "team-a", "budget_duration": 30}]
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<KeyInfoResponse>(json!({"info": {"user_id": "u", "key_name": 5}}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<KeyInfoResponse>(json!({"info": {"user_id": "u", "spend": "1"}}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<TeamInfoResponse>(json!({"team_info": {"budget_duration": 30}}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<KeyInfoResponse>(json!({
            "info": {"user_id": "u", "key_name": "ci", "spend": 1.5}
        }))
        .is_ok()
    );
}

#[test]
fn team_only_key_shows_team_budget_as_sole_window() {
    let key = binding(json!({"team_id": "team-a"}));
    let result = team_result(
        &key,
        json!({
            "team_id": "team-a",
            "team_info": {
                "team_id": "team-a",
                "team_alias": "Platform",
                "spend": 70.0,
                "max_budget": 1000.0,
                "budget_reset_at": "2026-10-01T00:00:00"
            }
        }),
    )
    .unwrap();
    let primary = &result.usage.primary;
    assert!((primary.used_percent - 7.0).abs() < 1e-9);
    assert_eq!(
        primary.reset_description.as_deref(),
        Some("Team Platform: $70.00 / $1,000.00")
    );
    assert!(primary.description_is_detail);
    assert_eq!(
        primary.resets_at,
        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap())
    );
    assert_eq!(result.usage.primary_label.as_deref(), Some("Team budget"));
    assert!(result.usage.secondary.is_none());
    assert!(result.usage.extra_rate_windows.is_empty());
    assert_eq!(
        result.usage.account_organization.as_deref(),
        Some("Platform")
    );
    assert_eq!(result.usage.account_email, None);
    assert!(!result.pace_authoritative);
    let cost = result.cost.expect("team cost");
    assert_eq!(cost.period, "Team budget");
    assert_eq!(cost.limit, Some(1000.0));
    assert!(!cost.always_visible);
}

#[test]
fn team_only_key_without_a_budget_reports_spend_only() {
    let key = binding(json!({"team_id": "team-a"}));
    let result = team_result(
        &key,
        json!({"team_info": {"team_id": "team-a", "spend": 12.5}}),
    )
    .unwrap();
    assert_no_budget(&result.usage.primary);
    assert_eq!(result.usage.primary_label.as_deref(), Some("Team budget"));
    assert!(result.usage.secondary.is_none());
    assert_eq!(result.usage.account_organization, None);
    let cost = result.cost.expect("team spend");
    assert_eq!(cost.used, 12.5);
    assert_eq!(cost.period, "Team spend");
    assert_eq!(cost.limit, None);
    assert!(cost.always_visible);
}

#[test]
fn mismatched_team_id_is_rejected() {
    let key = binding(json!({"team_id": "team-a"}));
    assert_parse_error(
        team_result(
            &key,
            json!({"team_id": "team-b", "team_info": {"spend": 1.0}}),
        ),
        "team_id did not match /key/info",
    );
    assert_parse_error(
        team_result(&key, json!({"team_info": {"team_id": "team-b"}})),
        "team_id did not match /key/info",
    );
    // A blank nested id falls back to the root id.
    assert_parse_error(
        team_result(
            &key,
            json!({"team_id": "team-b", "team_info": {"team_id": " "}}),
        ),
        "team_id did not match /key/info",
    );
}

#[test]
fn team_ids_in_team_info_are_trimmed() {
    let key = binding(json!({"team_id": "team-a"}));
    assert!(
        team_result(
            &key,
            json!({"team_id": " team-a ", "team_info": {"spend": 1.0}})
        )
        .is_ok()
    );
    assert!(team_result(&key, json!({"team_info": {"team_id": "team-a "}})).is_ok());
    assert!(team_result(&key, json!({"team_info": {}})).is_ok());
}

#[test]
fn spend_above_budget_clamps_percent_and_zero_budget_is_spend_only() {
    let over = personal_budget(150.0, 100.0);
    assert_eq!(over.usage.primary.used_percent, 100.0);
    assert_eq!(
        over.usage.primary.reset_description.as_deref(),
        Some("$150.00 / $100.00")
    );

    for limit in [0.0, -5.0] {
        let unbudgeted = personal_budget(4.0, limit);
        assert_no_budget(&unbudgeted.usage.primary);
        let cost = unbudgeted.cost.expect("cost");
        assert_eq!(cost.period, "Personal spend");
        assert_eq!(cost.limit, None);
        assert!(cost.always_visible);
    }

    let key = binding(json!({"user_id": "user-1"}));
    let empty = user_result(&key, json!({"user_info": {}})).unwrap();
    assert_no_budget(&empty.usage.primary);
    assert!(empty.cost.is_none());
}

#[test]
fn amounts_use_grouped_dollars_like_upstream() {
    assert_eq!(
        personal_budget(1_234_567.891, 2_000_000.0)
            .usage
            .primary
            .reset_description
            .as_deref(),
        Some("$1,234,567.89 / $2,000,000.00")
    );
    assert_eq!(
        personal_budget(999.999, 1000.0)
            .usage
            .primary
            .reset_description
            .as_deref(),
        Some("$1,000.00 / $1,000.00")
    );
    let credit = personal_budget(-5.0, 10.0);
    assert_eq!(credit.usage.primary.used_percent, 0.0);
    assert_eq!(
        credit.usage.primary.reset_description.as_deref(),
        Some("-$5.00 / $10.00")
    );
    assert_eq!(
        personal_budget(0.0, 0.5)
            .usage
            .primary
            .reset_description
            .as_deref(),
        Some("$0.00 / $0.50")
    );
}

#[test]
fn budget_reset_dates_accept_offsets_naive_times_and_dates() {
    assert_eq!(
        personal_reset("2026-10-01T05:30:00+02:00"),
        Some(Utc.with_ymd_and_hms(2026, 10, 1, 3, 30, 0).unwrap())
    );
    assert_eq!(
        personal_reset("2026-10-01T05:30:00"),
        Some(Utc.with_ymd_and_hms(2026, 10, 1, 5, 30, 0).unwrap())
    );
    assert_eq!(
        personal_reset(" 2026-10-01 "),
        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap())
    );
    assert_eq!(personal_reset("next month"), None);
    assert_eq!(personal_reset(""), None);
}

#[test]
fn saved_base_url_uses_only_app_saved_key() {
    let mut ctx = FetchContext {
        workspace_id: Some("https://litellm.example.com".to_string()),
        ..Default::default()
    };
    assert!(matches!(
        resolve_base_and_key(&ctx),
        Err(ProviderError::AuthRequired)
    ));

    ctx.api_key = Some("sk-app".to_string());
    let (base, key) = resolve_base_and_key(&ctx).unwrap();
    assert_eq!(base, "https://litellm.example.com");
    assert_eq!(key, "sk-app");
}

// Spend-report fallback (upstream 0.65.0). Shape reported upstream in #3834;
// identifiers and amounts are synthetic.
const REPORT: &str = r#"[{"api_key":"synthetic-key-identifier","total_cost":1.25,
    "total_input_tokens":1000,"total_output_tokens":100,
    "model_details":[{"model":"example-model","total_cost":1.25,
    "total_input_tokens":1000,"total_output_tokens":100}]},
    {"api_key":"another-synthetic-identifier","total_cost":2.5}]"#;
const KEY_QUERY: &str = "?start_date=2026-09-01&end_date=2026-09-21";

fn sept_21() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn test_client() -> Client {
    Client::builder().no_proxy().build().unwrap()
}

/// Serves `path` (under the `/proxy` prefix) with `status` and `body`,
/// asserting the bearer key. `hits` is how many requests it must receive.
async fn stub(
    server: &mut mockito::ServerGuard,
    path: &str,
    status: usize,
    body: &str,
    hits: usize,
) -> mockito::Mock {
    let mock = server.mock("GET", path);
    // A bare path would not match its own request once a query is added.
    let mock = if path.contains('?') {
        mock
    } else {
        mock.match_query(mockito::Matcher::Any)
    };
    mock.match_header("authorization", "Bearer fixture-key")
        .match_header("accept", "application/json")
        .with_status(status)
        .with_body(body)
        .expect(hits)
        .create_async()
        .await
}

/// Fetch through the real management-route builder, with the base path
/// prefix `/proxy/v1` (the mock server is a loopback HTTP origin).
async fn run(
    server: &mockito::ServerGuard,
    now: DateTime<Utc>,
) -> Result<ProviderFetchResult, ProviderError> {
    let base = format!("{}/proxy/v1", server.url());
    fetch_key_usage(
        &test_client(),
        |path, query| management_url(&base, path, query),
        "fixture-key",
        now,
    )
    .await
}

#[test]
fn management_url_keeps_path_prefix_and_strips_v1() {
    for base in [
        "https://proxy.example.com/proxy/v1/",
        "https://proxy.example.com/proxy/v1",
        "https://proxy.example.com/proxy/",
        "https://proxy.example.com/proxy",
    ] {
        assert_eq!(
            management_url(base, "key/spend/report", None)
                .unwrap()
                .as_str(),
            "https://proxy.example.com/proxy/key/spend/report"
        );
    }
    assert_eq!(
        management_url("https://litellm.example.com", "key/info", None)
            .unwrap()
            .as_str(),
        "https://litellm.example.com/key/info"
    );
    assert_eq!(
        management_url("https://litellm.example.com/v1", "key/info", None)
            .unwrap()
            .as_str(),
        "https://litellm.example.com/key/info"
    );
}

#[test]
fn sums_each_row_once_and_ignores_model_details() {
    assert_eq!(spend_report::parse(REPORT.as_bytes()).unwrap(), 3.75);
    assert_eq!(
        spend_report::parse(br#"[{"total_cost":0}]"#).unwrap(),
        0.0,
        "an explicit zero spend is valid"
    );
}

#[test]
fn invalid_or_empty_reports_never_fabricate_zero_spend() {
    for body in [
        "[]",
        "{}",
        "null",
        "not json",
        "",
        "[null]",
        "[{}]",
        r#"[{"total_cost":null}]"#,
        r#"[{"total_cost":"1.25"}]"#,
        r#"[{"total_cost":-1}]"#,
        r#"[{"total_cost":1e309}]"#,
        r#"[{"total_cost":1e308},{"total_cost":1e308}]"#,
    ] {
        match spend_report::parse(body.as_bytes()) {
            Err(ProviderError::Parse(message)) => {
                assert!(!message.contains("1.25"), "{body}: {message}");
            }
            other => panic!("{body}: expected a parse error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn management_denial_falls_back_to_key_spend() {
    for status in [401, 403, 404] {
        let mut server = mockito::Server::new_async().await;
        let info = stub(&mut server, "/proxy/key/info", status, "{}", 1).await;
        let report = stub(
            &mut server,
            &format!("/proxy/key/spend/report{KEY_QUERY}"),
            200,
            REPORT,
            1,
        )
        .await;
        let user = stub(&mut server, "/proxy/user/spend/report", 200, REPORT, 0).await;

        let result = run(&server, sept_21()).await.unwrap();

        info.assert_async().await;
        report.assert_async().await;
        user.assert_async().await;
        let period = "Key spend only (2026-09-01\u{2013}2026-09-21 UTC)";
        let cost = result.cost.expect("spend is reported as a cost");
        assert_eq!(cost.used, 3.75);
        assert_eq!(cost.currency_code, "USD");
        assert_eq!(cost.period, period);
        assert_eq!(
            (cost.limit, cost.resets_at, cost.balance),
            (None, None, None)
        );
        let primary = &result.usage.primary;
        assert!(primary.is_informational && !primary.usage_known);
        assert_eq!(primary.reset_description.as_deref(), Some(period));
        assert!(result.usage.secondary.is_none());
        assert!(result.usage.extra_rate_windows.is_empty());
        assert!(
            result.usage.account_email.is_none() && result.usage.account_organization.is_none()
        );
        assert!(result.usage.login_method.is_none());
    }
}

#[tokio::test]
async fn unavailable_key_report_falls_back_to_user_spend() {
    for status in [401, 403, 404] {
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
        let key = stub(
            &mut server,
            &format!("/proxy/key/spend/report{KEY_QUERY}"),
            status,
            "{}",
            1,
        )
        .await;
        let user = stub(
            &mut server,
            &format!("/proxy/user/spend/report{KEY_QUERY}"),
            200,
            REPORT,
            1,
        )
        .await;

        let result = run(&server, sept_21()).await.unwrap();

        key.assert_async().await;
        user.assert_async().await;
        let cost = result.cost.unwrap();
        assert_eq!(cost.used, 3.75);
        assert_eq!(
            cost.period,
            "User spend only (2026-09-01\u{2013}2026-09-21 UTC)"
        );
        assert!(result.usage.account_email.is_none());
    }
}

#[tokio::test]
async fn zero_spend_is_valid_and_month_boundary_uses_utc() {
    let mut server = mockito::Server::new_async().await;
    stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
    let report = stub(
        &mut server,
        "/proxy/key/spend/report?start_date=2027-01-01&end_date=2027-01-01",
        200,
        r#"[{"total_cost":0}]"#,
        1,
    )
    .await;
    let now = DateTime::parse_from_rfc3339("2027-01-01T00:00:01Z")
        .unwrap()
        .with_timezone(&Utc);

    let cost = run(&server, now).await.unwrap().cost.unwrap();

    report.assert_async().await;
    assert_eq!(cost.used, 0.0);
    assert_eq!(
        cost.period,
        "Key spend only (2027-01-01\u{2013}2027-01-01 UTC)"
    );
}

#[tokio::test]
async fn malformed_report_is_a_parse_error() {
    let mut server = mockito::Server::new_async().await;
    stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
    stub(
        &mut server,
        &format!("/proxy/key/spend/report{KEY_QUERY}"),
        200,
        "[]",
        1,
    )
    .await;
    assert!(matches!(
        run(&server, sept_21()).await,
        Err(ProviderError::Parse(_))
    ));
}

#[tokio::test]
async fn transport_failure_does_not_switch_report_scopes() {
    let mut server = mockito::Server::new_async().await;
    stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
    let user = stub(&mut server, "/proxy/user/spend/report", 200, REPORT, 0).await;
    let base = server.url();
    // Key report requests go to a closed port; the user report must not run.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let result = fetch_key_usage(
        &test_client(),
        |path, query| {
            let origin = if path == "key/spend/report" {
                format!("http://127.0.0.1:{closed_port}")
            } else {
                base.clone()
            };
            management_url(&format!("{origin}/proxy"), path, query)
        },
        "fixture-key",
        sept_21(),
    )
    .await;
    assert!(matches!(result, Err(ProviderError::Network(_))));
    user.assert_async().await;
}

#[tokio::test]
async fn other_failures_keep_their_class_and_never_echo_the_report() {
    for status in [429, 500, 400] {
        // A management failure other than 401/403/404 never tries a report.
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", status, REPORT, 1).await;
        let key = stub(&mut server, "/proxy/key/spend/report", 200, REPORT, 0).await;
        let Err(ProviderError::Other(message)) = run(&server, sept_21()).await else {
            panic!("expected an error for {status}");
        };
        assert!(message.contains(&status.to_string()), "{message}");
        key.assert_async().await;

        // The same statuses on the key report do not reach the user report.
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
        stub(
            &mut server,
            &format!("/proxy/key/spend/report{KEY_QUERY}"),
            status,
            REPORT,
            1,
        )
        .await;
        let user = stub(&mut server, "/proxy/user/spend/report", 200, REPORT, 0).await;
        let Err(ProviderError::Other(message)) = run(&server, sept_21()).await else {
            panic!("expected an error for {status}");
        };
        assert!(message.contains(&status.to_string()), "{message}");
        assert!(!message.contains("synthetic-key-identifier"), "{message}");
        user.assert_async().await;
    }
}

#[tokio::test]
async fn unavailable_user_report_surfaces_its_status_class() {
    for (status, expect_auth) in [(401, true), (403, true), (404, false)] {
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
        stub(
            &mut server,
            &format!("/proxy/key/spend/report{KEY_QUERY}"),
            404,
            "{}",
            1,
        )
        .await;
        stub(
            &mut server,
            &format!("/proxy/user/spend/report{KEY_QUERY}"),
            status,
            REPORT,
            1,
        )
        .await;
        match run(&server, sept_21()).await {
            Err(ProviderError::AuthRequired) if expect_auth => {}
            Err(ProviderError::Other(message)) if !expect_auth => {
                assert!(message.contains("404"), "{message}");
                assert!(!message.contains("synthetic-key-identifier"), "{message}");
            }
            other => panic!("status {status}: unexpected {other:?}"),
        }
    }
}

/// A readable `/key/info` keeps the budget flow (`/user/info` here) and
/// never requests a spend report.
#[tokio::test]
async fn key_info_success_is_unchanged() {
    let mut server = mockito::Server::new_async().await;
    let info = stub(
        &mut server,
        "/proxy/key/info",
        200,
        r#"{"info":{"user_id":"user-1"}}"#,
        1,
    )
    .await;
    let user = stub(
        &mut server,
        "/proxy/user/info",
        200,
        r#"{"user_info":{"user_id":"user-1","spend":25.0,"max_budget":100.0}}"#,
        1,
    )
    .await;
    let report = stub(&mut server, "/proxy/key/spend/report", 200, REPORT, 0).await;
    let result = run(&server, sept_21()).await.unwrap();
    info.assert_async().await;
    user.assert_async().await;
    report.assert_async().await;
    assert_eq!(result.usage.primary.used_percent, 25.0);
}
