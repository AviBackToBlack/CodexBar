use super::*;

#[test]
fn invalidated_owner_clears_orphaned_indexing_activity() {
    let mut coordinator = UsageSpendCoordinator::default();
    let owner = coordinator.begin("account:old".to_string());

    assert!(coordinator.clear_if_indexing(&owner));
    assert!(!coordinator.is_current(&owner));
}

#[test]
fn old_owner_cleanup_cannot_clear_a_replacement() {
    let mut coordinator = UsageSpendCoordinator::default();
    let old = coordinator.begin("account:old".to_string());
    let replacement = coordinator.begin("account:new".to_string());

    assert!(!coordinator.clear_if_indexing(&old));
    assert!(coordinator.is_current(&replacement));
}

#[test]
fn settings_replacement_preserves_an_intentional_pause() {
    let mut coordinator = UsageSpendCoordinator::default();
    let old = coordinator.begin("settings:old".to_string());
    let replacement = coordinator.begin("settings:new".to_string());
    let status = codexbar::core::CachedCostReadStatus {
        codex_scan_pause_reason: Some(codexbar::core::CodexScanPauseReason::NoProgress),
        ..Default::default()
    };
    mark_refresh_paused_if_codex_scan_paused(
        &mut coordinator,
        &replacement,
        true,
        status.codex_scan_pause_reason.as_ref(),
    );

    assert!(!coordinator.clear_if_indexing(&old));
    assert_eq!(
        coordinator.current.as_ref().map(|(_, phase)| *phase),
        Some(UsageSpendRefreshPhase::Paused)
    );
}

#[test]
fn privacy_mode_is_part_of_usage_spend_cache_identity() {
    let public = usage_spend_cache_key_with_privacy(&[], "rolling:30", false, false, false);
    let private = usage_spend_cache_key_with_privacy(&[], "rolling:30", false, false, true);
    assert_ne!(public, private);
}

fn identity(period: CostReportingPeriod, now: &str) -> String {
    let now = chrono::DateTime::parse_from_rfc3339(now)
        .expect("valid instant")
        .with_timezone(&Utc);
    period.identity(now, CostTimeZone::UTC)
}

#[test]
fn selected_period_is_part_of_usage_spend_cache_identity() {
    let now = "2026-09-15T12:00:00Z";
    let month = identity(CostReportingPeriod::MonthToDate, now);
    let rolling = identity(CostReportingPeriod::Rolling(30), now);
    let all = identity(CostReportingPeriod::AllAvailable, now);
    assert_ne!(month, rolling);
    assert_ne!(rolling, all);
    assert_ne!(
        usage_spend_cache_key_with_privacy(&[], &month, false, false, false),
        usage_spend_cache_key_with_privacy(&[], &rolling, false, false, false)
    );
}

#[test]
fn month_to_date_identity_changes_across_a_month_boundary() {
    let last_day = identity(CostReportingPeriod::MonthToDate, "2026-09-30T23:00:00Z");
    let first_day = identity(CostReportingPeriod::MonthToDate, "2026-10-01T01:00:00Z");
    assert_ne!(last_day, first_day);
    assert_ne!(
        usage_spend_cache_key_with_privacy(&[], &last_day, false, false, false),
        usage_spend_cache_key_with_privacy(&[], &first_day, false, false, false)
    );
}

#[test]
fn daily_period_cost_follows_the_selected_window() {
    let daily = ["2026-09-01", "2026-09-10", "2026-08-31"]
        .into_iter()
        .map(|day| super::super::bridge::CostDailyPointBridge {
            day: day.to_string(),
            amount: 1.0,
        })
        .collect::<Vec<_>>();
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-15T12:00:00Z")
        .expect("valid instant")
        .with_timezone(&Utc);
    assert_eq!(
        period_cost_from_daily(&daily, CostReportingPeriod::MonthToDate, now),
        Some(2.0)
    );
    assert_eq!(
        period_cost_from_daily(&daily, CostReportingPeriod::AllAvailable, now),
        Some(3.0)
    );
    assert_eq!(
        period_cost_from_daily(&daily, CostReportingPeriod::Rolling(3), now),
        None
    );
}

#[test]
fn long_provider_daily_history_does_not_widen_the_fixed_columns() {
    // Bedrock reports 14 months of daily spend; the 7d / 30d columns keep
    // their windows while the selected period still sees the whole history.
    let today = Utc::now().date_naive();
    let day = |ago: i64| {
        serde_json::json!({
            "day": (today - chrono::Duration::days(ago)).format("%Y-%m-%d").to_string(),
            "amount": 1.0,
        })
    };
    let cost: super::super::bridge::CostSnapshotBridge =
        serde_json::from_value(serde_json::json!({
            "used": 99.0,
            "period": "Monthly",
            "daily": [day(0), day(6), day(7), day(29), day(30), day(200)],
        }))
        .expect("cost fixture");
    let mut snapshot = ProviderUsageSnapshot::from_error(
        codexbar::core::ProviderId::Bedrock,
        codexbar::core::instantiate_provider(codexbar::core::ProviderId::Bedrock).metadata(),
        "unused".to_string(),
        codexbar::core::ProviderStateKind::Unknown,
    );
    snapshot.cost = Some(cost);

    let spend = cached_spend(
        Some(&snapshot),
        CostReportingPeriod::AllAvailable,
        Utc::now(),
    );
    assert_eq!(spend.seven_day, Some(2.0));
    assert_eq!(spend.thirty_day, Some(4.0));
    assert_eq!(spend.period_cost, Some(6.0));
}

#[test]
fn pi_history_is_an_alternate_view_not_a_shared_overview_source() {
    assert!(!include_in_shared_overview("pi", true, true));
    assert!(include_in_shared_overview("codex", true, false));
    assert!(include_in_shared_overview("claude", false, true));
    assert!(!include_in_shared_overview("codex", false, false));
}
