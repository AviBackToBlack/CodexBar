//! Mistral billing price lookup (upstream 0.70.0 #4076).
//!
//! The billing price table lists one metric several times: once per API zone
//! and service tier, and again under other event types. A per-second audio
//! price or a priority-tier price is far higher than the standard token price,
//! so a usage row only takes the price with the same event type, billing
//! metric, billing group, API zone and service tier.
//!
//! Legacy tables can omit both the API zone and the service tier. Only that
//! explicitly unqualified row is a fallback, for usage rows with the same event
//! type, metric and group. A zone or tier is never guessed.

use std::collections::HashMap;

use serde::Deserialize;

use super::UsageEntry;

#[derive(Debug, Deserialize)]
pub(super) struct MistralPrice {
    #[serde(rename = "event_type")]
    event_type: Option<String>,
    #[serde(rename = "billing_metric")]
    billing_metric: Option<String>,
    #[serde(rename = "billing_group")]
    billing_group: Option<String>,
    #[serde(rename = "api_zone")]
    api_zone: Option<String>,
    #[serde(rename = "service_tier")]
    service_tier: Option<String>,
    price: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct PriceKey {
    event_type: Option<String>,
    metric: String,
    group: String,
    api_zone: Option<String>,
    service_tier: Option<String>,
}

/// Finite unit prices keyed by every billing dimension.
#[derive(Debug, Default)]
pub(super) struct PriceIndex(HashMap<PriceKey, f64>);

impl PriceIndex {
    /// Indexes the finite prices. A later row with the same dimensions
    /// replaces an earlier one.
    pub(super) fn new(prices: Vec<MistralPrice>) -> Self {
        Self(
            prices
                .into_iter()
                .filter_map(|price| {
                    let key = PriceKey {
                        event_type: price.event_type,
                        metric: price.billing_metric?,
                        group: price.billing_group?,
                        api_zone: price.api_zone,
                        service_tier: price.service_tier,
                    };
                    let value = price.price?.parse::<f64>().ok()?;
                    value.is_finite().then_some((key, value))
                })
                .collect(),
        )
    }

    /// Unit price for a usage row: the row with the same dimensions, else the
    /// unqualified legacy row for the same event type, metric and group.
    /// `None` when neither exists or the usage row has no metric or group.
    pub(super) fn unit_price(&self, entry: &UsageEntry) -> Option<f64> {
        let mut key = PriceKey {
            event_type: entry.event_type.clone(),
            metric: entry.billing_metric.clone()?,
            group: entry.billing_group.clone()?,
            api_zone: entry.api_zone.clone(),
            service_tier: entry.service_tier.clone(),
        };
        if let Some(price) = self.0.get(&key) {
            return Some(*price);
        }
        key.api_zone = None;
        key.service_tier = None;
        self.0.get(&key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{BillingResponse, MistralProvider};
    use super::*;

    fn index(prices: serde_json::Value) -> PriceIndex {
        PriceIndex::new(serde_json::from_value(prices).unwrap())
    }

    fn entry(value: serde_json::Value) -> UsageEntry {
        serde_json::from_value(value).unwrap()
    }

    // Upstream `prices entries by event type zone and tier instead of the last
    // matching metric`: trimmed from a real September 2026 response, where the
    // table lists mistral-medium-3-5 input once per zone and tier and then again
    // as a per-second audio price (100x the token price).
    #[test]
    fn prices_entries_by_event_type_zone_and_tier_instead_of_the_last_matching_metric() {
        let usage_row = |group: &str, value: i64| {
            serde_json::json!({
                "usage_type": "vibe",
                "event_type": "api_tokens",
                "billing_metric": "mistral-medium-3-5",
                "billing_display_name": "mistral-vibe-cli-latest",
                "billing_group": group,
                "timestamp": "2026-09-16",
                "value": value,
                "value_paid": value,
                "api_zone": "global",
                "service_tier": "standard"
            })
        };
        let price = |event: &str, group: &str, zone: &str, tier: &str, price: &str| {
            serde_json::json!({
                "event_type": event,
                "billing_metric": "mistral-medium-3-5",
                "billing_group": group,
                "api_zone": zone,
                "service_tier": tier,
                "price": price
            })
        };

        for reversed in [false, true] {
            let mut prices = vec![
                price("api_tokens", "input", "global", "standard", "0.0000012750"),
                price("api_tokens", "input", "eu", "priority", "0.0000023588"),
                price(
                    "api_audio_seconds",
                    "input",
                    "global",
                    "standard",
                    "0.0001416667",
                ),
                price("api_tokens", "cached", "global", "standard", "1.275E-7"),
                price("api_tokens", "cached", "eu", "priority", "2.359E-7"),
                price("api_tokens", "output", "global", "standard", "0.0000063750"),
                price("api_tokens", "output", "eu", "priority", "0.0000117938"),
            ];
            if reversed {
                prices.reverse();
            }
            let billing: BillingResponse = serde_json::from_value(serde_json::json!({
                "vibe_code": { "completion": { "models": {
                    "mistral-vibe-cli-latest::mistral-medium-3-5": {
                        "input": [usage_row("input", 4_375_190)],
                        "cached": [usage_row("cached", 21_628_160)],
                        "output": [usage_row("output", 458_774)]
                    }
                } } },
                "start_date": "2026-09-01T00:00:00Z",
                "end_date": "2026-09-30T23:59:59Z",
                "currency": "EUR",
                "prices": prices
            }))
            .unwrap();

            let summary = MistralProvider::summarize_billing(billing).unwrap();

            let expected =
                4_375_190.0 * 0.000001275 + 21_628_160.0 * 1.275e-7 + 458_774.0 * 0.000006375;
            assert_eq!(summary.total_input_tokens, 4_375_190);
            assert_eq!(summary.total_cached_tokens, 21_628_160);
            assert_eq!(summary.total_output_tokens, 458_774);
            assert!(
                (summary.total_cost - expected).abs() < 1e-9,
                "reversed={reversed}: {} != {expected}",
                summary.total_cost
            );
        }
    }

    // Upstream `legacy prices match qualified usage without guessing a zone or
    // tier`.
    #[test]
    fn legacy_prices_match_qualified_usage_without_guessing_a_zone_or_tier() {
        for (dimension, qualifier, expected_cost) in [
            ("legacy", serde_json::json!({}), 10.0),
            (
                "zone",
                serde_json::json!({ "api_zone": "eu", "service_tier": "standard" }),
                0.0,
            ),
            (
                "tier",
                serde_json::json!({ "api_zone": "global", "service_tier": "priority" }),
                0.0,
            ),
        ] {
            let mut price = serde_json::json!({
                "event_type": "api_tokens",
                "billing_metric": "fixture",
                "billing_group": "input",
                "price": "0.25"
            });
            price
                .as_object_mut()
                .unwrap()
                .extend(qualifier.as_object().unwrap().clone());
            let billing: BillingResponse = serde_json::from_value(serde_json::json!({
                "completion": { "models": { "fixture": { "input": [{
                    "event_type": "api_tokens",
                    "billing_metric": "fixture",
                    "billing_group": "input",
                    "timestamp": "2026-09-16",
                    "value": 100,
                    "value_paid": 40,
                    "api_zone": "global",
                    "service_tier": "standard"
                }] } } },
                "prices": [price]
            }))
            .unwrap();

            let summary = MistralProvider::summarize_billing(billing).unwrap();

            assert_eq!(summary.total_input_tokens, 100, "{dimension}");
            assert_eq!(summary.total_cost, expected_cost, "{dimension}");
        }
    }

    #[test]
    fn exact_dimensions_win_over_the_unqualified_legacy_row() {
        let prices = index(serde_json::json!([
            { "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input", "price": "1" },
            { "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input",
              "api_zone": "eu", "service_tier": "priority", "price": "3" }
        ]));
        let row = |zone: &str, tier: &str| {
            entry(serde_json::json!({
                "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input",
                "api_zone": zone, "service_tier": tier
            }))
        };

        assert_eq!(prices.unit_price(&row("eu", "priority")), Some(3.0));
        assert_eq!(prices.unit_price(&row("global", "standard")), Some(1.0));
    }

    #[test]
    fn a_partly_qualified_price_is_not_a_fallback() {
        // Only a row without both zone and tier is the legacy fallback.
        let prices = index(serde_json::json!([
            { "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input",
              "api_zone": "global", "price": "1" },
            { "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input",
              "service_tier": "standard", "price": "2" }
        ]));

        assert_eq!(
            prices.unit_price(&entry(serde_json::json!({
                "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input",
                "api_zone": "global", "service_tier": "standard"
            }))),
            None
        );
    }

    #[test]
    fn event_type_must_match_even_for_the_legacy_row() {
        let prices = index(serde_json::json!([
            { "event_type": "api_audio_seconds", "billing_metric": "m", "billing_group": "input", "price": "0.5" }
        ]));

        assert_eq!(
            prices.unit_price(&entry(serde_json::json!({
                "event_type": "api_tokens", "billing_metric": "m", "billing_group": "input"
            }))),
            None
        );
        // Rows and prices that both omit every dimension still match, as the
        // tables without event types did before.
        let untyped = index(serde_json::json!([
            { "billing_metric": "m", "billing_group": "input", "price": "0.5" }
        ]));
        assert_eq!(
            untyped.unit_price(&entry(serde_json::json!({
                "billing_metric": "m", "billing_group": "input"
            }))),
            Some(0.5)
        );
    }

    #[test]
    fn rows_without_a_metric_or_group_have_no_price() {
        let prices = index(serde_json::json!([
            { "billing_metric": "m", "billing_group": "input", "price": "0.5" }
        ]));

        assert_eq!(
            prices.unit_price(&entry(serde_json::json!({ "billing_group": "input" }))),
            None
        );
        assert_eq!(
            prices.unit_price(&entry(serde_json::json!({ "billing_metric": "m" }))),
            None
        );
    }
}
