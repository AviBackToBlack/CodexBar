use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::CostCoverageCounts;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalHistoryCoverage {
    Complete,
    Partial,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalTokenHistorySummary {
    pub total_tokens: u64,
    pub session_count: usize,
    pub coverage: LocalHistoryCoverage,
    pub cost_estimate: LocalCostEstimate,
    /// The scan stopped short but every row it decoded is trustworthy, so the
    /// totals are floors ("at least N"), never exact. Readers that cannot tell a
    /// trustworthy subset from contradicted evidence leave this `false`.
    pub lower_bound: bool,
}

impl LocalTokenHistorySummary {
    /// A read that stopped short in a way that makes its rows untrustworthy
    /// (hard budget exhausted, or sources contradicting each other). The scan is
    /// known to be incomplete, but no total is published from it.
    pub(crate) fn withheld() -> Self {
        Self {
            coverage: LocalHistoryCoverage::Partial,
            ..Self::default()
        }
    }

    /// Mark a partial scan that still decoded rows as a lower bound.
    pub(crate) fn with_lower_bound_if_partial(mut self) -> Self {
        self.lower_bound = self.coverage == LocalHistoryCoverage::Partial && self.total_tokens > 0;
        self
    }

    /// Token total safe to publish: exact for a complete scan, a floor for a
    /// marked lower bound, unknown otherwise.
    pub fn published_tokens(&self) -> Option<u64> {
        (self.coverage == LocalHistoryCoverage::Complete || self.lower_bound)
            .then_some(self.total_tokens)
    }

    /// Return a complete list-price total only when both the history scan and
    /// pricing coverage are complete. A complete scan with no token usage is
    /// a known zero even though there were no requests to price.
    pub fn total_usd(&self) -> Option<f64> {
        if self.coverage != LocalHistoryCoverage::Complete {
            return None;
        }
        if self.total_tokens == 0 {
            return Some(0.0);
        }
        self.cost_estimate.complete_total_usd()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCostEstimate {
    /// Sum of requests whose models have public API list prices. This remains
    /// a subtotal when one or more requests are unpriced.
    pub known_subtotal_usd: Option<f64>,
    pub coverage: CostCoverageCounts,
    /// Recorded model names with no public price. Local-only input for an
    /// explicit pricing refresh; never part of the wire contract.
    #[serde(skip)]
    pub unpriced_models: BTreeSet<String>,
}

impl LocalCostEstimate {
    fn complete_total_usd(&self) -> Option<f64> {
        if self.coverage.unpriced == 0 && self.coverage.unmetered == 0 {
            self.known_subtotal_usd
        } else {
            None
        }
    }

    pub(crate) fn record_list_price(&mut self, model: Option<&str>, cost: Option<f64>) {
        let Some(cost) = cost.filter(|value| value.is_finite() && *value >= 0.0) else {
            self.coverage.unpriced = self.coverage.unpriced.saturating_add(1);
            if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
                self.unpriced_models.insert(model.to_string());
            }
            return;
        };
        let next = self.known_subtotal_usd.unwrap_or(0.0) + cost;
        if next.is_finite() {
            self.known_subtotal_usd = Some(next);
            self.coverage.estimated = self.coverage.estimated.saturating_add(1);
        } else {
            self.coverage.unpriced = self.coverage.unpriced.saturating_add(1);
        }
    }
}

pub fn local_token_history_json(
    provider: &str,
    history: &LocalTokenHistorySummary,
    days: u32,
) -> serde_json::Value {
    let complete = history.coverage == LocalHistoryCoverage::Complete;
    let total_usd = history.total_usd();
    let known_subtotal_usd = history.cost_estimate.known_subtotal_usd;
    let published_tokens = history.published_tokens();
    let note = if total_usd.is_some() {
        "Local token history estimated at public API list prices; not billed spend"
    } else if known_subtotal_usd.is_some() && !complete {
        "Known public API list-price subtotal; local history is incomplete (lower bound)"
    } else if known_subtotal_usd.is_some() {
        "Known public API list-price subtotal; some local requests are unpriced"
    } else {
        "Local token history; dollar costs unavailable"
    };
    serde_json::json!({
        "provider": provider,
        "supported": true,
        "days_scanned": days,
        "cost": {
            "total_usd": total_usd,
            "known_subtotal_usd": known_subtotal_usd,
            "currency": total_usd.or(known_subtotal_usd).map(|_| "USD"),
            "pricingCoverage": &history.cost_estimate.coverage,
        },
        "daily": [],
        "tokens": {"total": published_tokens},
        "sessions_count": (complete || history.lower_bound).then_some(history.session_count),
        "tokensAreLowerBound": history.lower_bound,
        "costIsLowerBound": known_subtotal_usd.is_some() && total_usd.is_none(),
        "historyCoverage": match history.coverage {
            LocalHistoryCoverage::Complete => "complete",
            LocalHistoryCoverage::Partial => "partial",
            LocalHistoryCoverage::Unavailable => "unavailable",
        },
        "knownZero": complete && history.total_tokens == 0,
        "note": note,
    })
}
