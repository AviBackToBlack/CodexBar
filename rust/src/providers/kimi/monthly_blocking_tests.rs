//! Upstream 0.69.0 `KimiMonthlyBlockingTests` (#4091) through the Kimi web
//! parser: a known, exhausted monthly membership pool blocks the shorter Code
//! windows without rewriting their raw usage. The card-level checks (status
//! text, no reset or pace) live with the menu card and settings tests. The
//! Code API monthly pool (upstream 0.60.5 #3694) is the same `Total usage`
//! lane, so it blocks the same way without web auth.

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use serde_json::json;

use super::{
    KimiCodeApiUsageResponse, KimiSubscriptionStatsResponse, KimiWebUsageResponse,
    MONTHLY_WINDOW_ID, code_api, web,
};
use crate::core::{BlockedWindows, ProviderId, UsageSnapshot};

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_788_000_000, 0).single().unwrap()
}

fn iso(offset: Duration) -> String {
    (now() + offset).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Upstream `snapshot(ratio:)`: #3536 reports `amountUsedRatio = 1` while both
/// Code counters are zero.
fn snapshot(ratio: f64) -> UsageSnapshot {
    let usage: KimiWebUsageResponse = serde_json::from_value(json!({
        "usages": [{
            "scope": "FEATURE_CODING",
            "detail": {
                "limit": "100",
                "used": "0",
                "remaining": "100",
                "resetTime": iso(Duration::days(4) + Duration::hours(9))
            },
            "limits": [{
                "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                "detail": {
                    "limit": "100",
                    "used": "0",
                    "remaining": "100",
                    "resetTime": iso(Duration::hours(1))
                }
            }]
        }]
    }))
    .unwrap();
    let stats: KimiSubscriptionStatsResponse = serde_json::from_value(json!({
        "subscriptionBalance": {
            "amountUsedRatio": ratio,
            "expireTime": iso(Duration::days(30)),
            "overdrawn": true
        },
        "ratelimitCode7d": { "ratio": 0.25, "enabled": true }
    }))
    .unwrap();
    web::snapshot_from_web_usage_response(usage, Some(stats)).unwrap()
}

fn extra_flag(blocked: &BlockedWindows, usage: &UsageSnapshot, id: &str) -> bool {
    let index = usage
        .extra_rate_windows
        .iter()
        .position(|window| window.id == id)
        .unwrap_or_else(|| panic!("missing extra window {id}"));
    blocked.extra[index]
}

#[test]
fn exhausted_membership_blocks_fresh_code_windows_without_changing_raw_usage() {
    let usage = snapshot(1.0);
    let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());

    assert!(blocked.primary && blocked.secondary);
    assert!(extra_flag(&blocked, &usage, "kimi-code-7d"));
    assert!(!extra_flag(&blocked, &usage, MONTHLY_WINDOW_ID));
    // The pool's reset lifts the block; the shorter resets cannot restore access.
    assert_eq!(blocked.resets_at, Some(now() + Duration::days(30)));
    assert_eq!(usage.primary.used_percent, 0.0);
    assert_eq!(usage.secondary.as_ref().unwrap().used_percent, 0.0);
}

#[test]
fn available_membership_preserves_code_windows() {
    for ratio in [0.5, 0.999] {
        let usage = snapshot(ratio);
        let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());
        assert!(!blocked.any(), "ratio {ratio} must not block");
        assert_eq!(blocked.resets_at, None);
    }
}

#[test]
fn membership_block_ends_at_the_pool_reset() {
    let usage = snapshot(1.0);
    let at_reset = now() + Duration::days(30);
    assert!(
        BlockedWindows::evaluate(ProviderId::Kimi, &usage, at_reset - Duration::seconds(1)).primary
    );
    assert!(!BlockedWindows::evaluate(ProviderId::Kimi, &usage, at_reset).any());
}

fn code_api_snapshot(pools: serde_json::Value) -> UsageSnapshot {
    let response: KimiCodeApiUsageResponse =
        serde_json::from_value(json!({ "usages": pools })).unwrap();
    code_api::snapshot_from_code_api_response(response).unwrap()
}

#[test]
fn exhausted_code_api_monthly_pool_blocks_code_windows() {
    let usage = code_api_snapshot(json!({
        "limit_5h": { "used_ratio": 0, "reset_time": iso(Duration::hours(1)) },
        "limit_7d": { "used_ratio": 0, "reset_time": iso(Duration::days(4)) },
        "limit_month_total": { "used_ratio": 1.0, "reset_time": iso(Duration::days(30)) }
    }));
    let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());

    assert!(blocked.primary && blocked.secondary);
    assert!(!extra_flag(&blocked, &usage, MONTHLY_WINDOW_ID));
    assert_eq!(blocked.resets_at, Some(now() + Duration::days(30)));
    assert_eq!(usage.primary.used_percent, 0.0);
    assert_eq!(usage.secondary.as_ref().unwrap().used_percent, 0.0);
}

#[test]
fn monthly_only_code_api_response_has_no_code_window_to_block() {
    let usage = code_api_snapshot(json!({
        "limit_month_total": { "used_ratio": 1.05, "reset_time": iso(Duration::days(30)) }
    }));
    let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());

    // The informational weekly placeholder is never blocked, and the pool
    // does not block itself.
    assert!(usage.primary.is_informational);
    assert!(!blocked.any());
}
