//! Monthly token-plan window (upstream 0.66.0, `TokenPlanMonthlyWindowTests`).

use chrono::{TimeZone, Utc};

use super::AlibabaTokenPlanProvider;
use super::cli::parse_cli_usage;
use super::personal::parse_personal_usage;
use crate::core::UsageSnapshot;

const MONTHLY: &str = r#"{"per1MonthPercentage":0.25,"per1MonthResetTime":1791043200000}"#;

fn usage(snapshot: super::TokenPlanSnapshot) -> UsageSnapshot {
    AlibabaTokenPlanProvider::snapshot_to_usage(snapshot).unwrap()
}

#[test]
fn cli_accepts_flat_and_embedded_monthly_usage() {
    let embedded = format!(r#"{{"data":{{"DataV2":{{"data":{{"data":{MONTHLY}}}}}}}}}"#);
    for payload in [MONTHLY.to_string(), embedded] {
        let usage = usage(parse_cli_usage(&payload).unwrap());
        assert_eq!(usage.primary.used_percent, 25.0);
        assert_eq!(usage.primary.window_minutes, Some(43_200));
        assert_eq!(
            usage.primary.resets_at,
            Utc.timestamp_millis_opt(1_791_043_200_000).single()
        );
        assert_eq!(usage.primary.reset_description, None);
        assert_eq!(usage.primary_label.as_deref(), Some("Monthly"));
        assert!(usage.secondary.is_none());
        assert!(usage.extra_rate_windows.is_empty());
        assert_eq!(usage.login_method.as_deref(), Some("Token Plan"));
    }
}

#[test]
fn monthly_usage_retains_rolling_windows_as_extra_row() {
    let usage = usage(
        parse_cli_usage(
            r#"{"per5HourPercentage":0.1,"per1WeekPercentage":0.2,"per1MonthPercentage":0.3}"#,
        )
        .unwrap(),
    );
    assert_eq!(usage.primary.used_percent, 10.0);
    assert_eq!(usage.primary.window_minutes, Some(300));
    assert_eq!(usage.primary_label, None);
    let secondary = usage.secondary.as_ref().unwrap();
    assert_eq!(secondary.used_percent, 20.0);
    assert_eq!(secondary.window_minutes, Some(10_080));
    let [extra] = usage.extra_rate_windows.as_slice() else {
        panic!("expected exactly one extra window");
    };
    assert_eq!(extra.id, "monthly");
    assert_eq!(extra.title, "Monthly");
    assert!((extra.window.used_percent - 30.0).abs() < 1e-9);
    assert_eq!(extra.window.window_minutes, Some(43_200));
}

#[test]
fn web_monthly_quota_preserves_totals_and_plan_identity() {
    let snapshot = parse_personal_usage(
        MONTHLY.as_bytes(),
        Some(br#"{"data":{"specCode":"standard"}}"#),
        Some(br#"{"standard":{"monthly":45000}}"#),
    )
    .unwrap();
    let usage = usage(snapshot);
    assert_eq!(usage.primary.used_percent, 25.0);
    assert_eq!(usage.primary.window_minutes, Some(43_200));
    assert_eq!(
        usage.primary.reset_description.as_deref(),
        Some("11,250 / 45,000 credits used")
    );
    assert_eq!(usage.login_method.as_deref(), Some("Standard"));
}

#[test]
fn monthly_without_quota_config_has_no_reset_description() {
    let snapshot = parse_personal_usage(
        MONTHLY.as_bytes(),
        Some(br#"{"data":{"specCode":"standard"}}"#),
        Some(br#"{"standard":{"weekly":9000}}"#),
    )
    .unwrap();
    assert_eq!(usage(snapshot).primary.reset_description, None);
}

#[test]
fn cli_rejects_invalid_monthly_ratios() {
    for ratio in ["true", "-0.1", "1.5", r#""0.25""#, "null"] {
        let payload = format!(r#"{{"per1MonthPercentage":{ratio}}}"#);
        assert!(parse_cli_usage(&payload).is_err(), "accepted {ratio}");
    }
}

#[test]
fn invalid_monthly_ratio_does_not_erase_a_valid_weekly_window() {
    let snapshot = parse_cli_usage(
        r#"{"per1WeekPercentage":0.2,"per1MonthPercentage":true,"per1MonthResetTime":1791043200000}"#,
    )
    .unwrap();
    assert_eq!(snapshot.weekly_used_percent, Some(20.0));
    assert_eq!(snapshot.monthly_used_percent, None);
    assert_eq!(snapshot.monthly_resets_at, None);
    let usage = usage(snapshot);
    assert_eq!(usage.primary.window_minutes, Some(10_080));
    assert!(usage.secondary.is_none());
    assert!(usage.extra_rate_windows.is_empty());
}

#[test]
fn shared_mapping_keeps_web_coercion_and_cli_reset_validation() {
    let payload = r#"{"per5HourPercentage":"invalid","per5HourResetTime":1791043200000,"per1WeekPercentage":0.2}"#;
    let web = parse_personal_usage(payload.as_bytes(), None, None).unwrap();
    let cli = parse_cli_usage(payload).unwrap();
    assert_eq!(web.five_hour_used_percent, None);
    assert_eq!(
        web.five_hour_resets_at,
        Utc.timestamp_millis_opt(1_791_043_200_000).single()
    );
    assert_eq!(cli.five_hour_resets_at, None);
    assert_eq!(cli.weekly_used_percent, web.weekly_used_percent);
}

#[test]
fn monthly_only_success_envelope_is_not_transient() {
    let payload = r#"{"code":"SUCCESS","successResponse":true,"errorCode":"","data":{"per1MonthPercentage":0.5}}"#;
    assert!(!super::personal::personal_usage_success_without_windows(
        payload.as_bytes()
    ));
}
