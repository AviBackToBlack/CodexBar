use std::collections::HashSet;

use crate::core::CostUsagePricing;
use crate::spend_contract::LocalCostEstimate;

/// Antigravity records routing variants of a vendor model that bill at the base
/// model's public price. The alias stays provider-local so shared Claude pricing
/// keeps reporting unknown Claude variants as unpriced.
fn pricing_base_model(model: &str) -> Option<&str> {
    let lowered = model.to_ascii_lowercase();
    ["-tiered", "-low", "-thinking"]
        .iter()
        .find(|suffix| lowered.ends_with(**suffix))
        .map(|suffix| &model[..model.len() - suffix.len()])
        .filter(|base| !base.is_empty())
}

/// Models worth fetching for an explicit pricing refresh: each unpriced model
/// and, for a routing variant, its base model.
fn refresh_model_ids(estimate: &LocalCostEstimate) -> HashSet<String> {
    estimate
        .unpriced_models
        .iter()
        .flat_map(|model| {
            [Some(model.as_str()), pricing_base_model(model)]
                .into_iter()
                .flatten()
                .map(str::to_string)
        })
        .collect()
}

/// Run one bounded models.dev refresh for unpriced models. Returns true when a
/// rescan may now price them. Empty or fully priced history never downloads.
pub(super) async fn refresh_unpriced_model_pricing(estimate: &LocalCostEstimate) -> bool {
    let model_ids = refresh_model_ids(estimate);
    !model_ids.is_empty()
        && crate::core::refresh_unknown_models_if_needed("anthropic", &model_ids).await
}

pub(super) fn estimate_cost_usd(
    model: Option<&str>,
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
) -> Option<f64> {
    let model = model.map(str::trim).filter(|value| !value.is_empty())?;
    let input = i32::try_from(input).ok()?;
    let cache_read = i32::try_from(cache_read).ok()?;
    let cache_write = i32::try_from(cache_write).ok()?;
    let output = i32::try_from(output).ok()?;
    let resolve = |candidate: &str| {
        CostUsagePricing::claude_cost_usd(candidate, input, cache_read, cache_write, output)
            .filter(|cost| cost.is_finite() && *cost >= 0.0)
    };
    resolve(model).or_else(|| pricing_base_model(model).and_then(resolve))
}

#[cfg(test)]
mod tests {
    use super::{estimate_cost_usd, refresh_model_ids};
    use crate::spend_contract::LocalCostEstimate;

    #[test]
    fn refresh_targets_unpriced_models_and_their_routing_base() {
        let mut estimate = LocalCostEstimate::default();
        estimate.record_list_price(Some("claude-future-9-thinking"), None);
        estimate.record_list_price(Some("claude-future-9-thinking"), None);
        estimate.record_list_price(None, None);
        let ids = refresh_model_ids(&estimate);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("claude-future-9-thinking"));
        assert!(ids.contains("claude-future-9"));
        assert!(refresh_model_ids(&LocalCostEstimate::default()).is_empty());
    }

    #[test]
    fn prices_known_models_and_provider_local_routing_variants() {
        let direct = estimate_cost_usd(Some("claude-sonnet-4-6"), 1_000, 200, 100, 500)
            .expect("known public price");
        let routed = estimate_cost_usd(Some("claude-sonnet-4-6-thinking"), 1_000, 200, 100, 500)
            .expect("routing suffix uses the base public price");
        assert!(direct > 0.0);
        assert_eq!(direct, routed);
    }

    #[test]
    fn unknown_or_oversized_pricing_inputs_fail_closed() {
        assert_eq!(estimate_cost_usd(Some("unknown"), 1, 2, 3, 4), None);
        assert_eq!(
            estimate_cost_usd(Some("claude-sonnet-4-6"), i32::MAX as u64 + 1, 0, 0, 0),
            None
        );
    }
}
