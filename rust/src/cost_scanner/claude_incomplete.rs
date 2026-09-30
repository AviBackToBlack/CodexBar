//! Incomplete Claude request tracking (upstream 0.60.5 #3688).
//!
//! Claude Code proxies can emit a preliminary `message_start` usage row (null
//! stop reason, input tokens, no output, no cache fields). Those rows are
//! excluded from cost and tokens, but the scanner still counts them per day and
//! per model so the UI can say "Incomplete" instead of showing a silent $0.
//! A completed record with the same dedup key supersedes the preliminary row.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, Utc};

use super::claude_usage::ClaudeUsageDedupKey;

/// Reconciled incomplete-request counts for one scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeIncompleteReport {
    pub total: u32,
    /// Local calendar day (`YYYY-MM-DD`) -> count.
    pub by_day: HashMap<String, u32>,
    pub by_model: HashMap<String, u32>,
}

#[derive(Debug)]
struct IncompleteRow {
    key: Option<ClaudeUsageDedupKey>,
    model: String,
    day: Option<String>,
}

/// Collects preliminary rows during a scan and reconciles them against the
/// completed dedup keys once the walk has finished.
#[derive(Debug, Default)]
pub(super) struct ClaudeIncompleteTracker {
    rows: Vec<IncompleteRow>,
    keys: HashSet<ClaudeUsageDedupKey>,
}

impl ClaudeIncompleteTracker {
    pub(super) fn record(
        &mut self,
        key: Option<ClaudeUsageDedupKey>,
        model: &str,
        timestamp: Option<DateTime<Utc>>,
        cutoff: &DateTime<Utc>,
    ) {
        if timestamp.is_some_and(|timestamp| timestamp < *cutoff) {
            return;
        }
        if let Some(key) = &key
            && !self.keys.insert(key.clone())
        {
            return;
        }
        let day = timestamp.map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .date_naive()
                .format("%Y-%m-%d")
                .to_string()
        });
        self.rows.push(IncompleteRow {
            key,
            model: model.to_string(),
            day,
        });
    }

    /// Drop rows whose request also produced a completed record, then count.
    pub(super) fn resolve(
        self,
        completed: &HashSet<ClaudeUsageDedupKey>,
    ) -> ClaudeIncompleteReport {
        let mut report = ClaudeIncompleteReport::default();
        for row in self.rows {
            if row.key.as_ref().is_some_and(|key| completed.contains(key)) {
                continue;
            }
            report.total = report.total.saturating_add(1);
            *report.by_model.entry(row.model).or_insert(0) += 1;
            if let Some(day) = row.day {
                *report.by_day.entry(day).or_insert(0) += 1;
            }
        }
        report
    }
}

impl ClaudeIncompleteReport {
    /// Copy the reconciled totals onto a summary.
    pub(super) fn apply_to(&self, summary: &mut super::CostSummary) {
        summary.incomplete_request_count = self.total;
        summary.incomplete_by_model = self.by_model.clone();
    }

    /// Days with at least one incomplete request, sorted ascending.
    pub(super) fn daily_sorted(&self) -> Vec<(String, u32)> {
        let mut days: Vec<_> = self
            .by_day
            .iter()
            .map(|(day, count)| (day.clone(), *count))
            .collect();
        days.sort_by(|left, right| left.0.cmp(&right.0));
        days
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(request: &str) -> Option<ClaudeUsageDedupKey> {
        Some(ClaudeUsageDedupKey::Request {
            message_id: None,
            request_id: request.to_string(),
        })
    }

    #[test]
    fn completed_key_supersedes_preliminary_row() {
        let cutoff = Utc::now() - chrono::Duration::days(7);
        let mut tracker = ClaudeIncompleteTracker::default();
        tracker.record(key("a"), "m", Some(Utc::now()), &cutoff);
        tracker.record(key("b"), "m", Some(Utc::now()), &cutoff);
        let completed: HashSet<_> = key("a").into_iter().collect();
        let report = tracker.resolve(&completed);
        assert_eq!(report.total, 1);
        assert_eq!(report.by_model.get("m"), Some(&1));
    }

    #[test]
    fn duplicate_preliminary_rows_count_once_and_old_rows_are_skipped() {
        let cutoff = Utc::now() - chrono::Duration::days(1);
        let mut tracker = ClaudeIncompleteTracker::default();
        tracker.record(key("a"), "m", Some(Utc::now()), &cutoff);
        tracker.record(key("a"), "m", Some(Utc::now()), &cutoff);
        tracker.record(
            key("old"),
            "m",
            Some(Utc::now() - chrono::Duration::days(5)),
            &cutoff,
        );
        tracker.record(None, "m", None, &cutoff);
        let report = tracker.resolve(&HashSet::new());
        assert_eq!(report.total, 2);
        assert_eq!(report.by_day.values().sum::<u32>(), 1);
    }
}
