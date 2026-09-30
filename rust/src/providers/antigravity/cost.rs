use crate::core::CostUsagePricing;

/// Antigravity records routing variants of a vendor model that bill at the base model's public
/// price. The alias stays provider-local so shared Claude pricing keeps unknown variants unpriced.
const ROUTING_VARIANT_SUFFIXES: [&str; 3] = ["-tiered", "-low", "-thinking"];

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
    use super::{estimate_cost_usd, pricing_base_model};

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
