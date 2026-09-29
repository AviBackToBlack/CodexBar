use super::*;
use crate::core::{CodexPriorityOverlay, CodexSourceUsageRow, CodexUsageRecord, CostUsagePricing};
use std::collections::BTreeMap;

type DayModels = HashMap<String, HashMap<String, Vec<i64>>>;

pub(super) fn rebuild_cache_days(cache: &mut CostUsageCache) {
    cache.days.clear();
    let cursor = cache.codex_priority_turns_cursor.as_ref();
    for (path, usage) in &cache.files {
        let overlaid = cursor
            .and_then(|cursor| cursor.overlay_for_file(Path::new(path)))
            .zip(cache.codex_source_rows.get(path))
            .and_then(|(overlay, source)| priority_days(&source.rows, &usage.days, &overlay));
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
    if !rows
        .iter()
        .any(|row| row_pricing_model(row, Some(overlay)).1)
    {
        return None;
    }
    let plain = days_from_codex_source_rows_with_priority(rows, None);
    if day_token_totals(&plain) != day_token_totals(parsed_days) {
        return None;
    }
    Some(days_from_codex_source_rows_with_priority(
        rows,
        Some(overlay),
    ))
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

/// The model a source row prices under, and whether Priority trace evidence
/// changed it.
fn row_pricing_model(
    row: &CodexSourceUsageRow,
    overlay: Option<&CodexPriorityOverlay<'_>>,
) -> (String, bool) {
    let model = match row.pricing.pricing_model.as_deref() {
        Some(model) if !model.is_empty() => {
            if row.pricing.pricing_mode.as_deref() == Some("priority")
                && !model.ends_with("-priority")
            {
                format!("{model}-priority")
            } else {
                model.to_string()
            }
        }
        _ => {
            return (
                CostUsagePricing::CODEX_UNATTRIBUTED_MODEL.to_string(),
                false,
            );
        }
    };
    match overlay.and_then(|overlay| overlay.priority_model(row.turn_id.as_deref(), &model)) {
        Some(priority) => (priority, true),
        None => (model, false),
    }
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
        let (model, _) = row_pricing_model(row, overlay);
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
