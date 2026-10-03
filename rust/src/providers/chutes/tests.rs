use chrono::{TimeZone, Utc};
use mockito::{Matcher, Mock, ServerGuard};
use serde_json::json;

use super::parse::{format_quota_amount, snapshot_from_usage};
use super::*;

fn utc(year: i32, month: u32, day: u32, hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
}

fn secondary(snapshot: &UsageSnapshot) -> &crate::core::RateWindow {
    snapshot
        .secondary
        .as_ref()
        .expect("secondary window should be present")
}

// --- parsing ---------------------------------------------------------------

#[test]
fn parses_ratio_window() {
    let snapshot = snapshot_from_usage(&json!({"quotas":[{"used":25,"limit":100}]}));
    assert_eq!(snapshot.primary.used_percent, 25.0);
}

#[test]
fn quota_counts_go_to_reset_description_not_schedule() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [{"used": 25, "limit": 100, "unit": "credits", "remaining": 75}]
    }));
    assert_eq!(snapshot.primary.used_percent, 25.0);
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("25/100 credits")
    );
    assert!(snapshot.primary.resets_at.is_none());
}

#[test]
fn small_numeric_fields_are_not_reset_schedules() {
    // A mis-keyed payload where "reset_time" is accidentally a remaining count.
    // Values below unix-epoch range must not become resets_at.
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [{"used": 10, "limit": 50, "reset_time": 40}]
    }));
    assert_eq!(snapshot.primary.used_percent, 20.0);
    assert!(snapshot.primary.resets_at.is_none());
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("10/50 credits")
    );
}

#[test]
fn iso_reset_timestamps_still_parse() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [{"used": 1, "limit": 2, "resets_at": "2026-08-01T00:00:00Z"}]
    }));
    assert_eq!(snapshot.primary.resets_at, Some(utc(2026, 8, 1, 0)));
}

#[test]
fn whole_percent_values_are_never_rescaled_as_fractions() {
    for (percent, expected) in [(1.0, 1.0), (0.5, 0.5), (100.0, 100.0)] {
        let snapshot = snapshot_from_usage(&json!({
            "quotas": [{"usage_percent": percent, "limit": 100}]
        }));
        assert_eq!(snapshot.primary.used_percent, expected);
    }
}

#[test]
fn exact_percent_of_one_stays_one_percent() {
    let used = snapshot_from_usage(&json!({"rolling_window": {"usage_percent": 1}}));
    let remaining = snapshot_from_usage(&json!({"rolling_window": {"percent_remaining": 1}}));
    assert_eq!(used.primary.used_percent, 1.0);
    assert_eq!(remaining.primary.used_percent, 99.0);
}

#[test]
fn large_quota_amounts_keep_their_description() {
    let snapshot = snapshot_from_usage(&json!({
        "rolling_window": {"used": 1e20, "limit": 2e20, "unit": "credits"}
    }));
    assert_eq!(snapshot.primary.used_percent, 50.0);
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("100000000000000000000/200000000000000000000 credits")
    );
}

#[test]
fn very_large_amounts_keep_the_represented_integer_digits() {
    let snapshot = snapshot_from_usage(&json!({"rolling": {"used": 1e25, "limit": 2e25}}));
    assert_eq!(snapshot.primary.used_percent, 50.0);
    assert_eq!(
        snapshot.primary.reset_description,
        Some(format!("{:.0}/{:.0} credits", 1e25, 2e25))
    );
}

#[test]
fn large_integer_amounts_preserve_native_digits() {
    let body: serde_json::Value = serde_json::from_str(
        r#"{"rolling":{"used":1234567890123456789,"limit":2469135780246913578}}"#,
    )
    .unwrap();
    let snapshot = snapshot_from_usage(&body);
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("1234567890123456768/2469135780246913536 credits")
    );
}

#[test]
fn fractional_amounts_round_like_native_printf() {
    for (value, expected) in [
        (1.125, "1.12"),
        (1.375, "1.38"),
        (1.625, "1.62"),
        (-1.125, "-1.12"),
        (49.585, "49.59"),
    ] {
        let snapshot = snapshot_from_usage(&json!({"rolling": {"used": value, "limit": 100}}));
        assert_eq!(
            snapshot.primary.reset_description,
            Some(format!("{expected}/100 credits"))
        );
    }
}

#[test]
fn duration_fields_populate_window_minutes() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [
            {"used": 25, "limit": 100, "duration": "4 hours"},
            {"used": 1, "limit": 2, "window_seconds": 1800}
        ]
    }));
    assert_eq!(snapshot.primary.window_minutes, Some(240));
    assert_eq!(secondary(&snapshot).window_minutes, Some(30));
}

#[test]
fn unrepresentable_durations_keep_usage_with_the_known_window_default() {
    for (key, value) in [
        ("window_minutes", json!(9_223_372_036_854_775_808_u64)),
        ("window_hours", json!("1e308")),
        ("window_days", json!("1e308")),
        ("window_seconds", json!("1e308")),
        ("window", json!("1e308 minutes")),
        ("window", json!("1e308 hours")),
        ("window", json!("1e308 days")),
        ("window", json!("1e308 months")),
    ] {
        let snapshot = snapshot_from_usage(&json!({
            "rolling_window": {"used": 25, "limit": 100, key: value}
        }));
        assert_eq!(snapshot.primary.used_percent, 25.0, "{key}");
        assert_eq!(snapshot.primary.window_minutes, Some(240), "{key}");
        assert_eq!(
            snapshot.primary.reset_description.as_deref(),
            Some("25/100 credits")
        );
    }
}

#[test]
fn unrepresentable_generic_quota_duration_remains_unknown() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [{"used": 25, "limit": 100, "window_minutes": 9_223_372_036_854_775_808_u64}]
    }));
    assert_eq!(snapshot.primary.used_percent, 25.0);
    assert_eq!(snapshot.primary.window_minutes, None);
}

#[test]
fn non_finite_amount_formatting_is_safe() {
    assert_eq!(format_quota_amount(f64::INFINITY), "unknown");
    assert_eq!(format_quota_amount(f64::NAN), "unknown");
}

#[test]
fn oversized_integral_amount_is_not_saturated_to_i64_max() {
    assert_eq!(format_quota_amount(2_f64.powi(63)), "9223372036854775808");
}

#[test]
fn identical_usage_values_keep_distinct_quota_windows() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [
            {"used": 0, "limit": 100, "window_minutes": 240},
            {"used": 0, "limit": 100, "window_minutes": 43200}
        ]
    }));
    assert_eq!(snapshot.primary.used_percent, 0.0);
    assert_eq!(snapshot.primary.window_minutes, Some(240));
    assert_eq!(secondary(&snapshot).used_percent, 0.0);
    assert_eq!(secondary(&snapshot).window_minutes, Some(43_200));
}

#[test]
fn distinct_raw_quotas_with_equal_percentages_remain_separate() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [
            {"used": 25, "remaining": 75, "window_minutes": 240},
            {"used": 50, "remaining": 150, "window_minutes": 240}
        ]
    }));
    assert_eq!(snapshot.primary.used_percent, 25.0);
    assert_eq!(secondary(&snapshot).used_percent, 25.0);
    assert_eq!(snapshot.primary.reset_description, None);
}

#[test]
fn active_subscription_maps_monthly_and_rolling_windows() {
    let snapshot = snapshot_from_usage(&json!({
        "subscription": {
            "active": true,
            "plan_name": "Pro",
            "current_period_end": "2026-07-01T00:00:00Z"
        },
        "monthly": {
            "used": 250,
            "limit": 1000,
            "resets_at": "2026-07-01T00:00:00Z",
            "unit": "credits"
        },
        "rolling_window": {
            "requests": 40,
            "limit": 100,
            "window_minutes": 240,
            "reset_at": "2026-06-13T18:00:00Z",
            "unit": "requests"
        }
    }));
    assert_eq!(snapshot.primary.used_percent, 40.0);
    assert_eq!(snapshot.primary.window_minutes, Some(240));
    assert_eq!(snapshot.primary.resets_at, Some(utc(2026, 6, 13, 18)));
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("40/100 requests")
    );
    let monthly = secondary(&snapshot);
    assert_eq!(monthly.used_percent, 25.0);
    assert_eq!(monthly.window_minutes, Some(43_200));
    assert_eq!(monthly.resets_at, Some(utc(2026, 7, 1, 0)));
    assert_eq!(
        monthly.reset_description.as_deref(),
        Some("250/1000 credits")
    );
    assert_eq!(
        snapshot.subscription.and_then(|s| s.renews_at),
        Some(utc(2026, 7, 1, 0))
    );
    assert_eq!(snapshot.login_method.as_deref(), Some("Pro"));
}

#[test]
fn monthly_reset_is_not_reported_as_a_subscription_renewal() {
    let snapshot = snapshot_from_usage(&json!({
        "monthly": {"used": 1, "limit": 10, "resets_at": "2026-07-01T00:00:00Z"}
    }));
    assert!(snapshot.subscription.is_none());
    assert_eq!(secondary(&snapshot).resets_at, Some(utc(2026, 7, 1, 0)));
}

#[test]
fn normalized_aliases_and_epoch_resets_preserve_quota_projection() {
    let snapshot = snapshot_from_usage(&json!({"result": {
        "plan_name": "Fixture",
        "rolling_4h": {"used_percent": "25", "reset_at": "1800000000000"},
        "billing_period": {"remaining": "$1,500", "cap": "2,000", "duration": "1 month"}
    }}));
    assert_eq!(snapshot.primary.used_percent, 25.0);
    assert_eq!(
        snapshot.primary.resets_at,
        Utc.timestamp_opt(1_800_000_000, 0).single()
    );
    let monthly = secondary(&snapshot);
    assert_eq!(monthly.used_percent, 25.0);
    assert_eq!(
        monthly.reset_description.as_deref(),
        Some("500/2000 credits")
    );
    assert_eq!(monthly.window_minutes, Some(43_200));
    assert_eq!(snapshot.login_method.as_deref(), Some("Fixture"));
}

#[test]
fn rolling_and_monthly_are_classified_by_label_or_window_length() {
    for quotas in [
        json!([
            {"name": "Monthly credits", "used": 30, "limit": 100},
            {"name": "4-hour usage", "used": 10, "limit": 100}
        ]),
        json!([
            {"used": 30, "limit": 100, "window_minutes": 43200},
            {"used": 10, "limit": 100, "window_minutes": 240}
        ]),
        json!([
            {"used": 30, "limit": 100, "window_days": 28},
            {"used": 10, "limit": 100, "window": "4h"}
        ]),
    ] {
        let snapshot = snapshot_from_usage(&json!({ "quotas": quotas }));
        assert_eq!(snapshot.primary.used_percent, 10.0, "{quotas}");
        assert_eq!(snapshot.primary.window_minutes, Some(240), "{quotas}");
        assert_eq!(secondary(&snapshot).used_percent, 30.0, "{quotas}");
        assert!(
            secondary(&snapshot).window_minutes >= Some(40_320),
            "{quotas}"
        );
    }
}

#[test]
fn payload_lanes_default_their_window_minutes() {
    let snapshot = snapshot_from_usage(&json!({
        "fourhour": {"used": 1, "limit": 10},
        "monthly": {"used": 2, "limit": 10}
    }));
    assert_eq!(snapshot.primary.window_minutes, Some(240));
    assert_eq!(secondary(&snapshot).window_minutes, Some(43_200));
}

#[test]
fn a_single_rolling_lane_is_not_repeated_as_the_secondary() {
    let snapshot = snapshot_from_usage(&json!({"rolling": {"used": 1, "limit": 10}}));
    assert_eq!(snapshot.primary.used_percent, 10.0);
    assert!(snapshot.secondary.is_none());
}

#[test]
fn unclassified_windows_fill_primary_then_secondary() {
    let snapshot = snapshot_from_usage(&json!({
        "quotas": [
            {"used": 1, "limit": 10},
            {"used": 2, "limit": 10},
            {"used": 3, "limit": 10}
        ]
    }));
    assert_eq!(snapshot.primary.used_percent, 10.0);
    assert_eq!(secondary(&snapshot).used_percent, 20.0);

    let with_rolling = snapshot_from_usage(&json!({
        "rolling": {"used": 5, "limit": 10},
        "other": {"used": 2, "limit": 10, "name": "extra"}
    }));
    assert_eq!(with_rolling.primary.used_percent, 50.0);
    assert_eq!(secondary(&with_rolling).used_percent, 20.0);

    let with_monthly = snapshot_from_usage(&json!({
        "monthly": {"used": 5, "limit": 10},
        "other": {"used": 2, "limit": 10, "name": "extra"}
    }));
    assert!(with_monthly.primary.is_informational);
    assert_eq!(secondary(&with_monthly).used_percent, 50.0);
}

#[test]
fn missing_usage_fields_return_a_no_data_snapshot() {
    let snapshot = snapshot_from_usage(&json!({
        "subscription": {"active": true},
        "unexpected": {"nested": true}
    }));
    assert!(snapshot.primary.is_informational);
    assert!(snapshot.secondary.is_none());
    assert_eq!(snapshot.login_method, None);

    let unknown = snapshot_from_usage(&json!({}));
    assert!(unknown.primary.is_informational);
    assert_eq!(unknown.login_method.as_deref(), Some("No usage data"));
}

#[test]
fn inactive_subscription_is_reported_and_plan_name_wins() {
    for payload in [
        json!({"subscription": {"active": false}}),
        json!({"subscription": {"active": "no"}}),
        json!({"active": 0}),
        json!({"status": "free"}),
        json!({"data": {"subscription": {"status": "Cancelled"}}}),
        json!({"state": "expired"}),
    ] {
        let snapshot = snapshot_from_usage(&payload);
        assert_eq!(
            snapshot.login_method.as_deref(),
            Some("No active subscription"),
            "{payload}"
        );
    }
    let active = snapshot_from_usage(&json!({"status": "Active"}));
    assert_eq!(active.login_method, None);
    let inactive_plan = snapshot_from_usage(&json!({"active": false, "tier": "Basic"}));
    assert_eq!(inactive_plan.login_method.as_deref(), Some("Basic"));
}

// --- endpoint construction and HTTP flow -------------------------------------

fn client() -> Client {
    Client::builder()
        .no_proxy()
        .build()
        .expect("the test client should build")
}

async fn fetch(server: &ServerGuard, path_and_query: &str) -> Result<UsageSnapshot, ProviderError> {
    let base = Url::parse(&format!("{}{path_and_query}", server.url()))
        .expect("the mock server URL should be valid");
    fetch_usage_snapshot(&client(), &base, "chutes-key").await
}

async fn respond(
    server: &mut ServerGuard,
    path: &str,
    status: usize,
    body: &str,
    hits: usize,
) -> Mock {
    server
        .mock("GET", path)
        .match_header("authorization", "Bearer chutes-key")
        .match_header("accept", "application/json")
        .with_status(status)
        .with_body(body)
        .expect(hits)
        .create_async()
        .await
}

#[test]
fn endpoint_keeps_the_configured_path_and_query() {
    let base = Url::parse("https://chutes.test/proxy/?tenant=fixture#ignored").unwrap();
    assert_eq!(
        endpoint(&base, &["quota_usage", "a/b c"]).unwrap().as_str(),
        "https://chutes.test/proxy/users/me/quota_usage/a%2Fb%20c?tenant=fixture"
    );
    let bare = Url::parse("https://api.chutes.ai").unwrap();
    assert_eq!(
        endpoint(&bare, &["subscription_usage"]).unwrap().as_str(),
        "https://api.chutes.ai/users/me/subscription_usage"
    );
}

#[test]
fn endpoint_override_must_be_https() {
    assert!(crate::providers::validated_https_url("http://chutes.test", "Chutes API").is_err());
    let with_query = crate::providers::validated_https_url("chutes.test/p?t=1", "Chutes API")
        .expect("a bare host is accepted as HTTPS");
    assert_eq!(with_query.as_str(), "https://chutes.test/p?t=1");
}

#[tokio::test]
async fn complete_subscription_usage_needs_a_single_request() {
    let mut server = mockito::Server::new_async().await;
    let usage = respond(
        &mut server,
        "/users/me/subscription_usage",
        200,
        r#"{"subscription":{"active":true,"plan_name":"Pro"},
            "monthly":{"used":250,"limit":1000},
            "rolling_window":{"requests":40,"limit":100,"window_minutes":240}}"#,
        1,
    )
    .await;
    let quotas = respond(&mut server, "/users/me/quotas", 200, "{}", 0).await;

    let snapshot = fetch(&server, "").await.unwrap();

    assert_eq!(snapshot.primary.used_percent, 40.0);
    assert_eq!(secondary(&snapshot).used_percent, 25.0);
    assert_eq!(snapshot.login_method.as_deref(), Some("Pro"));
    usage.assert_async().await;
    quotas.assert_async().await;
}

#[tokio::test]
async fn inactive_subscription_falls_back_to_the_quotas_endpoint() {
    let mut server = mockito::Server::new_async().await;
    let mocks = [
        respond(
            &mut server,
            "/users/me/subscription_usage",
            200,
            r#"{"subscription":{"active":false,"status":"free"}}"#,
            1,
        )
        .await,
        respond(
            &mut server,
            "/users/me/quotas",
            200,
            r#"[{"chute_id":"0","is_default":true,"quota":100}]"#,
            1,
        )
        .await,
        respond(
            &mut server,
            "/users/me/quota_usage/0",
            200,
            r#"{"quota":100,"used":10}"#,
            1,
        )
        .await,
    ];

    let snapshot = fetch(&server, "").await.unwrap();

    assert_eq!(snapshot.primary.used_percent, 10.0);
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("10/100 credits")
    );
    assert!(snapshot.secondary.is_none());
    assert_eq!(
        snapshot.login_method.as_deref(),
        Some("No active subscription")
    );
    for mock in mocks {
        mock.assert_async().await;
    }
}

#[tokio::test]
async fn wrapped_quota_list_fetches_per_quota_usage() {
    let mut server = mockito::Server::new_async().await;
    let mocks = [
        respond(
            &mut server,
            "/users/me/subscription_usage",
            200,
            r#"{"subscription":{"active":false}}"#,
            1,
        )
        .await,
        respond(
            &mut server,
            "/users/me/quotas",
            200,
            r#"{"quotas":{"metadata":true},"data":[{"chute_id":"wrapped","quota":200}]}"#,
            1,
        )
        .await,
        respond(
            &mut server,
            "/users/me/quota_usage/wrapped",
            200,
            r#"{"data":{"quota":200,"used":50}}"#,
            1,
        )
        .await,
    ];

    let snapshot = fetch(&server, "").await.unwrap();

    assert_eq!(snapshot.primary.used_percent, 25.0);
    for mock in mocks {
        mock.assert_async().await;
    }
}

#[tokio::test]
async fn partial_subscription_usage_fills_the_missing_rolling_window() {
    let mut server = mockito::Server::new_async().await;
    let mocks = [
        respond(
            &mut server,
            "/users/me/subscription_usage",
            200,
            r#"{"subscription":{"active":true,"plan_name":"Pro",
                "current_period_end":"2026-07-01T00:00:00Z"},
                "monthly":{"used":250,"limit":1000,"unit":"credits"}}"#,
            1,
        )
        .await,
        respond(
            &mut server,
            "/users/me/quotas",
            200,
            r#"{"rolling_window":{"requests":40,"limit":100,"window_minutes":240,
                "unit":"requests"}}"#,
            1,
        )
        .await,
    ];

    let snapshot = fetch(&server, "").await.unwrap();

    assert_eq!(snapshot.primary.used_percent, 40.0);
    assert_eq!(snapshot.primary.window_minutes, Some(240));
    assert_eq!(
        snapshot.primary.reset_description.as_deref(),
        Some("40/100 requests")
    );
    assert_eq!(secondary(&snapshot).used_percent, 25.0);
    assert_eq!(
        secondary(&snapshot).reset_description.as_deref(),
        Some("250/1000 credits")
    );
    assert_eq!(snapshot.login_method.as_deref(), Some("Pro"));
    for mock in mocks {
        mock.assert_async().await;
    }
}

#[tokio::test]
async fn quota_enrichment_is_best_effort_except_for_rejected_credentials() {
    for failing in ["quotas", "quota_usage/fixture"] {
        for status in [403_usize, 500] {
            let mut server = mockito::Server::new_async().await;
            let failing_path = format!("/users/me/{failing}");
            let quota_usage_hits = usize::from(failing != "quotas");
            let mocks = [
                respond(
                    &mut server,
                    "/users/me/subscription_usage",
                    200,
                    r#"{"subscription":{"active":false}}"#,
                    1,
                )
                .await,
                respond(
                    &mut server,
                    "/users/me/quotas",
                    if failing == "quotas" { status } else { 200 },
                    if failing == "quotas" {
                        "upstream error"
                    } else {
                        r#"{"quotas":[{"chute_id":"fixture","limit":100}]}"#
                    },
                    1,
                )
                .await,
                respond(
                    &mut server,
                    "/users/me/quota_usage/fixture",
                    status,
                    "upstream error",
                    quota_usage_hits,
                )
                .await,
            ];

            let result = fetch(&server, "").await;

            if status == 403 {
                assert!(
                    matches!(result, Err(ProviderError::AuthRequired)),
                    "{failing_path}"
                );
            } else {
                let snapshot = result.unwrap();
                assert!(snapshot.primary.is_informational, "{failing_path}");
                assert_eq!(
                    snapshot.login_method.as_deref(),
                    Some("No active subscription")
                );
            }
            for mock in mocks {
                mock.assert_async().await;
            }
        }
    }
}

#[tokio::test]
async fn configured_base_path_and_query_survive_every_request() {
    let mut server = mockito::Server::new_async().await;
    let query = Matcher::UrlEncoded("tenant".into(), "fixture".into());
    let usage = server
        .mock("GET", "/proxy/users/me/subscription_usage")
        .match_query(query.clone())
        .with_status(200)
        .with_body("{}")
        .expect(1)
        .create_async()
        .await;
    let quotas = server
        .mock("GET", "/proxy/users/me/quotas")
        .match_query(query)
        .with_status(200)
        .with_body("{}")
        .expect(1)
        .create_async()
        .await;

    let snapshot = fetch(&server, "/proxy?tenant=fixture").await.unwrap();

    assert_eq!(snapshot.login_method.as_deref(), Some("No usage data"));
    usage.assert_async().await;
    quotas.assert_async().await;
}

#[tokio::test]
async fn invalid_subscription_json_fails_but_optional_invalid_json_does_not() {
    let mut server = mockito::Server::new_async().await;
    let _usage = respond(
        &mut server,
        "/users/me/subscription_usage",
        200,
        "not JSON",
        1,
    )
    .await;
    assert!(matches!(
        fetch(&server, "").await,
        Err(ProviderError::Parse(_))
    ));

    let mut server = mockito::Server::new_async().await;
    let _usage = respond(&mut server, "/users/me/subscription_usage", 200, "{}", 1).await;
    let _quotas = respond(&mut server, "/users/me/quotas", 200, "not JSON", 1).await;
    let snapshot = fetch(&server, "").await.unwrap();
    assert!(snapshot.primary.is_informational);
}

#[tokio::test]
async fn non_container_json_and_http_errors_are_reported() {
    let mut server = mockito::Server::new_async().await;
    let _usage = respond(&mut server, "/users/me/subscription_usage", 200, "42", 1).await;
    assert!(matches!(
        fetch(&server, "").await,
        Err(ProviderError::Parse(_))
    ));

    let mut server = mockito::Server::new_async().await;
    let _usage = respond(&mut server, "/users/me/subscription_usage", 503, "", 1).await;
    let error = fetch(&server, "").await.unwrap_err();
    assert!(matches!(
        error,
        ProviderError::Other(message) if message == "Chutes usage API error: HTTP 503"
    ));
}

#[tokio::test]
async fn rejected_key_on_subscription_usage_requires_auth() {
    for status in [401, 403] {
        let mut server = mockito::Server::new_async().await;
        let _usage = respond(
            &mut server,
            "/users/me/subscription_usage",
            status,
            r#"{"detail":"unauthorized"}"#,
            1,
        )
        .await;
        assert!(matches!(
            fetch(&server, "").await,
            Err(ProviderError::AuthRequired)
        ));
    }
}
