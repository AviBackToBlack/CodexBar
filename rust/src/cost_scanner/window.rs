//! Resolves a scanner's [`CostReportingPeriod`] into concrete scan windows.

use chrono::{DateTime, Duration, NaiveDate, Utc};

use super::CostScanner;
use crate::cost_reporting_period::{CostReportingPeriod, clamp_window_days, cost_bucket_zone};

/// Inclusive local-day window plus the instant transcript scanners cut off at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScanWindow {
    /// First day of the window (also reported as `period_start`).
    pub start: NaiveDate,
    /// Last day of the window (today).
    pub end: NaiveDate,
    /// Records before this instant are outside the window.
    pub cutoff: DateTime<Utc>,
    /// Resolved day count for scanners that take a day count.
    pub days: u32,
}

impl CostScanner {
    /// The window in the pinned bucket zone, resolved from the calendar.
    ///
    /// `earliest` is the first day with source data; it only matters for
    /// [`CostReportingPeriod::AllAvailable`]. Rolling(N) covers N days
    /// including today, and the cutoff is the start of the first day.
    pub(super) fn calendar_window(
        &self,
        now: DateTime<Utc>,
        earliest: Option<NaiveDate>,
    ) -> ScanWindow {
        let tz = cost_bucket_zone();
        let bounds = self.period.bounds(now, tz, earliest);
        ScanWindow {
            start: bounds.start,
            end: bounds.end,
            cutoff: tz.start_of_day_utc(bounds.start),
            days: clamp_window_days(bounds.days()),
        }
    }

    /// The window used by the Claude and Pi transcript scanners.
    ///
    /// A rolling window keeps its historical shape (a `now - N * 24h` cutoff
    /// and `today - N` as the reported start, with `today` supplied by the
    /// caller); month to date and all available history use exact calendar
    /// midnights so no record from the previous month leaks in.
    pub(super) fn transcript_window(&self, now: DateTime<Utc>, today: NaiveDate) -> ScanWindow {
        match self.period {
            CostReportingPeriod::Rolling(days) => ScanWindow {
                start: today - Duration::days(i64::from(days)),
                end: today,
                cutoff: now - Duration::days(i64::from(days)),
                days,
            },
            CostReportingPeriod::MonthToDate | CostReportingPeriod::AllAvailable => {
                self.calendar_window(now, None)
            }
        }
    }
}
