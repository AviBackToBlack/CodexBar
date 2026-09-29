use super::{
    CostFetchFailure, ProviderLocalUsageSummary, cost_fetch_failure_allows_early_retry,
    local_usage_summary_from_cost_summary, localized_estimate_note, muse_local_usage_summary,
    token_cost_cache_is_fresh,
};
use crate::commands::is_provider_cache_fresh;
use codexbar::cost_reporting_period::CostReportingPeriod;
use codexbar::cost_scanner::CostSummary;
use codexbar::providers::muse::local_usage::{DailyUsage, Report};
use codexbar::settings::Language;
use codexbar::spend_contract::LocalHistoryCoverage;
use std::time::{Duration, Instant};

#[test]
fn token_cost_age_does_not_use_provider_quota_age() {
    let now = Instant::now();
    let token_loaded = now - Duration::from_secs(31);
    let provider_updated = now;
    assert!(!token_cost_cache_is_fresh(
        Some(token_loaded),
        now,
        Duration::from_secs(30)
    ));
    assert!(is_provider_cache_fresh(
        Some(provider_updated),
        Duration::from_secs(30)
    ));
}

#[test]
fn fast_cost_failures_allow_the_next_pass_to_retry() {
    assert!(cost_fetch_failure_allows_early_retry(
        CostFetchFailure::Failed
    ));
    assert!(!cost_fetch_failure_allows_early_retry(
        CostFetchFailure::TimedOut
    ));
}

#[test]
fn local_usage_summary_serializes_token_cost_timestamp() {
    let summary = ProviderLocalUsageSummary {
        today_cost: Some(1.0),
        thirty_day_cost: Some(2.0),
        thirty_day_tokens: Some(300),
        period_cost: Some(3.0),
        period_tokens: Some(400),
        reporting_period: "month-to-date".to_string(),
        latest_tokens: Some(40),
        top_model: Some("gpt-5".to_string()),
        estimate_note: "estimated".to_string(),
        token_cost_updated_at_ms: 1234,
    };

    let json = serde_json::to_value(summary).expect("serialize summary");
    assert_eq!(
        json.get("tokenCostUpdatedAtMs").and_then(|v| v.as_i64()),
        Some(1234)
    );
}

#[test]
fn muse_local_usage_summary_exposes_complete_tokens_without_cost() {
    let report = Report {
        daily: vec![DailyUsage {
            day: "2026-09-20".to_string(),
            input_tokens: 10,
            output_tokens: 2,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 12,
            request_count: 1,
            models: vec![("muse-spark-1.3".to_string(), 12)],
        }],
        total_tokens: Some(12),
        today_tokens: Some(12),
        session_count: 1,
        top_model: Some("muse-spark-1.3".to_string()),
        coverage: LocalHistoryCoverage::Complete,
    };
    let period = CostReportingPeriod::Rolling(30);
    let summary = muse_local_usage_summary(
        &report,
        &report,
        period,
        codexbar::settings::Language::default(),
    )
    .expect("complete history is visible");
    assert_eq!(summary.period_tokens, Some(12));
    assert_eq!(summary.reporting_period, "rolling:30");
    assert_eq!(summary.today_cost, None);
    assert_eq!(summary.thirty_day_cost, None);
    assert_eq!(summary.thirty_day_tokens, Some(12));
    assert_eq!(summary.latest_tokens, Some(12));
    assert_eq!(summary.top_model.as_deref(), Some("muse-spark-1.3"));

    let partial = Report {
        coverage: LocalHistoryCoverage::Partial,
        ..report
    };
    assert!(
        muse_local_usage_summary(
            &partial,
            &partial,
            period,
            codexbar::settings::Language::default()
        )
        .is_none()
    );
}

#[test]
fn local_usage_keeps_thirty_day_fixed_while_period_follows_selection() {
    let thirty = CostSummary {
        total_cost_usd: 30.0,
        input_tokens: 3_000,
        output_tokens: 0,
        ..CostSummary::default()
    };
    let month = CostSummary {
        total_cost_usd: 5.0,
        input_tokens: 400,
        output_tokens: 100,
        ..CostSummary::default()
    };
    let summary = local_usage_summary_from_cost_summary(
        "claude",
        &thirty,
        CostReportingPeriod::MonthToDate,
        Some(&month),
    )
    .expect("usage is visible");
    assert_eq!(summary.thirty_day_cost, Some(30.0));
    assert_eq!(summary.thirty_day_tokens, Some(3_000));
    assert_eq!(summary.period_cost, Some(5.0));
    assert_eq!(summary.period_tokens, Some(500));
    assert_eq!(summary.reporting_period, "month-to-date");
}

#[test]
fn japanese_estimate_note_is_localized() {
    assert_eq!(
        localized_estimate_note("codex", Language::Japanese),
        "ローカルログから推定したもので、請求書と異なる場合があります"
    );
    assert_eq!(
        localized_estimate_note("claude", Language::Japanese),
        "ClaudeのローカルログからAPIレートで推定したもので、トークン総数が請求書と異なる場合があります"
    );
}

#[test]
fn english_estimate_note_is_localized() {
    assert_eq!(
        localized_estimate_note("codex", Language::English),
        "Estimated from local logs; may differ from your bill"
    );
    assert_eq!(
        localized_estimate_note("claude", Language::English),
        "Estimated from local Claude logs at API rates; token totals may differ from your bill"
    );
}
