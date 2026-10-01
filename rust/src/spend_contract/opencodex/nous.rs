//! Nous Portal rows in the OpenCodex ledger (upstream 0.68.0 #4008).
//!
//! The Hermes extractor records `provider: "nous"` with the exact inference
//! model id (`anthropic/claude-sonnet-4.6`), whatever vendor it names. Pricing
//! never falls back to that vendor's rates: only a custom pricing override or
//! an exact Nous models.dev entry prices a row.
//!
//! Ledger convention: `inputTokens` excludes cache reads and writes (a row may
//! carry more cache-read than input tokens). A custom override bills input,
//! cache reads and cache writes as separate lanes, as the upstream Nous tests
//! expect; the catalog path adds both back to form the inclusive prompt size
//! that upstream `providerCostUSD` prices.

use crate::core::{CostUsagePricing, ModelsDevPricingSnapshot};

use super::super::CustomRates;
use super::{CustomPricing, OpenCodexEntry};

pub(super) const SUBSCRIPTION_ID: &str = "nous";

pub(super) fn cost(
    entry: &OpenCodexEntry,
    custom: &CustomPricing,
    pricing_snapshot: &ModelsDevPricingSnapshot,
) -> Option<f64> {
    // Upstream `listPriceUSD`: a row without both input and output is unpriced.
    let input = entry.input_tokens?;
    let output = entry.output_tokens?;
    let cache_read = entry.cache_read_tokens.unwrap_or(0);
    let cache_write = entry.cache_creation_tokens.unwrap_or(0);
    if let Some(rates) = custom_rates(entry, custom) {
        return rates.cost_parts(
            input.checked_add(cache_read)?,
            output,
            cache_read,
            cache_write,
        );
    }
    let pricing = pricing_snapshot.lookup_exact(SUBSCRIPTION_ID, catalog_model_id(entry)?)?;
    // A consumed cache lane the catalog does not price keeps the row unknown;
    // it never borrows the input rate.
    if (cache_read > 0 && pricing.cache_read_input_cost_per_token.is_none())
        || (cache_write > 0 && pricing.cache_write_input_cost_per_token.is_none())
    {
        return None;
    }
    let inclusive_input = input.checked_add(cache_read)?.checked_add(cache_write)?;
    Some(CostUsagePricing::models_dev_cost_usd(
        &pricing,
        inclusive_input,
        cache_read,
        cache_write,
        output,
    ))
}

/// The custom override for a Nous row, shared by its cost and the model's
/// custom-pricing flag: the recorded identity (`nous/<model>`, then the bare
/// model key), then the catalog id when the model repeats the `nous/` prefix.
pub(super) fn custom_rates<'a>(
    entry: &OpenCodexEntry,
    custom: &'a CustomPricing,
) -> Option<&'a CustomRates> {
    custom
        .rates(&entry.provider, &entry.model)
        .or_else(|| catalog_model_id(entry).and_then(|model| custom.rates(SUBSCRIPTION_ID, model)))
}

/// models.dev id of a row recorded under `provider: "nous"`: the model without
/// a repeated `nous/` prefix (upstream `ModelsDevPricingTargetResolver`).
/// Legacy OpenAI-transport rows (`provider: "openai"`, model `nous/<id>`) keep
/// upstream's OpenAI pricing route, which has no Nous catalog: only a custom
/// override prices them.
fn catalog_model_id(entry: &OpenCodexEntry) -> Option<&str> {
    if !entry.provider.trim().eq_ignore_ascii_case(SUBSCRIPTION_ID) {
        return None;
    }
    let model = entry.model.trim();
    let model = match model.split_once('/') {
        Some((prefix, rest)) if prefix.trim().eq_ignore_ascii_case(SUBSCRIPTION_ID) => rest,
        _ => model,
    };
    // Every id the resolver rejects before stripping is also rejected here.
    (!model.is_empty() && !model.starts_with('/') && !model.ends_with('/')).then_some(model)
}
