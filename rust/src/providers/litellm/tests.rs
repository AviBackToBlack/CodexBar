use serde_json::{Value, json};

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

fn assert_parse_error(result: Result<ProviderFetchResult, ProviderError>, expected: &str) {
    match result {
        Err(ProviderError::Parse(message)) => {
            assert!(message.contains(expected), "unexpected message: {message}")
        }
        Err(other) => panic!("expected parse error, got {other}"),
        Ok(_) => panic!("expected parse error"),
    }
}

#[test]
fn key_info_without_user_or_team_id_fails() {
    let response: KeyInfoResponse =
        serde_json::from_value(json!({"info": {"user_id": " ", "spend": 1.0}})).unwrap();
    match bind_key(response) {
        Err(ProviderError::Parse(message)) => assert!(
            message.contains("LiteLLM key info did not include a user_id or team_id."),
            "unexpected message: {message}"
        ),
        _ => panic!("expected parse error"),
    }
}

#[test]
fn key_info_requires_info_object() {
    assert!(serde_json::from_value::<KeyInfoResponse>(json!({"user_id": "u"})).is_err());
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
    assert_eq!(result.usage.primary.used_percent, 25.0);
    assert_eq!(
        result.usage.primary.reset_description.as_deref(),
        Some("$25.00 / $100.00")
    );
    assert!(result.usage.primary.resets_at.is_some());
    assert_eq!(
        result.usage.account_email.as_deref(),
        Some("dev@example.com")
    );
    assert_eq!(result.usage.login_method.as_deref(), Some("api"));
    assert!(result.usage.extra_rate_windows.is_empty());
    assert!(
        result
            .usage
            .subscription
            .as_ref()
            .is_some_and(|sub| sub.expires_at.is_some())
    );
    let cost = result.cost.expect("personal cost");
    assert_eq!(cost.used, 25.0);
    assert_eq!(cost.limit, Some(100.0));
    assert_eq!(cost.period, "Personal budget");
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
}

#[test]
fn matching_team_budget_is_a_separate_row() {
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
    assert_eq!(result.usage.extra_rate_windows.len(), 1);
    let team = &result.usage.extra_rate_windows[0].window;
    assert!((team.used_percent - 7.0).abs() < 1e-9);
    assert_eq!(
        team.reset_description.as_deref(),
        Some("Team Platform: $70.00 / $1000.00")
    );
    assert_eq!(
        result.usage.account_organization.as_deref(),
        Some("Platform")
    );
    let cost = result.cost.expect("spend-only cost");
    assert_eq!(cost.period, "Personal spend");
    assert_eq!(cost.limit, None);
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
    assert!(result.usage.extra_rate_windows.is_empty());
    assert_eq!(result.usage.account_organization, None);
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
    assert!((result.usage.primary.used_percent - 7.0).abs() < 1e-9);
    assert_eq!(
        result.usage.primary.reset_description.as_deref(),
        Some("Team Platform: $70.00 / $1000.00")
    );
    assert!(result.usage.primary.resets_at.is_some());
    assert_eq!(result.usage.primary_label.as_deref(), Some("Team budget"));
    assert!(result.usage.extra_rate_windows.is_empty());
    assert_eq!(
        result.usage.account_organization.as_deref(),
        Some("Platform")
    );
    assert_eq!(result.usage.account_email, None);
    assert_eq!(result.cost.expect("team cost").period, "Team budget");
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
}

#[test]
fn spend_above_budget_clamps_percent_and_zero_budget_is_spend_only() {
    let key = binding(json!({"user_id": "user-1"}));
    let over = user_result(
        &key,
        json!({"user_info": {"spend": 150.0, "max_budget": 100.0}}),
    )
    .unwrap();
    assert_eq!(over.usage.primary.used_percent, 100.0);
    let unbudgeted = user_result(
        &key,
        json!({"user_info": {"spend": 4.0, "max_budget": 0.0}}),
    )
    .unwrap();
    assert_eq!(unbudgeted.usage.primary.reset_description, None);
    assert_eq!(unbudgeted.cost.expect("cost").period, "Personal spend");
    let empty = user_result(&key, json!({"user_info": {}})).unwrap();
    assert!(empty.cost.is_none());
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
