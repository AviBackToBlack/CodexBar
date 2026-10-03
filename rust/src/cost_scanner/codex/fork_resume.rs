//! Bounded fork parses continue at their saved cursor.
//!
//! Upstream 0.67.0 resumes a resolved fork at its cached offset while its
//! parent dependency is unchanged (`canResumeCodexForkAccounting`). A fork that
//! restarted at byte zero on every pass would never finish once it is larger
//! than its byte allowance.

use super::*;
use crate::core::CodexForkParseResume;

/// An unfinished fork parse that can continue at its saved cursor.
pub(super) struct CodexForkResume {
    pub(super) parse: CodexForkParseResume,
    pub(super) target_size: i64,
}

impl CodexForkResume {
    /// Resume point for a cached fork, or `None` when the fork must be parsed
    /// from byte zero: no saved parser state, a validated parent that now
    /// supplies a different baseline, a rewritten file, or a cursor that is
    /// not on a line boundary.
    pub(super) fn for_entry(
        path: &Path,
        entry: &CostUsageFileUsage,
        state: &CodexForkAccountingState,
        accounting_mode: &CodexAccountingMode,
        size: i64,
        mtime_ms: i64,
    ) -> Option<Self> {
        let CodexAccountingMode::Baseline {
            baseline,
            paginated_continuation,
            provenance,
            ..
        } = accounting_mode
        else {
            return None;
        };
        let saved = state.resume.as_ref()?;
        // A cached-parent baseline is the saved state itself.
        let same_parent_baseline = match provenance {
            CodexBaselineProvenance::ValidatedParent { .. } => &saved.parse_baseline == baseline,
            CodexBaselineProvenance::CachedValidatedParent => true,
        };
        let start_offset = entry.parsed_bytes.unwrap_or(0);
        let target_size = codex_resumable_scan_target_size(size, entry)?;
        let same_partial = size == entry.size && mtime_ms == entry.mtime_unix_ms;
        let growing = size > entry.size;
        let resumable = same_parent_baseline
            && !state.locally_resolved
            && !entry.codex_unresolved_fork_parent
            && (same_partial || growing)
            && start_offset > 0
            && entry.codex_token_timestamps_monotonic.is_some()
            && JsonlScanner::is_line_boundary_offset(path, start_offset);
        if !resumable {
            return None;
        }
        Some(Self {
            parse: CodexForkParseResume {
                start_offset,
                paginated_continuation: *paginated_continuation,
                inherited_totals: state.inherited_totals.clone()?,
                remaining_inherited_totals: state.remaining_inherited_totals.clone(),
                last_model: entry.last_model.clone(),
                last_totals: entry.last_totals.clone()?,
                last_token_timestamp: entry.codex_last_token_timestamp.clone(),
                first_token_timestamp: state.first_token_timestamp.clone(),
                token_timestamps_monotonic: entry.codex_token_timestamps_monotonic,
                state: saved.clone(),
            },
            target_size,
        })
    }
}
