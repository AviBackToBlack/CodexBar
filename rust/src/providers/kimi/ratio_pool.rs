//! Kimi Code API ratio-pool reconciliation (upstream 0.63.0 `fd2414d`,
//! #3755).
//!
//! Mixed legacy responses can carry zero ratio placeholders next to populated
//! counters for the same quota. A zero ratio stays authoritative unless all
//! upstream conditions hold: no monthly ratio pool, reliable legacy weekly
//! counters, a count window of the same duration with nonzero reliable use,
//! and a count reset within two seconds of the ratio reset.

use chrono::TimeDelta;

use super::{
    KimiCodeApiUsageResponse, KimiRatioPool, KimiUsageDetail, RateWindow, format_usage_amount,
};

/// Upstream observed the legacy and ratio reset clocks about 1.45 s apart.
const MATCHING_RESET_TOLERANCE_SECS: i64 = 2;

/// Integer usage counters (upstream `KimiUsageSnapshot.usageCounts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UsageCounts {
    used: i64,
    limit: i64,
    reliable: bool,
}

/// `used` is authoritative and may exceed the limit during overage;
/// `remaining` only counts when it describes a valid balance. A valid limit
/// without usable counters is kept but marked unreliable.
fn usage_counts(detail: &KimiUsageDetail) -> Option<UsageCounts> {
    let limit = integer_counter(detail.limit.as_ref()).filter(|limit| *limit > 0)?;
    if let Some(used) = integer_counter(detail.used.as_ref()).filter(|used| *used >= 0) {
        return Some(UsageCounts {
            used,
            limit,
            reliable: true,
        });
    }
    if let Some(remaining) = integer_counter(detail.remaining.as_ref())
        .filter(|remaining| (0..=limit).contains(remaining))
    {
        return Some(UsageCounts {
            used: limit - remaining,
            limit,
            reliable: true,
        });
    }
    Some(UsageCounts {
        used: 0,
        limit,
        reliable: false,
    })
}

/// Upstream keeps counters as strings and reads them with `Int(_)`: integer
/// strings and integral JSON numbers count; fractions, padded or formatted
/// strings do not.
fn integer_counter(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(number) => number.as_i64().or_else(|| {
            let value = number.as_f64()?;
            let integral = value.is_finite()
                && value.fract() == 0.0
                && value >= i64::MIN as f64
                && value < i64::MAX as f64;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "finite integral value is bounded to the i64 range above"
            )]
            integral.then_some(value as i64)
        }),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

/// Resolve a ratio pool, replacing a zero placeholder with the matching
/// legacy count window when the response proves the counters are the same
/// quota. `count_window_minutes` is the duration the legacy counters report.
pub(super) fn resolved_ratio_window(
    response: &KimiCodeApiUsageResponse,
    pool: &KimiRatioPool,
    detail: Option<&KimiUsageDetail>,
    window_minutes: u32,
    count_window_minutes: Option<u32>,
) -> Option<RateWindow> {
    let ratio_window = pool.rate_window(window_minutes)?;
    let count_window = matching_count_window(
        response,
        &ratio_window,
        detail,
        window_minutes,
        count_window_minutes,
    );
    Some(count_window.unwrap_or(ratio_window))
}

fn matching_count_window(
    response: &KimiCodeApiUsageResponse,
    ratio_window: &RateWindow,
    detail: Option<&KimiUsageDetail>,
    window_minutes: u32,
    count_window_minutes: Option<u32>,
) -> Option<RateWindow> {
    let has_monthly_pool = response
        .usages
        .as_ref()
        .is_some_and(|pools| pools.monthly.is_some());
    let weekly_counts_reliable = response
        .usage
        .as_ref()
        .and_then(usage_counts)
        .is_some_and(|counts| counts.reliable);
    if ratio_window.used_percent != 0.0
        || has_monthly_pool
        || !weekly_counts_reliable
        || count_window_minutes != Some(window_minutes)
    {
        return None;
    }

    let detail = detail?;
    let counts = usage_counts(detail).filter(|counts| counts.reliable && counts.used > 0)?;
    let count_reset = detail
        .reset_time
        .as_ref()
        .and_then(super::parse_kimi_timestamp)?;
    let ratio_reset = ratio_window.resets_at?;
    if (count_reset - ratio_reset).abs() > TimeDelta::seconds(MATCHING_RESET_TOLERANCE_SECS) {
        return None;
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "quota counters are far below 2^52; the percent is display-only"
    )]
    let (used, limit) = (counts.used as f64, counts.limit as f64);
    Some(RateWindow::with_details(
        used / limit * 100.0,
        Some(window_minutes),
        Some(count_reset),
        Some(format!(
            "{}/{} credits",
            format_usage_amount(used),
            format_usage_amount(limit)
        )),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::code_api::snapshot_from_code_api_response;
    use super::super::{KimiCodeApiUsageResponse, UsageSnapshot};
    use chrono::{DateTime, Utc};
    use serde_json::{Value, json};

    fn parse(value: Value) -> UsageSnapshot {
        let response: KimiCodeApiUsageResponse =
            serde_json::from_value(value).expect("fixture parses");
        snapshot_from_code_api_response(response).expect("fixture has usable quota")
    }

    fn at(text: &str) -> Option<DateTime<Utc>> {
        Some(
            DateTime::parse_from_rfc3339(text)
                .expect("valid fixture timestamp")
                .with_timezone(&Utc),
        )
    }

    /// Win-CodexBar keeps the session pool primary, so single-lane upstream
    /// fixtures add a nonzero session pool that is never reconciled.
    fn weekly_fixture(usage: Value, weekly_pool: Value) -> Value {
        json!({
            "usage": usage,
            "usages": {
                "limit_5h": { "used_ratio": 0.5 },
                "limit_7d": weekly_pool
            }
        })
    }

    fn weekly_percent(snapshot: &UsageSnapshot) -> f64 {
        snapshot
            .secondary
            .as_ref()
            .expect("weekly lane is present")
            .used_percent
    }

    fn mixed_international_response() -> Value {
        json!({
            "usage": {
                "limit": "100",
                "used": "19",
                "remaining": "81",
                "resetTime": "2026-09-19T16:45:59.449979Z"
            },
            "limits": [{
                "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                "detail": {
                    "limit": "100",
                    "used": "1",
                    "remaining": "99",
                    "resetTime": "2026-09-19T14:45:59.449979Z"
                }
            }],
            "usages": {
                "limit_5h": { "used_ratio": 0, "reset_time": "2026-09-19T14:45:58Z" },
                "limit_7d": { "used_ratio": 0, "reset_time": "2026-09-19T16:45:58Z" }
            }
        })
    }

    // Upstream: `zero ratio placeholder does not hide matching nonzero counts`.
    #[test]
    fn zero_ratio_placeholder_does_not_hide_matching_nonzero_counts() {
        for reset in ["2026-09-19T16:45:58Z", "2026-09-19T16:45:59Z"] {
            let snapshot = parse(weekly_fixture(
                json!({
                    "limit": "100",
                    "used": "19",
                    "remaining": "81",
                    "resetTime": "2026-09-19T16:45:59.449979Z"
                }),
                json!({ "used_ratio": 0, "reset_time": reset }),
            ));
            assert_eq!(weekly_percent(&snapshot), 19.0, "ratio reset {reset}");
            assert_eq!(snapshot.primary.used_percent, 50.0);
        }
    }

    // Upstream: `zero ratio after a different reset stays authoritative`.
    #[test]
    fn zero_ratio_after_a_different_reset_stays_authoritative() {
        let snapshot = parse(weekly_fixture(
            json!({
                "limit": "100",
                "used": "19",
                "remaining": "81",
                "resetTime": "2026-09-19T16:45:59Z"
            }),
            json!({ "used_ratio": 0, "reset_time": "2026-09-26T16:45:59Z" }),
        ));
        assert_eq!(weekly_percent(&snapshot), 0.0);
    }

    // Upstream: `mixed international response retains the used weekly and
    // session quotas` (lanes in Win-CodexBar order).
    #[test]
    fn mixed_international_response_retains_the_used_weekly_and_session_quotas() {
        let snapshot = parse(mixed_international_response());
        let weekly = snapshot.secondary.as_ref().expect("weekly lane");
        assert_eq!(weekly.used_percent, 19.0);
        assert_eq!(weekly.window_minutes, Some(10_080));
        assert_eq!(weekly.resets_at, at("2026-09-19T16:45:59.449979Z"));
        assert_eq!(snapshot.primary.used_percent, 1.0);
        assert_eq!(snapshot.primary.window_minutes, Some(300));
        assert_eq!(
            snapshot.primary.resets_at,
            at("2026-09-19T14:45:59.449979Z")
        );
        assert!(snapshot.tertiary.is_none());
        assert!(snapshot.extra_rate_windows.is_empty());
    }

    // Upstream: `nonzero ratios remain authoritative over legacy counts`.
    #[test]
    fn nonzero_ratios_remain_authoritative_over_legacy_counts() {
        for ratio in [0.1869, 0.5] {
            let snapshot = parse(weekly_fixture(
                json!({ "limit": "100", "used": "19", "resetTime": "2026-09-19T16:45:59Z" }),
                json!({ "used_ratio": ratio, "reset_time": "2026-09-19T16:45:59Z" }),
            ));
            assert!((weekly_percent(&snapshot) - ratio * 100.0).abs() < 0.000_01);
        }
    }

    // Upstream: `monthly ratio accounts retain zero ratios even with matching
    // legacy counts`.
    #[test]
    fn monthly_ratio_accounts_retain_zero_ratios_even_with_matching_legacy_counts() {
        let snapshot = parse(json!({
            "usage": { "limit": "100", "used": "19", "resetTime": "2026-09-19T16:45:59Z" },
            "usages": {
                "limit_5h": { "used_ratio": 0.5 },
                "limit_7d": { "used_ratio": 0, "reset_time": "2026-09-19T16:45:59Z" },
                "limit_month_total": { "used_ratio": 0.0313 }
            }
        }));
        assert_eq!(weekly_percent(&snapshot), 0.0);
        let monthly = snapshot.tertiary.expect("monthly pool");
        assert!((monthly.used_percent - 3.13).abs() < 0.000_01);
    }

    // Upstream: `unmatched count resets cannot override a zero ratio`.
    #[test]
    fn unmatched_count_resets_cannot_override_a_zero_ratio() {
        for reset in [json!(null), json!("invalid"), json!("2026-09-19T16:46:02Z")] {
            let snapshot = parse(weekly_fixture(
                json!({ "limit": "100", "used": "19", "resetTime": reset }),
                json!({ "used_ratio": 0, "reset_time": "2026-09-19T16:45:59Z" }),
            ));
            assert_eq!(weekly_percent(&snapshot), 0.0, "count reset {reset}");
        }
    }

    // Upstream: `invalid or empty counts cannot override a zero ratio`.
    #[test]
    fn invalid_or_empty_counts_cannot_override_a_zero_ratio() {
        for used in ["0", "-1", "invalid"] {
            let snapshot = parse(weekly_fixture(
                json!({ "limit": "100", "used": used, "resetTime": "2026-09-19T16:45:59Z" }),
                json!({ "used_ratio": 0, "reset_time": "2026-09-19T16:45:59Z" }),
            ));
            assert_eq!(weekly_percent(&snapshot), 0.0, "used {used}");
        }
    }

    // Upstream: `different count window duration cannot override the session
    // ratio`.
    #[test]
    fn different_count_window_duration_cannot_override_the_session_ratio() {
        let mut response = mixed_international_response();
        response["limits"][0]["window"]["duration"] = json!(120);
        let snapshot = parse(response);
        assert_eq!(weekly_percent(&snapshot), 19.0);
        assert_eq!(snapshot.primary.used_percent, 0.0);
        assert_eq!(snapshot.primary.window_minutes, Some(300));
    }

    // Upstream `usageCounts`: an invalid `used` falls back to a valid
    // `remaining` balance, which is reliable evidence for both lanes.
    #[test]
    fn remaining_balance_recovers_invalid_used_counters() {
        let mut response = mixed_international_response();
        response["usage"]["used"] = json!("invalid");
        response["limits"][0]["detail"]["used"] = json!("-1");
        let snapshot = parse(response);
        assert_eq!(weekly_percent(&snapshot), 19.0);
        assert_eq!(snapshot.primary.used_percent, 1.0);
        assert_eq!(
            snapshot.primary.reset_description.as_deref(),
            Some("1/100 credits")
        );
    }

    // Upstream gates every override on reliable legacy weekly counters.
    #[test]
    fn session_override_requires_reliable_weekly_counters() {
        for weekly_usage in [
            None,
            Some(json!({ "limit": "100", "used": "invalid", "remaining": "101" })),
            Some(json!({ "limit": "0", "used": "19" })),
        ] {
            let mut response = mixed_international_response();
            match weekly_usage {
                Some(usage) => response["usage"] = usage,
                None => {
                    response
                        .as_object_mut()
                        .expect("fixture object")
                        .remove("usage");
                }
            }
            let snapshot = parse(response);
            assert_eq!(snapshot.primary.used_percent, 0.0);
            assert_eq!(snapshot.primary.window_minutes, Some(300));
        }
    }

    // Upstream reads counters with `Int(_)`: fractional or padded values are
    // not reliable evidence, while integral JSON numbers are.
    #[test]
    fn counters_must_be_integers() {
        for used in [json!("19.5"), json!(" 19"), json!(19.5)] {
            let snapshot = parse(weekly_fixture(
                json!({ "limit": "100", "used": used, "resetTime": "2026-09-19T16:45:59Z" }),
                json!({ "used_ratio": 0, "reset_time": "2026-09-19T16:45:59Z" }),
            ));
            assert_eq!(weekly_percent(&snapshot), 0.0, "used {used}");
        }
        let snapshot = parse(weekly_fixture(
            json!({ "limit": 100.0, "used": 19, "resetTime": "2026-09-19T16:45:59Z" }),
            json!({ "used_ratio": 0, "reset_time": "2026-09-19T16:45:59Z" }),
        ));
        assert_eq!(weekly_percent(&snapshot), 19.0);
    }

    fn zero_session_ratio_with_legacy_window(window: Option<Value>) -> UsageSnapshot {
        let mut response = mixed_international_response();
        match window {
            Some(window) => response["limits"][0]["window"] = window,
            None => {
                response["limits"][0]
                    .as_object_mut()
                    .expect("limit object")
                    .remove("window");
            }
        }
        parse(response)
    }

    // Win-CodexBar tolerates a legacy limit without a window (upstream fails
    // to decode it); without a duration the counters cannot claim the lane.
    #[test]
    fn missing_legacy_window_does_not_override_zero_session_ratio() {
        let snapshot = zero_session_ratio_with_legacy_window(None);
        assert_eq!(snapshot.primary.window_minutes, Some(300));
        assert_eq!(snapshot.primary.used_percent, 0.0);
    }

    #[test]
    fn unrecognized_legacy_window_does_not_override_zero_session_ratio() {
        let snapshot = zero_session_ratio_with_legacy_window(Some(json!({
            "duration": 300,
            "timeUnit": "TIME_UNIT_FORTNIGHT"
        })));
        assert_eq!(snapshot.primary.window_minutes, Some(300));
        assert_eq!(snapshot.primary.used_percent, 0.0);
    }
}
