//! Optional-request diagnostics and display rows for the OpenRouter provider
//! (upstream `openrouter.js` at v0.61.0: safe degradation reasons plus the
//! Credits, API key, and Activity detail sections).
//!
//! Reasons are fixed strings chosen from the failure class, never raw
//! response bodies, headers, or keys.

use reqwest::StatusCode;

use super::activity::ActivitySummary;
use super::{CreditsData, KeyData, OpenRouterProvider};
use crate::core::{ProviderDisplayDetail, ProviderError};

const UNAVAILABLE: &str = "Unavailable right now";
pub(super) const ACTIVITY_NOT_CONFIGURED: &str = "Management API key not configured";
pub(super) const ACTIVITY_KEY_REQUIRED: &str = "Management API key required";

/// One optional request that did not produce usable data: the typed error the
/// refresh pipeline acts on, plus the safe reason shown to the user.
#[derive(Debug)]
pub(super) struct Degraded {
    pub(super) error: ProviderError,
    pub(super) reason: String,
}

impl Degraded {
    /// A non-success HTTP status. Statuses in `auth_statuses` are typed as
    /// [`ProviderError::AuthRequired`]; the reason always names the status.
    pub(super) fn http(label: &str, status: StatusCode, auth_statuses: &[StatusCode]) -> Self {
        let error = if auth_statuses.contains(&status) {
            ProviderError::AuthRequired
        } else {
            ProviderError::Other(format!("OpenRouter {label} request returned HTTP {status}"))
        };
        Self {
            error,
            reason: format!("Request returned HTTP {}", status.as_u16()),
        }
    }

    /// A successful HTTP response whose body failed parsing or validation.
    pub(super) fn invalid(error: ProviderError) -> Self {
        Self {
            error,
            reason: "Response was invalid".to_string(),
        }
    }

    /// A response body that failed to decode, unless the body read itself
    /// timed out (then it is a timeout, not an invalid response).
    pub(super) fn body(label: &str, error: reqwest::Error) -> Self {
        if error.is_timeout() {
            return Self::from(error);
        }
        Self::invalid(ProviderError::Parse(format!(
            "OpenRouter {label} response was invalid: {error}"
        )))
    }

    pub(super) fn with_reason(mut self, reason: &str) -> Self {
        self.reason = reason.to_string();
        self
    }
}

impl From<reqwest::Error> for Degraded {
    fn from(error: reqwest::Error) -> Self {
        let reason = if error.is_timeout() {
            "Request timed out"
        } else {
            "Request failed"
        };
        Self {
            error: ProviderError::Network(error),
            reason: reason.to_string(),
        }
    }
}

/// What each optional source reported, with the safe reason when it did not.
pub(super) struct Observations<'a> {
    pub(super) credits: &'a Result<CreditsData, String>,
    pub(super) key: &'a Result<KeyData, String>,
    pub(super) activity: &'a Result<ActivitySummary, String>,
}

pub(super) fn build_display_details(observed: &Observations<'_>) -> Vec<ProviderDisplayDetail> {
    let mut rows = Vec::new();
    match observed.credits {
        Ok(credits) => {
            rows.push(row(
                "credits-remaining",
                "Credits remaining",
                currency(credits.balance()),
            ));
            rows.push(row(
                "credits-used",
                "Credits used",
                currency(credits.total_usage),
            ));
            rows.push(row(
                "credits-total",
                "Credits total added",
                currency(credits.total_credits),
            ));
        }
        Err(reason) => rows.push(unavailable("credits-balance", "Credits balance", reason)),
    }
    match observed.key {
        Ok(key) => rows.extend(key_rows(key)),
        Err(reason) => rows.push(unavailable("key-limit", "API key limit", reason)),
    }
    match observed.activity {
        Ok(summary) => {
            rows.push(
                ProviderDisplayDetail::new(
                    "activity-tokens",
                    "Activity tokens",
                    summary.tokens.to_string(),
                )
                .and_then(|row| row.with_secondary_value("Last 30 completed UTC days")),
            );
            rows.push(row(
                "activity-requests",
                "Activity requests",
                summary.requests.to_string(),
            ));
            rows.push(row(
                "activity-models",
                "Activity models",
                summary.models.to_string(),
            ));
        }
        Err(reason) => rows.push(unavailable(
            "spend-history",
            "Spend history (last 30 days)",
            reason,
        )),
    }
    rows.into_iter().flatten().collect()
}

fn key_rows(key: &KeyData) -> Vec<Option<ProviderDisplayDetail>> {
    let mut rows = Vec::new();
    match key.limit.filter(|limit| *limit > 0.0) {
        Some(limit) => {
            rows.push(
                ProviderDisplayDetail::new("key-limit", "API key limit", currency(limit))
                    .and_then(|row| row.with_secondary_value("Spending cap, not balance")),
            );
            if let Some((_, used, limit)) = OpenRouterProvider::key_quota_metrics(key) {
                rows.push(row(
                    "key-remaining",
                    "API key remaining",
                    currency((limit - used).max(0.0)),
                ));
            }
            if let Some(usage) = key.usage {
                rows.push(row("key-used", "API key used", currency(usage)));
            }
        }
        None => rows.push(row(
            "key-limit",
            "API key limit",
            "No limit configured".to_string(),
        )),
    }
    if let Some(window) = key
        .limit_reset
        .as_deref()
        .map(str::trim)
        .filter(|window| !window.is_empty())
    {
        rows.push(row("key-reset-window", "Reset window", window.to_string()));
    }
    rows
}

fn row(id: &str, title: &str, value: String) -> Option<ProviderDisplayDetail> {
    ProviderDisplayDetail::new(id, title, value)
}

fn unavailable(id: &str, title: &str, reason: &str) -> Option<ProviderDisplayDetail> {
    ProviderDisplayDetail::new(id, title, UNAVAILABLE)
        .and_then(|row| row.with_secondary_value(reason))
}

fn currency(value: f64) -> String {
    format!("${:.2}", value.max(0.0))
}
