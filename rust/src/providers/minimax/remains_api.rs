//! Bearer-authenticated MiniMax coding-plan quota fetch.
//!
//! Kept separate from the legacy billing client so the client-rendered console
//! workaround (#425) does not further grow the already-large provider module.

use chrono::Utc;

use crate::core::{FetchContext, ProviderError, ProviderFetchResult};

use super::{MiniMaxRegion, coding_plan, coding_plan_html};

/// A plain MiniMax API key from Settings (`ctx.api_key`) or the environment.
/// Unlike the legacy billing API, the coding-plan remains endpoint does not
/// require a paired `group_id`.
pub(super) fn read_plain_api_key(ctx: &FetchContext) -> Option<String> {
    ctx.api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("MINIMAX_API_KEY")
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty())
        })
}

/// Fetch coding-plan quota using a Bearer API key. Try the platform host, then
/// the www host, matching the existing cookie-based fallback chain.
pub(super) async fn fetch_remains_via_api_key(
    api_key: &str,
    region: MiniMaxRegion,
) -> Result<ProviderFetchResult, ProviderError> {
    let now = Utc::now();
    let urls = [region.coding_plan_remains_url(), region.www_remains_url()];
    let mut last_err: Option<ProviderError> = None;
    for url in urls {
        match fetch_remains_once_via_api_key(api_key, &url).await {
            Ok(snapshot) => {
                let usage = coding_plan_html::to_usage_snapshot(&snapshot, now)?;
                return Ok(ProviderFetchResult::new(usage, "api"));
            }
            Err(err @ ProviderError::Parse(_)) => last_err = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last_err.unwrap_or_else(|| ProviderError::Parse("Missing MiniMax remains URL.".into())))
}

async fn fetch_remains_once_via_api_key(
    api_key: &str,
    url: &str,
) -> Result<coding_plan::MiniMaxCodingPlanSnapshot, ProviderError> {
    let client = crate::core::credentialed_http_client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ProviderError::Other(e.to_string()))?;

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json, text/plain, */*")
        .send()
        .await?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ProviderError::AuthRequired);
    }
    if !status.is_success() {
        let message = format!("MiniMax remains (api key) returned status {status}");
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            return Err(ProviderError::Parse(message));
        }
        return Err(ProviderError::Other(message));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ProviderError::Parse(format!("Failed to parse remains JSON: {e}")))?;
    coding_plan::parse_coding_plan_value(&json, Utc::now())
}
