use super::*;
use crate::core::{
    CodexPriorityOverlay, CodexSourceUsageRow, CodexUsageRecord, CostUsagePricing, row_priced_model,
};
use std::collections::BTreeMap;

type DayModels = HashMap<String, HashMap<String, Vec<i64>>>;

pub(super) fn rebuild_cache_days(cache: &mut CostUsageCache) {
    cache.days.clear();
    let files = &cache.files;
    cache.codex_fork_rows.retain(|path, _| {
        files.get(path).is_some_and(|usage| {
            (usage.codex_forked_from_id.is_some() || usage.codex_lineage.uses_parent_baseline())
                && !usage.codex_unresolved_fork_parent
        })
    });
    let cursor = cache.codex_priority_turns_cursor.as_ref();
    for (path, usage) in &cache.files {
        let rows = cache
            .codex_source_rows
            .get(path)
            .map(|source| source.rows.as_slice())
            .or_else(|| cache.codex_fork_rows.get(path).map(Vec::as_slice));
        let overlaid = cursor
            .and_then(|cursor| cursor.overlay_for_file(Path::new(path)))
            .zip(rows)
            .and_then(|(overlay, rows)| priority_days(rows, &usage.days, &overlay));
        let file_days = overlaid.as_ref().unwrap_or(&usage.days);
        for (day, models) in file_days {
            let day_entry = cache.days.entry(day.clone()).or_default();
            for (model, packed) in models {
                let dest = day_entry
                    .entry(model.clone())
                    .or_insert_with(|| vec![0, 0, 0]);
                if dest.len() < 3 {
                    dest.resize(3, 0);
                }

                let had_core_tokens = dest[0] != 0 || dest[1] != 0 || dest[2] != 0;
                let source_input = packed.first().copied().unwrap_or(0);
                let source_cached = packed.get(1).copied().unwrap_or(0);
                let source_output = packed.get(2).copied().unwrap_or(0);
                let source_has_tokens =
                    source_input != 0 || source_cached != 0 || source_output != 0;
                let source_reasoning = packed
                    .get(3)
                    .copied()
                    .map(|reasoning| reasoning.max(0).min(source_output.max(0)));

                dest[0] = dest[0].saturating_add(source_input);
                dest[1] = dest[1].saturating_add(source_cached);
                dest[2] = dest[2].saturating_add(source_output);

                if !source_has_tokens {
                    continue;
                }

                if !had_core_tokens {
                    match source_reasoning {
                        Some(reasoning) => {
                            if dest.len() >= 4 {
                                dest[3] = reasoning.min(dest[2].max(0));
                            } else {
                                dest.push(reasoning.min(dest[2].max(0)));
                            }
                        }
                        None => dest.truncate(3),
                    }
                    continue;
                }

                match (dest.get(3).copied(), source_reasoning) {
                    (Some(previous), Some(reasoning)) => {
                        let merged = previous.saturating_add(reasoning).min(dest[2].max(0));
                        dest[3] = merged;
                    }
                    _ => dest.truncate(3),
                }
            }
        }
    }
}

/// Day totals for one file with Priority trace evidence applied, or `None`
/// when no row is affected or the retained source rows no longer match the
/// file's parsed day totals (a stale row cache must never replace newer data).
fn priority_days(
    rows: &[CodexSourceUsageRow],
    parsed_days: &DayModels,
    overlay: &CodexPriorityOverlay<'_>,
) -> Option<DayModels> {
    let plain = days_from_codex_source_rows_with_priority(rows, None);
    if day_token_totals(&plain) != day_token_totals(parsed_days) {
        return None;
    }
    let overlaid = days_from_codex_source_rows_with_priority(rows, Some(overlay));
    (overlaid != plain).then_some(overlaid)
}

fn day_token_totals(days: &DayModels) -> BTreeMap<&str, (i64, i64, i64)> {
    let mut totals = BTreeMap::new();
    for (day, models) in days {
        let entry: &mut (i64, i64, i64) = totals.entry(day.as_str()).or_default();
        for packed in models.values() {
            entry.0 = entry.0.saturating_add(packed.first().copied().unwrap_or(0));
            entry.1 = entry.1.saturating_add(packed.get(1).copied().unwrap_or(0));
            entry.2 = entry.2.saturating_add(packed.get(2).copied().unwrap_or(0));
        }
    }
    totals
}

pub(super) fn days_from_codex_source_rows(rows: &[CodexSourceUsageRow]) -> DayModels {
    days_from_codex_source_rows_with_priority(rows, None)
}

fn days_from_codex_source_rows_with_priority(
    rows: &[CodexSourceUsageRow],
    overlay: Option<&CodexPriorityOverlay<'_>>,
) -> DayModels {
    let mut days: DayModels = HashMap::new();
    for row in rows {
        let model = row_priced_model(row, overlay)
            .unwrap_or_else(|| CostUsagePricing::CODEX_UNATTRIBUTED_MODEL.to_string());
        let record = CodexUsageRecord {
            day_key: row.day_key.clone(),
            timestamp: row.timestamp,
            model,
            input: row.input,
            cached: row.cached,
            output: row.output,
            reasoning: row.reasoning,
            turn_id: row.turn_id.clone(),
        };
        let packed = days
            .entry(record.day_key.clone())
            .or_default()
            .entry(record.model.clone())
            .or_default();
        JsonlScanner::merge_codex_record_into_packed(packed, &record);
    }
    days
}
