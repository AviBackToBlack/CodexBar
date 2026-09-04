use super::*;
use chrono::{DateTime, Utc};
use reqwest::{Client, Url};

fn billing(
    quota_balance: Option<f64>,
    on_demand: Option<DevinOnDemandUsage>,
    status: Option<DevinBillingStatus>,
) -> DevinBillingEnrichment {
    DevinBillingEnrichment {
        quota_balance,
        on_demand,
        status,
    }
}

fn mock_internal_org(server: &mockito::Server) -> DevinOrganization {
    DevinOrganization {
        display: "org-abc123".to_string(),
        request_url: Url::parse(&format!(
            "{}/api/org-abc123/billing/quota/usage",
            server.url()
        ))
        .unwrap(),
        x_cog_org_id: Some("org-abc123".to_string()),
    }
}

#[test]
fn normalizes_raw_bearer_token() {
    assert_eq!(
        normalize_bearer_token("auth1_xxx"),
        Some("auth1_xxx".to_string())
    );
}

#[test]
fn normalizes_bearer_prefixed_token() {
    assert_eq!(
        normalize_bearer_token("Bearer auth1_xxx"),
        Some("auth1_xxx".to_string())
    );
}

#[test]
fn normalizes_authorization_header_token() {
    assert_eq!(
        normalize_bearer_token("Authorization: Bearer auth1_xxx"),
        Some("auth1_xxx".to_string())
    );
}

#[test]
fn one_character_quote_inputs_do_not_panic() {
    assert_eq!(normalize_bearer_token("'"), Some("'".to_string()));
    assert_eq!(normalize_bearer_token("\""), Some("\"".to_string()));
}

#[test]
fn normalizes_org_hyphen_id_to_canonical_api_url() {
    let org = normalize_organization("org-abc123").unwrap();
    assert_eq!(org.display(), "org-abc123");
    assert_eq!(org.x_cog_org_id(), Some("org-abc123"));
    assert_eq!(
        org.request_url(),
        &Url::parse("https://app.devin.ai/api/org-abc123/billing/quota/usage").unwrap()
    );
}

#[test]
fn normalizes_org_underscore_id_to_canonical_api_url() {
    let org = normalize_organization("org_abc123").unwrap();
    assert_eq!(org.display(), "org_abc123");
    assert_eq!(org.x_cog_org_id(), Some("org_abc123"));
    assert_eq!(
        org.request_url(),
        &Url::parse("https://app.devin.ai/api/org_abc123/billing/quota/usage").unwrap()
    );
}

#[test]
fn parses_plain_slug() {
    let org = normalize_organization("demo-team").unwrap();
    assert_eq!(org.display(), "org/demo-team");
    assert_eq!(org.x_cog_org_id(), None);
    assert_eq!(
        org.request_url(),
        &Url::parse("https://app.devin.ai/api/org/demo-team/billing/quota/usage").unwrap()
    );
}

#[test]
fn parses_org_prefixed_slug() {
    let org = normalize_organization("org/demo-team").unwrap();
    assert_eq!(org.display(), "org/demo-team");
    assert_eq!(org.x_cog_org_id(), None);
    assert_eq!(
        org.request_url(),
        &Url::parse("https://app.devin.ai/api/org/demo-team/billing/quota/usage").unwrap()
    );
}

#[test]
fn parses_organizations_internal_id() {
    let org = normalize_organization("organizations/org-abc123").unwrap();
    assert_eq!(org.display(), "org-abc123");
    assert_eq!(org.x_cog_org_id(), Some("org-abc123"));
    assert_eq!(
        org.request_url(),
        &Url::parse("https://app.devin.ai/api/org-abc123/billing/quota/usage").unwrap()
    );
}

#[test]
fn parses_devin_organization_url() {
    let org = normalize_organization("https://app.devin.ai/org/demo-team").unwrap();
    assert_eq!(org.display(), "org/demo-team");
    assert_eq!(org.x_cog_org_id(), None);
}

#[test]
fn parses_devin_api_url() {
    let org =
        normalize_organization("https://app.devin.ai/api/org-abc123/billing/quota/usage").unwrap();
    assert_eq!(org.display(), "org-abc123");
    assert_eq!(org.x_cog_org_id(), Some("org-abc123"));
}

#[test]
fn generates_on_demand_usage_url() {
    let org = normalize_organization("org-abc123").unwrap();
    assert_eq!(
        org.on_demand_request_url().as_str(),
        "https://app.devin.ai/api/org-abc123/billing/quota/my-on-demand-usage"
    );
}

#[test]
fn generates_status_url() {
    let org = normalize_organization("org-abc123").unwrap();
    assert_eq!(
        org.status_request_url().as_str(),
        "https://app.devin.ai/api/org-abc123/billing/status"
    );
}

#[test]
fn builds_devin_headers_for_internal_org_requests() {
    let org = normalize_organization("org-abc123").unwrap();
    for (url, expected) in [
        (
            org.request_url().clone(),
            "https://app.devin.ai/api/org-abc123/billing/quota/usage",
        ),
        (
            org.on_demand_request_url(),
            "https://app.devin.ai/api/org-abc123/billing/quota/my-on-demand-usage",
        ),
        (
            org.status_request_url(),
            "https://app.devin.ai/api/org-abc123/billing/status",
        ),
    ] {
        let request = send_devin_get(&Client::new(), "auth1_xxx", &org, url)
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), expected);
        assert_eq!(request.headers().get("accept").unwrap(), "application/json");
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer auth1_xxx"
        );
        assert_eq!(request.headers().get("x-cog-org-id").unwrap(), "org-abc123");
    }
}

#[test]
fn rejects_invalid_internal_org_in_organizations_prefix() {
    assert!(normalize_organization("organizations/foo").is_none());
}

#[test]
fn rejects_invalid_trailing_segments() {
    assert!(normalize_organization("org/foo/garbage").is_none());
    assert!(normalize_organization("organizations/org-abc/garbage").is_none());
}

#[test]
fn rejects_non_exact_devin_urls() {
    assert!(normalize_organization("http://app.devin.ai/org/demo-team").is_none());
    assert!(normalize_organization("https://user@app.devin.ai/org/demo-team").is_none());
    assert!(normalize_organization("https://app.devin.ai.evil.example/org/demo-team").is_none());
    assert!(normalize_organization("https://app.devin.ai/org/demo-team?x=1").is_none());
    assert!(normalize_organization("https://app.devin.ai/org/demo-team#frag").is_none());
}

#[test]
fn preserves_percentage_semantics_exactly() {
    assert_eq!(
        percent(
            &serde_json::json!({"daily_percentage": 0.0}),
            &["daily_percentage"]
        ),
        Some(0.0)
    );
    assert_eq!(
        percent(
            &serde_json::json!({"daily_percentage": 0.25}),
            &["daily_percentage"]
        ),
        Some(25.0)
    );
    assert_eq!(
        percent(
            &serde_json::json!({"daily_percentage": 1.0}),
            &["daily_percentage"]
        ),
        Some(1.0)
    );
    assert_eq!(
        percent(
            &serde_json::json!({"daily_percentage": 25.0}),
            &["daily_percentage"]
        ),
        Some(25.0)
    );
    assert_eq!(
        percent(
            &serde_json::json!({"daily_percentage": 100.0}),
            &["daily_percentage"]
        ),
        Some(100.0)
    );
}

#[test]
fn parses_daily_and_weekly_resets_from_rfc3339() {
    let snapshot = snapshot_from_quota(
        &serde_json::json!({
            "daily_percentage": 0.25,
            "weekly_percentage": 0.5,
            "daily_reset_at": "2026-09-03T12:00:00Z",
            "weekly_reset_at": "2026-09-07T12:00:00Z",
        }),
        "org-abc123",
    );

    assert_eq!(snapshot.primary.used_percent, 25.0);
    assert_eq!(snapshot.primary.window_minutes, Some(1440));
    assert_eq!(
        snapshot.primary.resets_at,
        Some(
            DateTime::parse_from_rfc3339("2026-09-03T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    let weekly = snapshot.secondary.unwrap();
    assert_eq!(weekly.used_percent, 50.0);
    assert_eq!(weekly.window_minutes, Some(10080));
    assert_eq!(
        weekly.resets_at,
        Some(
            DateTime::parse_from_rfc3339("2026-09-07T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}

#[test]
fn billing_status_fallbacks_and_plan_labels_work() {
    assert_eq!(format_plan_label("pro"), "Devin Pro");
    assert_eq!(format_plan_label("team-plan"), "Devin Team Plan");

    let quota = serde_json::json!({
        "daily_percentage": 0.0,
    });
    let status = parse_billing_status(&serde_json::json!({
        "plan_slug": "team-plan",
        "overage_credits": 8.879356694312191,
    }))
    .unwrap();
    let cost = fetch_result_from_quota(&quota, "org-abc123", billing(None, None, Some(status)))
        .cost
        .unwrap();
    assert_eq!(cost.used, 0.0);
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert_eq!(cost.limit, None);
    assert_eq!(cost.period, "On-demand billing cycle");
}

#[test]
fn quota_balance_precedes_status_balance() {
    let quota = serde_json::json!({
        "daily_percentage": 0.25,
        "weekly_percentage": 0.5,
        "overage_balance": 8.879356694312191,
    });
    let status = parse_billing_status(&serde_json::json!({
        "plan_slug": "pro",
        "overage_credits": 99.0,
    }))
    .unwrap();
    let cost = fetch_result_from_quota(
        &quota,
        "org-abc123",
        billing(extra_usage_balance(&quota), None, Some(status)),
    )
    .cost
    .unwrap();
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert_eq!(cost.used, 0.0);
}

#[test]
fn missing_optional_enrichments_keep_quota_result() {
    let quota = serde_json::json!({
        "daily_percentage": 0.25,
        "weekly_percentage": 0.5,
        "daily_reset_at": "2026-09-03T12:00:00Z",
        "weekly_reset_at": "2026-09-07T12:00:00Z",
        "overage_balance": 8.879356694312191,
    });
    let result = fetch_result_from_quota(
        &quota,
        "org-abc123",
        billing(extra_usage_balance(&quota), None, None),
    );
    assert_eq!(result.usage.primary.used_percent, 25.0);
    assert_eq!(result.usage.secondary.unwrap().used_percent, 50.0);
    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 0.0);
    assert_eq!(cost.balance, Some(8.879356694312191));
}

#[tokio::test]
async fn primary_quota_unauthorized_returns_auth_required() {
    let mut server = mockito::Server::new_async().await;
    let org = mock_internal_org(&server);
    let quota_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(401)
        .create_async()
        .await;

    let result = fetch_quota(&Client::new(), "auth1_xxx", &org).await;
    quota_mock.assert_async().await;
    assert!(matches!(result, Err(ProviderError::AuthRequired)));
}

#[tokio::test]
async fn primary_quota_forbidden_returns_auth_required() {
    let mut server = mockito::Server::new_async().await;
    let org = mock_internal_org(&server);
    let quota_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(403)
        .create_async()
        .await;

    let result = fetch_quota(&Client::new(), "auth1_xxx", &org).await;
    quota_mock.assert_async().await;
    assert!(matches!(result, Err(ProviderError::AuthRequired)));
}

#[tokio::test]
async fn successful_quota_ignores_on_demand_unauthorized() {
    let mut server = mockito::Server::new_async().await;
    let org = mock_internal_org(&server);
    let quota_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"daily_percentage":0,"weekly_percentage":66,"daily_reset_at":"2026-09-04T09:00:00+01:00","weekly_reset_at":"2026-09-06T09:00:00+01:00","overage_balance":8.879356694312191}"#,
        )
        .create_async()
        .await;
    let on_demand_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/my-on-demand-usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(401)
        .create_async()
        .await;
    let status_mock = server
        .mock("GET", "/api/org-abc123/billing/status")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"plan_slug":"pro","overage_credits":8.879356694312191}"#)
        .create_async()
        .await;

    let result = fetch_quota(&Client::new(), "auth1_xxx", &org)
        .await
        .unwrap();
    quota_mock.assert_async().await;
    on_demand_mock.assert_async().await;
    status_mock.assert_async().await;

    assert_eq!(result.usage.primary.used_percent, 0.0);
    assert_eq!(result.usage.login_method.as_deref(), Some("Devin Pro"));
    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 0.0);
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert!(cost.resets_at.is_none());
}

#[tokio::test]
async fn successful_quota_ignores_status_forbidden() {
    let mut server = mockito::Server::new_async().await;
    let org = mock_internal_org(&server);
    let quota_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"daily_percentage":0,"weekly_percentage":66,"daily_reset_at":"2026-09-04T09:00:00+01:00","weekly_reset_at":"2026-09-06T09:00:00+01:00","overage_balance":8.879356694312191}"#,
        )
        .create_async()
        .await;
    let on_demand_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/my-on-demand-usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"cycle_end":"2026-09-11T19:44:42+00:00","amount":38.08}"#)
        .create_async()
        .await;
    let status_mock = server
        .mock("GET", "/api/org-abc123/billing/status")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(403)
        .create_async()
        .await;

    let result = fetch_quota(&Client::new(), "auth1_xxx", &org)
        .await
        .unwrap();
    quota_mock.assert_async().await;
    on_demand_mock.assert_async().await;
    status_mock.assert_async().await;

    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 38.08);
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert_eq!(result.usage.login_method.as_deref(), None);
    assert_eq!(
        cost.resets_at,
        Some(
            DateTime::parse_from_rfc3339("2026-09-11T19:44:42+00:00")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}

#[tokio::test]
async fn successful_quota_ignores_billing_status_unauthorized() {
    let mut server = mockito::Server::new_async().await;
    let org = mock_internal_org(&server);
    let quota_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"daily_percentage":0,"weekly_percentage":66,"daily_reset_at":"2026-09-04T09:00:00+01:00","weekly_reset_at":"2026-09-06T09:00:00+01:00","overage_balance":8.879356694312191}"#,
        )
        .create_async()
        .await;
    let on_demand_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/my-on-demand-usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"cycle_end":"2026-09-11T19:44:42+00:00","amount":38.08}"#)
        .create_async()
        .await;
    let status_mock = server
        .mock("GET", "/api/org-abc123/billing/status")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(401)
        .create_async()
        .await;

    let result = fetch_quota(&Client::new(), "auth1_xxx", &org)
        .await
        .unwrap();
    quota_mock.assert_async().await;
    on_demand_mock.assert_async().await;
    status_mock.assert_async().await;

    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 38.08);
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert_eq!(
        cost.resets_at,
        Some(
            DateTime::parse_from_rfc3339("2026-09-11T19:44:42+00:00")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert!(result.usage.login_method.is_none());
}

#[tokio::test]
async fn one_enrichment_failure_keeps_other_enrichments() {
    let mut server = mockito::Server::new_async().await;
    let org = mock_internal_org(&server);
    let quota_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"daily_percentage":0,"weekly_percentage":66,"daily_reset_at":"2026-09-04T09:00:00+01:00","weekly_reset_at":"2026-09-06T09:00:00+01:00","overage_balance":8.879356694312191}"#,
        )
        .create_async()
        .await;
    let on_demand_mock = server
        .mock("GET", "/api/org-abc123/billing/quota/my-on-demand-usage")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(500)
        .create_async()
        .await;
    let status_mock = server
        .mock("GET", "/api/org-abc123/billing/status")
        .match_header("authorization", "Bearer auth1_xxx")
        .match_header("accept", "application/json")
        .match_header("x-cog-org-id", "org-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"plan_slug":"pro","overage_credits":8.879356694312191}"#)
        .create_async()
        .await;

    let result = fetch_quota(&Client::new(), "auth1_xxx", &org)
        .await
        .unwrap();
    quota_mock.assert_async().await;
    on_demand_mock.assert_async().await;
    status_mock.assert_async().await;

    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 0.0);
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert_eq!(result.usage.login_method.as_deref(), Some("Devin Pro"));
    assert!(cost.resets_at.is_none());
}

#[test]
fn parses_on_demand_usage_cost_snapshot() {
    let on_demand = parse_on_demand_usage(&serde_json::json!({
        "cycle_start": "2026-08-11T19:44:42+00:00",
        "cycle_end": "2026-09-11T19:44:42+00:00",
        "amount": 38.08,
    }))
    .unwrap();
    let result = fetch_result_from_quota(
        &serde_json::json!({
            "daily_percentage": 0.25,
            "overage_balance": 8.879356694312191,
        }),
        "org-abc123",
        billing(
            extra_usage_balance(&serde_json::json!({
                "daily_percentage": 0.25,
                "overage_balance": 8.879356694312191,
            })),
            Some(on_demand),
            None,
        ),
    );
    let cost = result.cost.unwrap();
    assert_eq!(cost.used, 38.08);
    assert_eq!(cost.balance, Some(8.879356694312191));
    assert_eq!(cost.period, "On-demand billing cycle");
    assert_eq!(
        cost.resets_at,
        Some(
            DateTime::parse_from_rfc3339("2026-09-11T19:44:42+00:00")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}
