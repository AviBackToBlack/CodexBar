//! Month-to-date spend for deployments that disable the management routes.
//!
//! Upstream 0.65.0 `litellm.ts` (`keyRoot === undefined`): when `/key/info`
//! is unavailable to the key, read the key-scoped spend report for the UTC
//! month to date, and the user-scoped report only when that one is also
//! unavailable. No `api_key` or `internal_user_id` parameter is sent.

use chrono::{DateTime, Utc};
use reqwest::{Client, Url};
use serde::Deserialize;

use crate::core::{CostSnapshot, ProviderError, ProviderFetchResult, RateWindow, UsageSnapshot};
use crate::providers::read_bounded_response;

use super::{check_status, parse_error, route_unavailable, send};

/// Spend reports list one row per key or user; bound the body regardless.
const MAX_SPEND_REPORT_BYTES: usize = 4 * 1024 * 1024;

/// Tries the key-scoped report first and the user-scoped report only when the
/// key report is unavailable; every other failure keeps its class.
pub(super) async fn fetch<F>(
    client: &Client,
    url_for: &F,
    key: &str,
    now: DateTime<Utc>,
) -> Result<ProviderFetchResult, ProviderError>
where
    F: Fn(&str, Option<(&str, &str)>) -> Result<Url, ProviderError>,
{
    let start = now.format("%Y-%m-01").to_string();
    let end = now.format("%Y-%m-%d").to_string();
    let report_url = |path: &str| -> Result<Url, ProviderError> {
        let mut url = url_for(path, Some(("start_date", &start)))?;
        url.query_pairs_mut().append_pair("end_date", &end);
        Ok(url)
    };

    let mut scope = "Key";
    let mut url = report_url("key/spend/report")?;
    let mut response = send(client, url.clone(), key).await?;
    if route_unavailable(response.status()) {
        scope = "User";
        url = report_url("user/spend/report")?;
        response = send(client, url.clone(), key).await?;
    }
    // The report body is never echoed: it lists key identifiers.
    check_status(url.path(), response.status())?;
    let body = read_bounded_response(response, MAX_SPEND_REPORT_BYTES)
        .await
        .map_err(|_| ProviderError::Parse("Failed to read LiteLLM spend report".into()))?;
    let used = parse(&body)?;

    let period = format!("{scope} spend only ({start}\u{2013}{end} UTC)");
    // UsageSnapshot needs a primary lane; a report carries no budget, so the
    // labeled window is informational and the amount lives in the cost.
    let snapshot = UsageSnapshot::new(RateWindow::informational(period.clone()));
    // Upstream's "API spend" card (cost without a limit): the amount is the
    // only usage signal, so it stays visible like the budget-less spend.
    let cost = CostSnapshot::new(used, "USD", period).always_visible();
    Ok(ProviderFetchResult::new(snapshot, "api").with_cost(cost))
}

#[derive(Deserialize)]
struct SpendReportRow {
    total_cost: f64,
}

/// Sum each row's `total_cost` once (model subtotals are ignored). An empty,
/// malformed, negative, or overflowing report is an error, never zero spend.
pub(super) fn parse(body: &[u8]) -> Result<f64, ProviderError> {
    // Serde messages can quote report values, so they are not surfaced.
    let invalid = || parse_error("spend report is empty or invalid");
    let rows: Vec<SpendReportRow> = serde_json::from_slice(body).map_err(|_| invalid())?;
    if rows.is_empty() {
        return Err(invalid());
    }
    let mut used = 0.0;
    for row in rows {
        if row.total_cost < 0.0 {
            return Err(invalid());
        }
        used += row.total_cost;
    }
    if used.is_finite() {
        Ok(used)
    } else {
        Err(invalid())
    }
}
