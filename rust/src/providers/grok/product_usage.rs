//! Per-product credit shares carried by the Grok credits proxy
//! (`config.productUsage`), shown as plain detail rows beside the total.
//!
//! Every share is a slice of the same credit pool as the primary window, so
//! it never becomes a rate window or progress bar. Shares are trusted only
//! when they compose the credit percentage published in the same payload; any
//! malformed entry drops the whole list so a partial list can never pass the
//! composition check as if it were complete. Follows upstream
//! `GrokCreditsProxyFetcher` (`composingProducts`, `LossyProductUsageArray`)
//! and `GrokProductUsageDetails` (v0.67.0).

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

use crate::core::ProviderDisplayDetail;

/// Rounding allowance between the summed shares and the raw credit percent.
const COMPOSITION_TOLERANCE_PERCENT: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GrokProductUsage {
    pub(super) product: String,
    pub(super) used_percent: f64,
}

#[derive(Deserialize)]
struct RawProductUsage {
    product: String,
    #[serde(rename = "usagePercent")]
    usage_percent: f64,
}

/// `config.productUsage` decoded all-or-nothing: a non-array value or a single
/// malformed entry leaves `None` without failing the surrounding payload.
#[derive(Debug, Default)]
pub(super) struct LossyProductUsage(Option<Vec<GrokProductUsage>>);

impl<'de> Deserialize<'de> for LossyProductUsage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Buffer the value first so a bad shape cannot desynchronize the parser.
        let value = serde_json::Value::deserialize(deserializer)?;
        let products = serde_json::from_value::<Vec<RawProductUsage>>(value)
            .ok()
            .and_then(|raw| {
                raw.into_iter()
                    .map(|entry| {
                        let product = entry.product.trim();
                        (!product.is_empty()
                            && entry.usage_percent.is_finite()
                            && entry.usage_percent >= 0.0)
                            .then(|| GrokProductUsage {
                                product: product.to_string(),
                                used_percent: entry.usage_percent,
                            })
                    })
                    .collect::<Option<Vec<_>>>()
            });
        Ok(Self(products))
    }
}

impl LossyProductUsage {
    /// Shares whose sum matches the raw (unclamped) credit percent from the
    /// same payload; anything else is dropped.
    pub(super) fn composing(self, credit_usage_percent: f64) -> Vec<GrokProductUsage> {
        let products = self.0.unwrap_or_default();
        let sum: f64 = products.iter().map(|entry| entry.used_percent).sum();
        if products.is_empty() || (sum - credit_usage_percent).abs() > COMPOSITION_TOLERANCE_PERCENT
        {
            return Vec::new();
        }
        products
    }
}

fn product_label(product: &str) -> &str {
    match product {
        "GrokBuild" => "Grok Build",
        "GrokChat" => "Grok Chat",
        "GrokImagine" => "Grok Imagine",
        "GrokAppBuilder" => "Grok App Builder",
        other => other,
    }
}

fn format_share(percent: f64) -> String {
    if percent > 0.0 && percent < 1.0 {
        "<1%".to_string()
    } else {
        format!("{percent:.0}%")
    }
}

/// Plain detail rows sorted by share, largest first (ties keep wire order),
/// with zero-share products omitted.
pub(super) fn display_details(products: &[GrokProductUsage]) -> Vec<ProviderDisplayDetail> {
    let mut name_counts = HashMap::new();
    for entry in products {
        *name_counts.entry(entry.product.as_str()).or_insert(0usize) += 1;
    }

    let mut shares: Vec<(usize, &GrokProductUsage)> = products
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.used_percent > 0.0)
        .collect();
    // `sort_by` is stable, so equal shares keep their wire order.
    shares.sort_by(|(_, left), (_, right)| right.used_percent.total_cmp(&left.used_percent));
    shares
        .into_iter()
        .filter_map(|(wire_index, entry)| {
            let id = if name_counts[entry.product.as_str()] > 1 {
                // Repeated product names are distinct wire rows. Keep their ids
                // unique so ProviderFetchResult does not silently discard them.
                format!("grok.product.row.{wire_index}")
            } else {
                format!("grok.product.{}", entry.product)
            };
            ProviderDisplayDetail::new(
                id,
                product_label(&entry.product),
                format_share(entry.used_percent),
            )
        })
        .collect()
}
