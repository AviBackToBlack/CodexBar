//! Longer-window quota that blocks shorter windows (upstream 0.69.0 #4091).
//!
//! When a provider's known longer pool (Kimi's monthly membership) is
//! exhausted, its shorter windows still report fresh capacity from their own
//! counters even though nothing can be used. Surfaces mark those rows as
//! blocked while the raw percentages stay untouched, so notifications, the tray
//! icon, and explicit menu-bar selections keep reading the provider's data.
//! Mirrors upstream `RateWindow.bindingQuotaProjection` plus
//! `MenuCardView.blockingQuotaMetrics`; the blocker's own reset owns the reset
//! text, so no projected reset is computed here.

use chrono::{DateTime, Utc};

use super::{ProviderId, RateWindow, SESSION_WINDOW_MINUTES, UsageSnapshot};

/// Which windows of one snapshot a longer exhausted quota blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockedWindows {
    pub primary: bool,
    pub secondary: bool,
    pub model_specific: bool,
    pub tertiary: bool,
    /// Parallel to [`UsageSnapshot::extra_rate_windows`].
    pub extra: Vec<bool>,
}

impl BlockedWindows {
    /// Evaluate `usage` against the provider's declared blocking quota.
    pub fn evaluate(id: ProviderId, usage: &UsageSnapshot, now: DateTime<Utc>) -> Self {
        let mut blocked = Self {
            extra: vec![false; usage.extra_rate_windows.len()],
            ..Self::default()
        };
        let Some(blocker_id) = id.blocking_quota_window_id() else {
            return blocked;
        };
        let Some(blocker) = usage
            .extra_rate_windows
            .iter()
            .find(|extra| extra.id == blocker_id && extra.usage_known)
            .map(|extra| &extra.window)
        else {
            return blocked;
        };
        let blocks = |window: &RateWindow| is_blocked_by(window, blocker, now);
        blocked.primary = blocks(&usage.primary);
        blocked.secondary = usage.secondary.as_ref().is_some_and(blocks);
        blocked.model_specific = usage.model_specific.as_ref().is_some_and(blocks);
        blocked.tertiary = usage.tertiary.as_ref().is_some_and(blocks);
        for (flag, extra) in blocked.extra.iter_mut().zip(&usage.extra_rate_windows) {
            *flag = extra.usage_known && blocks(&extra.window);
        }
        blocked
    }
}

/// True when `blocker` is a longer, actively exhausted lane than `window`.
/// A window without duration metadata counts as the 5-hour session lane.
fn is_blocked_by(window: &RateWindow, blocker: &RateWindow, now: DateTime<Utc>) -> bool {
    if window.is_informational {
        return false;
    }
    let window_minutes = window.window_minutes.unwrap_or(SESSION_WINDOW_MINUTES);
    blocker
        .window_minutes
        .is_some_and(|minutes| minutes > window_minutes)
        && blocker.remaining_percent() <= 0.0
        && blocker.resets_at.is_none_or(|reset| reset > now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    const MONTH: u32 = 30 * 24 * 60;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_788_000_000, 0).single().unwrap()
    }

    fn window(used: f64, minutes: Option<u32>, reset: Option<DateTime<Utc>>) -> RateWindow {
        RateWindow::with_details(used, minutes, reset, None)
    }

    fn kimi_snapshot(monthly: RateWindow) -> UsageSnapshot {
        let mut usage = UsageSnapshot::new(window(0.0, Some(10_080), None))
            .with_extra_rate_window("kimi-monthly", "Total usage", monthly)
            .with_extra_rate_window(
                "kimi-code-7d",
                "Code 7-day",
                window(25.0, Some(10_080), None),
            );
        usage.secondary = Some(window(0.0, None, None));
        usage
    }

    #[test]
    fn exhausted_monthly_blocks_shorter_windows_but_not_itself() {
        let usage = kimi_snapshot(window(100.0, Some(MONTH), Some(now() + Duration::days(30))));
        let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());
        assert!(blocked.primary && blocked.secondary);
        assert_eq!(blocked.extra, vec![false, true]);
        assert!(!blocked.tertiary && !blocked.model_specific);

        // Tertiary and model-specific lanes follow the same rule as the primary.
        let mut usage = usage;
        usage.tertiary = Some(window(10.0, Some(10_080), None));
        usage.model_specific = Some(window(10.0, Some(60), None));
        let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());
        assert!(blocked.tertiary && blocked.model_specific);
        // Evaluation never rewrites the raw percentages.
        assert_eq!(usage.primary.used_percent, 0.0);
    }

    #[test]
    fn unknown_reset_still_blocks() {
        let usage = kimi_snapshot(window(100.0, Some(MONTH), None));
        assert!(BlockedWindows::evaluate(ProviderId::Kimi, &usage, now()).primary);
    }

    #[test]
    fn available_expired_or_unknown_usage_never_blocks() {
        let unblocked = BlockedWindows {
            extra: vec![false, false],
            ..Default::default()
        };
        for monthly in [
            window(99.9, Some(MONTH), Some(now() + Duration::days(30))),
            window(100.0, Some(MONTH), Some(now())),
            window(100.0, Some(MONTH), Some(now() - Duration::days(1))),
        ] {
            let usage = kimi_snapshot(monthly);
            assert_eq!(
                BlockedWindows::evaluate(ProviderId::Kimi, &usage, now()),
                unblocked
            );
        }

        let mut usage = kimi_snapshot(window(100.0, Some(MONTH), None));
        usage.extra_rate_windows[0].usage_known = false;
        assert_eq!(
            BlockedWindows::evaluate(ProviderId::Kimi, &usage, now()),
            unblocked
        );
    }

    #[test]
    fn missing_minutes_default_to_the_session_lane_and_informational_rows_are_skipped() {
        let mut usage = kimi_snapshot(window(100.0, Some(MONTH), None));
        usage.secondary = Some(RateWindow::informational("No active session"));
        let blocked = BlockedWindows::evaluate(ProviderId::Kimi, &usage, now());
        assert!(blocked.primary && !blocked.secondary);

        // A blocker no longer than the window (or without minutes) cannot block it.
        let usage = kimi_snapshot(window(100.0, Some(10_080), None));
        assert!(!BlockedWindows::evaluate(ProviderId::Kimi, &usage, now()).primary);
        let usage = kimi_snapshot(window(100.0, None, None));
        assert!(!BlockedWindows::evaluate(ProviderId::Kimi, &usage, now()).primary);
    }

    #[test]
    fn providers_without_a_declared_blocker_are_untouched() {
        let usage = kimi_snapshot(window(100.0, Some(MONTH), None));
        let blocked = BlockedWindows::evaluate(ProviderId::Codex, &usage, now());
        assert!(!blocked.primary && blocked.extra.iter().all(|flag| !flag));
    }
}
