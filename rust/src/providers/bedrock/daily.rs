//! Cost Explorer date ranges and daily Bedrock cost parsing (upstream
//! `BedrockUsageFetcher.dailyRange` / `parseDailyResponse`).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Months, NaiveDate, Utc};
use serde_json::Value;

use super::bedrock_group_amounts;
use crate::core::CostDailyPoint;
use crate::cost_reporting_period::{CostReportingPeriod, CostTimeZone};

/// Cost Explorer `TimePeriod` for daily buckets (upstream `dailyRange`).
///
/// Cost Explorer buckets are UTC, so the month is resolved in UTC. It exposes
/// the current month plus thirteen earlier months, so `since` never reaches
/// further back than that; the exclusive end is tomorrow. An all-available
/// request passes any early `since` and gets the whole exposed range.
pub(super) fn daily_range(since: NaiveDate, now: DateTime<Utc>) -> (String, String) {
    let month_start = utc_month_start(now);
    let earliest = month_start
        .checked_sub_months(Months::new(13))
        .unwrap_or(month_start);
    let tomorrow = now.date_naive() + Duration::days(1);
    (
        since.max(earliest).format("%Y-%m-%d").to_string(),
        tomorrow.format("%Y-%m-%d").to_string(),
    )
}

fn utc_month_start(now: DateTime<Utc>) -> NaiveDate {
    CostReportingPeriod::MonthToDate
        .bounds(now, CostTimeZone::UTC, None)
        .start
}

/// Current-month range: month to date through tomorrow (exclusive).
pub(super) fn current_month_range() -> (String, String) {
    let now = Utc::now();
    daily_range(utc_month_start(now), now)
}

/// Every month Cost Explorer exposes, through tomorrow (exclusive).
pub(super) fn all_available_range() -> (String, String) {
    daily_range(NaiveDate::MIN, Utc::now())
}

/// Daily Bedrock spend from `GetCostAndUsage` DAILY pages.
///
/// Days without positive Bedrock spend are omitted and the same day across
/// pages is summed, as upstream merges its daily reports.
pub(super) fn parse_daily_costs(pages: &[Value]) -> Vec<CostDailyPoint> {
    let mut by_day: BTreeMap<String, f64> = BTreeMap::new();
    for result in pages
        .iter()
        .filter_map(|page| page.get("ResultsByTime").and_then(Value::as_array))
        .flatten()
    {
        let Some(day) = result
            .get("TimePeriod")
            .and_then(|period| period.get("Start"))
            .and_then(Value::as_str)
            .filter(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok())
        else {
            continue;
        };
        let cost: f64 = bedrock_group_amounts(result)
            .filter(|amount| amount.is_finite() && *amount > 0.0)
            .sum();
        if cost > 0.0 {
            *by_day.entry(day.to_string()).or_default() += cost;
        }
    }
    by_day
        .into_iter()
        .map(|(day, amount)| CostDailyPoint { day, amount })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).single().unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn daily_range_month_to_date_starts_at_utc_month_start() {
        let now = utc(2026, 5, 15, 12);
        let (start, end) = daily_range(utc_month_start(now), now);
        assert_eq!(start, "2026-05-01");
        assert_eq!(end, "2026-05-16");
    }

    #[test]
    fn daily_range_month_start_uses_utc_not_local_time() {
        // 23:30 UTC on the last day of April is still April in Cost Explorer.
        let now = Utc
            .with_ymd_and_hms(2026, 4, 30, 23, 30, 0)
            .single()
            .unwrap();
        let (start, end) = daily_range(utc_month_start(now), now);
        assert_eq!(start, "2026-04-01");
        assert_eq!(end, "2026-05-01");
    }

    #[test]
    fn daily_range_all_is_capped_at_current_month_plus_thirteen() {
        let now = utc(2026, 5, 15, 12);
        let (start, end) = daily_range(date(2000, 1, 1), now);
        assert_eq!(start, "2025-04-01");
        assert_eq!(end, "2026-05-16");
        assert_eq!(daily_range(NaiveDate::MIN, now).0, "2025-04-01");
    }

    #[test]
    fn daily_range_keeps_a_recent_since() {
        let now = utc(2026, 5, 15, 12);
        let (start, _) = daily_range(date(2026, 3, 10), now);
        assert_eq!(start, "2026-03-10");
    }

    fn day(start: &str, groups: &[(&str, &str)]) -> Value {
        json!({
            "TimePeriod": { "Start": start, "End": "ignored" },
            "Groups": groups.iter().map(|(service, amount)| json!({
                "Keys": [service],
                "Metrics": { "UnblendedCost": { "Amount": amount, "Unit": "USD" } }
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn daily_costs_keep_only_positive_bedrock_days() {
        let page = json!({ "ResultsByTime": [
            day("2026-03-01", &[("Amazon Bedrock", "7.00"), ("Amazon S3", "99.00")]),
            day("2026-03-02", &[("Amazon Bedrock", "0.00")]),
            day("2026-03-03", &[("Amazon S3", "5.00")]),
            day("2026-03-04", &[("Claude Sonnet (Amazon Bedrock Edition)", "1.25"), ("Amazon Bedrock", "0.75")]),
        ]});
        assert_eq!(
            parse_daily_costs(&[page]),
            vec![
                CostDailyPoint {
                    day: "2026-03-01".into(),
                    amount: 7.0
                },
                CostDailyPoint {
                    day: "2026-03-04".into(),
                    amount: 2.0
                },
            ]
        );
    }

    #[test]
    fn daily_costs_merge_the_same_day_across_pages_and_skip_bad_rows() {
        let first = json!({ "ResultsByTime": [day("2026-03-01", &[("Amazon Bedrock", "3.00")])]});
        let second = json!({ "ResultsByTime": [
            day("2026-03-01", &[("Amazon Bedrock", "4.00")]),
            day("not-a-date", &[("Amazon Bedrock", "9.00")]),
            day("2026-03-02", &[("Amazon Bedrock", "NaN")]),
            day("2026-03-05", &[("Amazon Bedrock", "-2.00")]),
        ]});
        let points = parse_daily_costs(&[first, second, json!({})]);
        assert_eq!(
            points,
            vec![CostDailyPoint {
                day: "2026-03-01".into(),
                amount: 7.0
            }]
        );
    }
}
