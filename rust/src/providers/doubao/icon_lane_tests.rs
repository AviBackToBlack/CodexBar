//! Agent Plan tray-icon lane hints (upstream 0.66.0 `iconWindowResolver`).

use super::*;

fn snapshot_for(levels: &[(&str, f64)]) -> UsageSnapshot {
    coding_plan_snapshot(CodingPlanResult {
        status: None,
        update_timestamp: None,
        quota_usage: levels
            .iter()
            .map(|(level, percent)| CodingPlanQuota {
                level: (*level).to_string(),
                percent: *percent,
                reset_timestamp: Some(1_783_040_400.0),
            })
            .collect(),
    })
}

fn extra<'a>(snapshot: &'a UsageSnapshot, id: &str) -> &'a NamedRateWindow {
    snapshot
        .extra_rate_windows
        .iter()
        .find(|extra| extra.id == id)
        .unwrap_or_else(|| panic!("missing extra window {id}"))
}

#[test]
fn agent_only_snapshot_has_no_invented_coding_lanes() {
    let snapshot = snapshot_for(&[
        ("agent_5h", 0.0),
        ("agent_weekly", 31.0),
        ("agent_monthly", 72.0),
    ]);

    assert!(snapshot.primary.is_informational);
    assert_eq!(snapshot.primary.window_minutes, Some(300));
    assert!(snapshot.secondary.is_none());
    assert!(snapshot.tertiary.is_none());
    assert_eq!(
        snapshot
            .extra_rate_windows
            .iter()
            .map(|extra| extra.id.as_str())
            .collect::<Vec<_>>(),
        [
            "doubao-agent-session",
            "doubao-agent-weekly",
            "doubao-agent-monthly"
        ]
    );
    let session = extra(&snapshot, "doubao-agent-session");
    assert_eq!(session.icon_fallback, Some(IconLane::Primary));
    assert_eq!(session.window.window_minutes, Some(300));
    assert!(session.window.resets_at.is_some());
    let weekly = extra(&snapshot, "doubao-agent-weekly");
    assert_eq!(weekly.icon_fallback, Some(IconLane::Secondary));
    assert_eq!(weekly.window.window_minutes, Some(10_080));
    assert_eq!(weekly.window.used_percent, 31.0);
    assert_eq!(extra(&snapshot, "doubao-agent-monthly").icon_fallback, None);
}

#[test]
fn coding_lanes_stay_authoritative_beside_agent_lanes() {
    let snapshot = snapshot_for(&[
        ("session", 10.0),
        ("weekly", 20.0),
        ("agent_5h", 0.0),
        ("agent_weekly", 31.0),
    ]);

    assert!(!snapshot.primary.is_informational);
    assert_eq!(snapshot.primary.used_percent, 10.0);
    assert_eq!(snapshot.secondary.as_ref().unwrap().used_percent, 20.0);
    // The hints stay on the agent lanes; the shell decides when they apply.
    assert_eq!(
        extra(&snapshot, "doubao-agent-session").icon_fallback,
        Some(IconLane::Primary)
    );
}

#[test]
fn team_and_monthly_buckets_never_carry_an_icon_hint() {
    let snapshot = snapshot_for(&[
        ("agent_team_5h", 25.0),
        ("agent_team_weekly", 40.0),
        ("coding_team_5h", 15.0),
        ("agent_monthly", 60.0),
    ]);

    assert!(snapshot.primary.is_informational);
    assert!(!snapshot.extra_rate_windows.is_empty());
    assert!(
        snapshot
            .extra_rate_windows
            .iter()
            .all(|extra| extra.icon_fallback.is_none())
    );
}
