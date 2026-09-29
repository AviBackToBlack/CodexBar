//! Nous Portal rows in the OpenCodex ledger (upstream 0.68.0 #4008).
//!
//! The Hermes extractor records `provider: "nous"` with the exact inference
//! model id (`anthropic/claude-sonnet-4.6`), whatever vendor it names. Pricing
//! never falls back to that vendor's rates: only an exact Nous catalog entry
//! or a `nous/<model>` custom override prices a row.

use crate::core::{CostUsagePricing, ModelsDevPricingSnapshot};

use super::{CustomPricing, OpenCodexEntry};

pub(super) const SUBSCRIPTION_ID: &str = "nous";

/// Ledger convention for Nous rows: `inputTokens` excludes cache reads and
/// writes (a row may carry more cache-read than input tokens), so both are
/// added back for helpers that treat input as the inclusive total.
pub(super) fn cost(
    entry: &OpenCodexEntry,
    custom: &CustomPricing,
    pricing_snapshot: &ModelsDevPricingSnapshot,
) -> Option<f64> {
    let input = entry.input_tokens.unwrap_or(0);
    let output = entry.output_tokens.unwrap_or(0);
    let cache_read = entry.cache_read_tokens.unwrap_or(0);
    let cache_write = entry.cache_creation_tokens.unwrap_or(0);
    if let Some(rates) = custom.rates(SUBSCRIPTION_ID, &entry.model) {
        return rates.cost_parts(
            input.saturating_add(cache_read),
            output,
            cache_read,
            cache_write,
        );
    }
    CostUsagePricing::codex_cost_usd_with_cache_write_and_pricing_snapshot(
        &format!("{SUBSCRIPTION_ID}/{}", entry.model.trim()),
        input.saturating_add(cache_read).saturating_add(cache_write),
        cache_read,
        cache_write,
        output,
        Some(pricing_snapshot),
    )
}
