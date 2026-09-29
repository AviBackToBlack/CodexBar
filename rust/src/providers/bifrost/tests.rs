use super::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn component_rate_limits_take_precedence_over_aggregate_compatibility_field() {
    let value = json!({
        "rate_limit": { "token_max_limit": 900, "token_current_usage": 450, "token_reset_duration": "1h" },
        "rate_limits": [
            { "source_name": "provider-a", "token_max_limit": 100, "token_current_usage": 25, "token_reset_duration": "1h" },
            { "source_name": "provider-b", "request_max_limit": 20, "request_current_usage": 5, "request_reset_duration": "1d" }
        ]
    });
    let parsed = parse_usage(&value, now()).unwrap();
    assert_eq!(parsed.limits.len(), 2);
    assert_eq!(parsed.limits[0].1["token_max_limit"], 100);
    assert_eq!(parsed.limits[1].1["request_max_limit"], 20);
    assert!(
        !parsed
            .limits
            .iter()
            .any(|(_, limit, _)| limit["token_max_limit"] == 900)
    );
}

#[test]
fn aggregate_rate_limit_is_fallback_when_components_are_absent() {
    let parsed = parse_usage(&json!({
        "rate_limit": { "token_max_limit": 50, "token_current_usage": 10, "token_reset_duration": "1h" }
    }), now()).unwrap();
    assert_eq!(parsed.limits.len(), 1);
    assert_eq!(parsed.limits[0].1["token_max_limit"], 50);
}

#[test]
fn missing_current_usage_at_a_positive_rate_limit_is_known_zero() {
    for (usage_field, id) in [
        ("token_current_usage", "bifrost-tokens-0"),
        ("request_current_usage", "bifrost-requests-0"),
    ] {
        let mut limit = json!({
            "token_max_limit": 100,
            "token_current_usage": 25,
            "request_max_limit": 20,
            "request_current_usage": 5
        });
        limit.as_object_mut().unwrap().remove(usage_field);
        let result =
            result_from_usage(parse_usage(&json!({ "rate_limit": limit }), now()).unwrap());
        let named = result
            .usage
            .extra_rate_windows
            .iter()
            .find(|window| window.id == id)
            .unwrap_or_else(|| panic!("missing named rate window {id}"));

        assert_eq!(named.window.used_percent, 0.0, "{usage_field}");
        assert!(named.usage_known, "{usage_field}");
        assert!(named.window.usage_known(), "{usage_field}");
    }
}

#[test]
fn validates_gateway_before_request_url_is_built() {
    assert!(
        quota_url_for_test("https://bifrost.example.com/base/")
            .unwrap()
            .as_str()
            .starts_with("https://bifrost.example.com/base/api/governance/virtual-keys/quota")
    );
    assert!(quota_url_for_test("http://10.1.2.3:8080").is_ok());
    assert!(quota_url_for_test("http://bifrost.example.com").is_err());
    assert!(quota_url_for_test("https://user:secret@bifrost.example.com").is_err());
    assert!(quota_url_for_test("ftp://10.1.2.3").is_err());
}

#[tokio::test]
async fn rejects_public_http_before_resolving_any_credential() {
    let provider = BifrostProvider::new();
    let ctx = FetchContext {
        gateway_url: Some("http://public.example.com".into()),
        ..FetchContext::default()
    };
    let error = provider.fetch_api(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("must use HTTPS"));
}

#[tokio::test]
async fn does_not_forward_virtual_key_through_gateway_redirects() {
    let redirect_target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_target_addr = redirect_target.local_addr().unwrap();
    let gateway = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_addr = gateway.local_addr().unwrap();
    let gateway_task = tokio::spawn(async move {
        let (mut stream, _) = gateway.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let read = stream.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
        assert!(request.contains("x-bf-vk: test-virtual-key"));
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{redirect_target_addr}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let redirect_target_task =
        tokio::spawn(
            async move { timeout(Duration::from_millis(250), redirect_target.accept()).await },
        );

    let provider = BifrostProvider::new();
    let ctx = FetchContext {
        gateway_url: Some(format!("http://{gateway_addr}")),
        api_key: Some("test-virtual-key".into()),
        ..FetchContext::default()
    };
    let error = provider.fetch_api(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("HTTP 302"));
    gateway_task.await.unwrap();
    assert!(redirect_target_task.await.unwrap().is_err());
}

#[test]
fn parses_fixed_reset_and_calendar_label_without_inventing_calendar_time() {
    let fixed = reset_timing(Some("1h"), Some("2025-12-31T23:30:00Z"), now());
    assert_eq!(fixed.window_minutes, Some(60));
    assert_eq!(
        fixed.resets_at.unwrap().to_rfc3339(),
        "2026-01-01T00:30:00+00:00"
    );
    let calendar = reset_timing(Some("1M"), Some("2025-12-01T00:00:00Z"), now());
    assert_eq!(calendar.label, Some("Monthly"));
    assert_eq!(calendar.resets_at, None);
    assert_eq!(calendar.window_minutes, None);
    assert_eq!(parse_duration("1h30m"), Some(5_400.0));
}

#[test]
fn budgets_and_overrides_are_mapped_to_percent_and_spend() {
    let parsed = parse_usage(
        &json!({
            "virtual_key_name": "Build key",
            "budgets": [{ "id": "b1", "max_limit": 10, "current_usage": 5,
                "override_amount": 5, "override_mode": "forever", "source_name": "Team" }]
        }),
        now(),
    )
    .unwrap();
    let result = result_from_usage(parsed);
    assert!((result.usage.primary.used_percent - (100.0 / 3.0)).abs() < 0.001);
    assert_eq!(result.cost.unwrap().limit, Some(15.0));
}

#[test]
fn shortest_root_budget_is_primary_and_cost_without_summing_budgets() {
    let parsed = parse_usage(
        &json!({
            "budgets": [
                { "id": "monthly", "max_limit": 1_000, "current_usage": 200,
                    "reset_duration": "1M" },
                { "id": "daily", "max_limit": 100, "current_usage": 10,
                    "reset_duration": "1d" }
            ]
        }),
        now(),
    )
    .unwrap();
    let result = result_from_usage(parsed);

    assert_eq!(result.usage.primary.used_percent, 10.0);
    assert_eq!(
        result.usage.primary.reset_description.as_deref(),
        Some("Daily · $10.00 / $100.00")
    );
    let secondary = result.usage.secondary.as_ref().unwrap();
    assert_eq!(secondary.used_percent, 20.0);
    assert_eq!(
        secondary.reset_description.as_deref(),
        Some("Monthly · $200.00 / $1000.00")
    );

    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 10.0);
    assert_eq!(cost.limit, Some(100.0));
    assert_eq!(cost.period, "Daily");
}

#[test]
fn malformed_optional_scope_collection_fails_closed() {
    assert!(parse_usage(&json!({ "provider_configs": {} }), now()).is_err());
}

fn result_for(value: Value) -> ProviderFetchResult {
    result_from_usage(parse_usage(&value, now()).unwrap())
}

fn detail_rows(result: &ProviderFetchResult) -> Vec<(String, String, Option<String>)> {
    result
        .display_details()
        .iter()
        .filter(|row| row.id().starts_with("bifrost-model-"))
        .map(|row| {
            (
                row.title().to_owned(),
                row.value().to_owned(),
                row.secondary_value().map(str::to_owned),
            )
        })
        .collect()
}

#[test]
fn identity_uses_key_name_and_first_root_budget_source() {
    let result = result_for(json!({
        "virtual_key_name": "  Build key ",
        "budgets": [
            { "id": "monthly", "max_limit": 1_000, "current_usage": 1, "reset_duration": "1M",
                "source_name": "Monthly team" },
            { "id": "daily", "max_limit": 100, "current_usage": 1, "reset_duration": "1d",
                "source_name": "Daily team" }
        ],
        "provider_configs": [
            { "provider": "openai", "budgets": [
                { "id": "p", "max_limit": 5, "current_usage": 1, "reset_duration": "1h",
                    "source_name": "Provider team" }
            ] }
        ]
    }));
    assert_eq!(result.usage.account_email.as_deref(), Some("Build key"));
    assert_eq!(
        result.usage.account_organization.as_deref(),
        Some("Daily team")
    );
    assert_eq!(result.usage.login_method.as_deref(), Some("API"));
}

#[test]
fn identity_is_omitted_when_key_name_and_source_are_empty() {
    let result = result_for(json!({
        "virtual_key_name": "   ",
        "budgets": [{ "id": "b", "max_limit": 5, "current_usage": 1, "source_name": " " }]
    }));
    assert_eq!(result.usage.account_email, None);
    assert_eq!(result.usage.account_organization, None);
    assert!(
        parse_usage(&json!({ "virtual_key_name": 7 }), now()).is_err(),
        "a non-string key name is a malformed response"
    );
}

#[test]
fn reset_only_rate_limit_dimension_is_an_unknown_named_window() {
    let result = result_for(json!({
        "rate_limit": {
            "token_max_limit": 0, "token_current_usage": 40, "token_reset_duration": "1h",
            "request_max_limit": 0, "request_current_usage": 3,
            "request_last_reset": "2025-12-31T23:30:00Z"
        }
    }));
    let windows = &result.usage.extra_rate_windows;
    assert_eq!(
        windows.len(),
        1,
        "unconfigured request dimension is skipped"
    );
    let tokens = &windows[0];
    assert_eq!(tokens.id, "bifrost-tokens-0");
    assert_eq!(tokens.window.used_percent, 0.0);
    assert!(!tokens.usage_known);
    assert!(!tokens.window.usage_known());
    assert_eq!(tokens.window.window_minutes, Some(60));
}

#[test]
fn root_budget_extras_use_scope_and_budget_id() {
    let result = result_for(json!({
        "budgets": [
            { "id": "a", "max_limit": 10, "current_usage": 1, "reset_duration": "1h" },
            { "id": "b", "max_limit": 10, "current_usage": 1, "reset_duration": "1d" },
            { "id": "c", "max_limit": 10, "current_usage": 1, "reset_duration": "1w" }
        ],
        "model_configs": [
            { "model_name": "m", "budgets": [{ "id": "d", "max_limit": 10, "current_usage": 1 }] }
        ]
    }));
    let ids = result
        .usage
        .extra_rate_windows
        .iter()
        .map(|window| window.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["bifrost-budget-c", "bifrost-model-0-budget-d"]);
}

#[test]
fn missing_current_usage_is_a_known_zero_budget_window() {
    let result = result_for(json!({
        "budgets": [{ "id": "b", "max_limit": 10 }]
    }));
    assert_eq!(result.usage.primary.used_percent, 0.0);
    assert!(result.usage.primary.usage_known());
}

#[test]
fn model_rows_normalize_names_and_format_token_counts() {
    let result = result_for(json!({
        "budgets": [{ "id": "b", "max_limit": 10, "current_usage": 3, "per_model_usage": [
            { "model": "us.anthropic.claude-sonnet-4-20250514-v1:0", "provider": "bedrock",
                "total_cost": 2.5, "total_tokens": 1_250_000 },
            { "model": "gpt-4o-2024-08-06", "provider": "bedrock",
                "total_cost": 0.5, "total_tokens": 999 },
            { "model": "idle", "total_cost": 0, "total_tokens": 0 }
        ] }]
    }));
    assert_eq!(
        detail_rows(&result),
        [
            (
                "claude-sonnet-4".to_owned(),
                "$2.50".to_owned(),
                Some("1.3M tokens".to_owned())
            ),
            (
                "gpt-4o".to_owned(),
                "$0.50".to_owned(),
                Some("999 tokens".to_owned())
            ),
        ]
    );
}

#[test]
fn model_rows_prefix_provider_only_when_providers_are_mixed() {
    let result = result_for(json!({
        "budgets": [{ "id": "b", "max_limit": 10, "current_usage": 3, "per_model_usage": [
            { "model": "eu.mistral.large-v1:0", "provider": "bedrock", "total_cost": 2 },
            { "model": "gpt-4o", "provider": "openai", "total_cost": 1 }
        ] }]
    }));
    let titles = detail_rows(&result)
        .into_iter()
        .map(|row| row.0)
        .collect::<Vec<_>>();
    assert_eq!(titles, ["bedrock · large", "openai · gpt-4o"]);
}

#[test]
fn model_rows_use_raw_name_when_short_names_collide() {
    let result = result_for(json!({
        "budgets": [{ "id": "b", "max_limit": 10, "current_usage": 3, "per_model_usage": [
            { "model": "us.anthropic.claude-3-haiku-20240307-v1:0", "total_cost": 3 },
            { "model": "claude-3-haiku-20240307", "total_cost": 2 },
            { "model": "gpt-4o", "total_cost": 1 }
        ] }]
    }));
    let titles = detail_rows(&result)
        .into_iter()
        .map(|row| row.0)
        .collect::<Vec<_>>();
    assert_eq!(
        titles,
        [
            "us.anthropic.claude-3-haiku-20240307-v1:0",
            "claude-3-haiku-20240307",
            "gpt-4o"
        ]
    );
}

#[test]
fn model_rows_cap_at_five_with_an_other_models_count() {
    let models = (0..7)
        .map(|index| json!({ "model": format!("m{index}"), "total_cost": 10 - index }))
        .collect::<Vec<_>>();
    let result = result_for(json!({
        "budgets": [{ "id": "b", "max_limit": 10, "current_usage": 3, "per_model_usage": models }]
    }));
    let rows = detail_rows(&result);
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[5], ("Other models".to_owned(), "2".to_owned(), None));
}
