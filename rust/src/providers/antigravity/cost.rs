use std::collections::{BTreeMap, HashSet};
use std::future::Future;

use crate::core::CostUsagePricing;
use crate::spend_contract::LocalCostEstimate;

/// Antigravity records routing variants of a vendor model that bill at the base model's public
/// price. The alias stays provider-local so shared Claude pricing keeps unknown variants unpriced.
const ROUTING_VARIANT_SUFFIXES: [&str; 3] = ["-tiered", "-low", "-thinking"];

/// models.dev entries worth refreshing for unpriced history, grouped by
/// models.dev provider: each unpriced model and, for a routing variant, its
/// base model, routed the way a rescan prices them (Gemini models through
/// `google`, Claude models through `anthropic`, GPT models through `openai`).
fn refresh_targets(estimate: &LocalCostEstimate) -> BTreeMap<&'static str, HashSet<String>> {
    let mut targets = BTreeMap::<&'static str, HashSet<String>>::new();
    for model in &estimate.unpriced_models {
        for name in [Some(model.as_str()), pricing_base_model(model)]
            .into_iter()
            .flatten()
        {
            for (provider, model_id) in CostUsagePricing::claude_models_dev_pricing_targets(name) {
                targets.entry(provider).or_default().insert(model_id);
            }
        }
    }
    targets
}

/// One bounded models.dev refresh for the unpriced models of a scan, or None
/// when nothing recorded lacks a price: empty or fully priced history never
/// downloads. The refresh resolves to true when a rescan can now price at
/// least one of those models.
pub(super) fn unpriced_model_pricing_refresh(
    estimate: &LocalCostEstimate,
) -> Option<impl Future<Output = bool> + Send + use<>> {
    let targets = refresh_targets(estimate);
    (!targets.is_empty()).then(|| refresh_pricing_targets(targets))
}

async fn refresh_pricing_targets(targets: BTreeMap<&'static str, HashSet<String>>) -> bool {
    for (provider, model_ids) in &targets {
        if crate::core::refresh_unknown_models_if_needed(provider, model_ids).await {
            return true;
        }
    }
    // An earlier provider group may refresh the shared catalog without
    // resolving its own models while a later group's models became priced.
    let snapshot = crate::core::pricing_snapshot();
    targets.iter().any(|(provider, model_ids)| {
        model_ids
            .iter()
            .any(|model_id| snapshot.lookup(provider, model_id).is_some())
    })
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

/// Matches the routing suffix case-insensitively and keeps the recorded spelling of the base.
fn pricing_base_model(model: &str) -> Option<&str> {
    let lowered = model.to_ascii_lowercase();
    let suffix = ROUTING_VARIANT_SUFFIXES
        .iter()
        .find(|suffix| lowered.ends_with(*suffix))?;
    model
        .get(..model.len() - suffix.len())
        .filter(|base| !base.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        estimate_cost_usd, pricing_base_model, refresh_targets, unpriced_model_pricing_refresh,
    };
    use crate::spend_contract::LocalCostEstimate;

    fn ids(values: &[&str]) -> HashSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn refresh_targets_route_unpriced_models_and_their_routing_base_by_vendor() {
        let mut estimate = LocalCostEstimate::default();
        estimate.record_list_price(Some("claude-future-9-thinking"), None);
        estimate.record_list_price(Some("claude-future-9-thinking"), None);
        estimate.record_list_price(Some("gemini-future-pro-low"), None);
        estimate.record_list_price(Some("gpt-future"), None);
        estimate.record_list_price(None, None);
        let targets = refresh_targets(&estimate);
        assert_eq!(
            targets.keys().copied().collect::<Vec<_>>(),
            ["anthropic", "google", "openai"]
        );
        assert_eq!(
            targets["anthropic"],
            ids(&["claude-future-9-thinking", "claude-future-9"])
        );
        assert_eq!(
            targets["google"],
            ids(&["gemini-future-pro-low", "gemini-future-pro"])
        );
        assert_eq!(targets["openai"], ids(&["gpt-future"]));
        assert!(refresh_targets(&LocalCostEstimate::default()).is_empty());
    }

    #[test]
    fn pricing_refresh_is_offered_only_for_named_unpriced_models() {
        assert!(unpriced_model_pricing_refresh(&LocalCostEstimate::default()).is_none());

        let mut priced = LocalCostEstimate::default();
        priced.record_list_price(Some("claude-sonnet-4-6"), Some(0.25));
        assert!(unpriced_model_pricing_refresh(&priced).is_none());

        let mut unnamed = priced.clone();
        unnamed.record_list_price(None, None);
        assert_eq!(unnamed.coverage.unpriced, 1);
        assert!(unpriced_model_pricing_refresh(&unnamed).is_none());

        let mut unpriced = priced;
        unpriced.record_list_price(Some("gemini-future-pro"), None);
        assert!(unpriced_model_pricing_refresh(&unpriced).is_some());
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
    fn routing_suffixes_match_case_insensitively() {
        let direct = estimate_cost_usd(Some("claude-sonnet-4-6"), 1_000, 200, 100, 500)
            .expect("known public price");
        for model in [
            "claude-sonnet-4-6-Thinking",
            "claude-sonnet-4-6-LOW",
            "claude-sonnet-4-6-Tiered",
        ] {
            assert_eq!(
                estimate_cost_usd(Some(model), 1_000, 200, 100, 500),
                Some(direct),
                "{model}"
            );
        }
        assert_eq!(
            pricing_base_model("Claude-Sonnet-4-6-THINKING"),
            Some("Claude-Sonnet-4-6")
        );
        assert_eq!(pricing_base_model("-thinking"), None);
        assert_eq!(pricing_base_model("claude-sonnet-4-6"), None);
    }

    #[test]
    fn unknown_or_oversized_pricing_inputs_fail_closed() {
        assert_eq!(estimate_cost_usd(Some("unknown"), 1, 2, 3, 4), None);
        assert_eq!(
            estimate_cost_usd(Some("unknown-thinking"), 1, 2, 3, 4),
            None
        );
        assert_eq!(estimate_cost_usd(Some("  "), 1, 2, 3, 4), None);
        assert_eq!(estimate_cost_usd(None, 1, 2, 3, 4), None);
        assert_eq!(
            estimate_cost_usd(Some("claude-sonnet-4-6"), i32::MAX as u64 + 1, 0, 0, 0),
            None
        );
    }
}
