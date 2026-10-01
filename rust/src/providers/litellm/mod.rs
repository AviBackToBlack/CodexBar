use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use serde::de::DeserializeOwned;

use crate::core::{
    FetchContext, Provider, ProviderError, ProviderFetchResult, ProviderId, ProviderMetadata,
    SourceMode,
};
use crate::providers::{BoundedBodyError, read_bounded_response};

mod endpoint;
mod info;
#[cfg(test)]
mod tests;

use endpoint::management_url;
pub(crate) use endpoint::validated_base_url;
use info::{
    KeyInfoResponse, TeamInfoResponse, UserInfoResponse, bind_key, parse_error, result_from_team,
    result_from_user,
};

const CREDENTIAL_TARGET: &str = "codexbar-litellm";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct LiteLLMProvider {
    metadata: ProviderMetadata,
    client: Client,
}

impl LiteLLMProvider {
    pub fn new() -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::LiteLLM,
                display_name: "LiteLLM",
                session_label: "Personal budget",
                weekly_label: "Team budget",
                supports_opus: false,
                supports_credits: false,
                default_enabled: false,
                is_primary: false,
                dashboard_url: None,
                status_page_url: None,
                tertiary_label_key: None,
            },
            client: crate::core::credentialed_http_client_builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }
}

impl LiteLLMProvider {
    async fn get_json<T: DeserializeOwned>(&self, url: Url, key: &str) -> Result<T, ProviderError> {
        let route = url.path().to_string();
        let response = self
            .client
            .get(url)
            .bearer_auth(key)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ProviderError::AuthRequired);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderError::Other(
                "LiteLLM rate limited the request (HTTP 429).".into(),
            ));
        }
        if !status.is_success() {
            return Err(ProviderError::Other(format!(
                "LiteLLM {route} returned status {status}"
            )));
        }
        let body = read_bounded_response(response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                BoundedBodyError::TooLarge => parse_error("response too large"),
                BoundedBodyError::Read(error) => ProviderError::Network(error),
            })?;
        serde_json::from_slice(&body).map_err(|e| parse_error(format!("{route}: {e}")))
    }
}

impl Default for LiteLLMProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for LiteLLMProvider {
    fn id(&self) -> ProviderId {
        ProviderId::LiteLLM
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    /// Upstream's LiteLLM menu bar resolver: the team budget is enforced for
    /// the key, so Automatic shows it unless a budget is already exhausted.
    fn automatic_metric_prefers_secondary_window(&self) -> bool {
        true
    }

    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::OAuth => {
                let (base, key) = resolve_base_and_key(ctx)?;
                let key_info: KeyInfoResponse = self
                    .get_json(management_url(&base, "key/info", None)?, &key)
                    .await?;
                let binding = bind_key(key_info)?;
                if let Some(user_id) = binding.user_id.as_deref() {
                    let url = management_url(&base, "user/info", Some(("user_id", user_id)))?;
                    let response: UserInfoResponse = self.get_json(url, &key).await?;
                    result_from_user(&binding, user_id, response)
                } else if let Some(team_id) = binding.team_id.as_deref() {
                    let url = management_url(&base, "team/info", Some(("team_id", team_id)))?;
                    let response: TeamInfoResponse = self.get_json(url, &key).await?;
                    result_from_team(&binding, team_id, response)
                } else {
                    Err(parse_error(
                        "LiteLLM key info did not include a user_id or team_id.",
                    ))
                }
            }
            SourceMode::Web | SourceMode::Cli => {
                Err(ProviderError::UnsupportedSource(ctx.source_mode))
            }
        }
    }

    fn available_sources(&self) -> Vec<SourceMode> {
        vec![SourceMode::Auto, SourceMode::OAuth]
    }
}

fn resolve_base_and_key(ctx: &FetchContext) -> Result<(String, String), ProviderError> {
    if let Some(base) = ctx
        .workspace_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let key = ctx.api_key.as_deref().ok_or(ProviderError::AuthRequired)?;
        return Ok((base.to_string(), key.to_string()));
    }

    let key = crate::providers::resolve_api_key(
        ctx.api_key.as_deref(),
        CREDENTIAL_TARGET,
        &["LITELLM_API_KEY"],
    )?;
    let base = std::env::var("LITELLM_BASE_URL")
        .ok()
        .or_else(|| std::env::var("LITELLM_API_BASE").ok())
        .ok_or_else(|| {
            ProviderError::NotInstalled(
                "LiteLLM base URL not found. Set it in provider extras or LITELLM_BASE_URL.".into(),
            )
        })?;
    Ok((base, key))
}
