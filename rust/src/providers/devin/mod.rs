use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode, Url};
use serde_json::Value;

use crate::core::{
    CostSnapshot, FetchContext, Provider, ProviderError, ProviderFetchResult, ProviderId,
    ProviderMetadata, RateWindow, SourceMode, UsageSnapshot,
};

const CREDENTIAL_TARGET: &str = "codexbar-devin";
const BASE_URL: &str = "https://app.devin.ai/api/";
const ON_DEMAND_PERIOD: &str = "On-demand billing cycle";
const DEFAULT_ORG_ENV_VARS: &[&str] = &["DEVIN_ORGANIZATION", "DEVIN_ORG"];
const TOKEN_ENV_VARS: &[&str] = &["DEVIN_BEARER_TOKEN", "DEVIN_AUTHORIZATION", "DEVIN_API_KEY"];

pub struct DevinProvider {
    metadata: ProviderMetadata,
    client: Client,
}

impl DevinProvider {
    pub fn new() -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::Devin,
                display_name: "Devin",
                session_label: "Daily",
                weekly_label: "Weekly",
                supports_opus: false,
                supports_credits: true,
                default_enabled: false,
                is_primary: false,
                dashboard_url: Some("https://app.devin.ai/settings/billing"),
                status_page_url: None,
            },
            client: crate::core::credentialed_http_client_builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }
}

impl Default for DevinProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for DevinProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Devin
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::OAuth => {
                let token = resolve_bearer_token(ctx.api_key.as_deref())?;
                let organization = resolve_organization(ctx.workspace_id.as_deref())?;
                fetch_quota(&self.client, &token, &organization).await
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

async fn fetch_quota(
    client: &Client,
    token: &str,
    organization: &DevinOrganization,
) -> Result<ProviderFetchResult, ProviderError> {
    let response = send_devin_get(
        client,
        token,
        organization,
        organization.request_url().clone(),
    )
    .send()
    .await?;
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::AuthRequired),
        StatusCode::NOT_FOUND => Err(ProviderError::Other(format!(
            "Devin quota endpoint returned 404 for {}",
            organization.display()
        ))),
        status if !status.is_success() => Err(ProviderError::Other(format!(
            "Devin quota returned status {status} for {}",
            organization.request_url()
        ))),
        _ => {
            let value: Value = response
                .json()
                .await
                .map_err(|e| ProviderError::Parse(format!("Failed to parse Devin quota: {e}")))?;
            let mut billing = fetch_billing_enrichment(client, token, organization).await;
            billing.quota_balance = extra_usage_balance(&value);

            Ok(fetch_result_from_quota(
                &value,
                organization.display(),
                billing,
            ))
        }
    }
}

#[derive(Debug, Default)]
struct DevinBillingEnrichment {
    quota_balance: Option<f64>,
    on_demand: Option<DevinOnDemandUsage>,
    status: Option<DevinBillingStatus>,
}

async fn fetch_billing_enrichment(
    client: &Client,
    token: &str,
    organization: &DevinOrganization,
) -> DevinBillingEnrichment {
    let (on_demand, status) = tokio::join!(
        fetch_on_demand_usage(client, token, organization),
        fetch_billing_status(client, token, organization),
    );

    DevinBillingEnrichment {
        quota_balance: None,
        on_demand: on_demand.ok().flatten(),
        status: status.ok().flatten(),
    }
}

async fn fetch_on_demand_usage(
    client: &Client,
    token: &str,
    organization: &DevinOrganization,
) -> Result<Option<DevinOnDemandUsage>, ProviderError> {
    let response = send_devin_get(
        client,
        token,
        organization,
        organization.on_demand_request_url(),
    )
    .send()
    .await?;

    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::AuthRequired),
        StatusCode::NOT_FOUND => Ok(None),
        status if !status.is_success() => Ok(None),
        _ => {
            let value: Value = response.json().await.map_err(|_| {
                ProviderError::Other("Failed to parse Devin on-demand usage".into())
            })?;
            Ok(parse_on_demand_usage(&value))
        }
    }
}

async fn fetch_billing_status(
    client: &Client,
    token: &str,
    organization: &DevinOrganization,
) -> Result<Option<DevinBillingStatus>, ProviderError> {
    let response = send_devin_get(
        client,
        token,
        organization,
        organization.status_request_url(),
    )
    .send()
    .await?;

    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::AuthRequired),
        StatusCode::NOT_FOUND => Ok(None),
        status if !status.is_success() => Ok(None),
        _ => {
            let value: Value = response
                .json()
                .await
                .map_err(|_| ProviderError::Other("Failed to parse Devin billing status".into()))?;
            Ok(parse_billing_status(&value))
        }
    }
}

fn send_devin_get(
    client: &Client,
    token: &str,
    organization: &DevinOrganization,
    url: Url,
) -> reqwest::RequestBuilder {
    let mut request = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json");
    if let Some(org_id) = organization.x_cog_org_id() {
        request = request.header("x-cog-org-id", org_id);
    }
    request
}

fn resolve_bearer_token(raw: Option<&str>) -> Result<String, ProviderError> {
    let raw = crate::providers::resolve_api_key(raw, CREDENTIAL_TARGET, TOKEN_ENV_VARS)?;
    normalize_bearer_token(&raw).ok_or_else(|| {
        ProviderError::NotInstalled(
            "Devin bearer token not found. Set DEVIN_BEARER_TOKEN, DEVIN_AUTHORIZATION, or Preferences в†’ Providers."
                .into(),
        )
    })
}

fn resolve_organization(raw: Option<&str>) -> Result<DevinOrganization, ProviderError> {
    if let Some(org) = raw.and_then(normalize_organization) {
        return Ok(org);
    }

    for env in DEFAULT_ORG_ENV_VARS {
        if let Ok(value) = std::env::var(env)
            && let Some(org) = normalize_organization(&value)
        {
            return Ok(org);
        }
    }

    Err(ProviderError::NotInstalled(
        "Devin organization not found. Set it in provider extras or DEVIN_ORGANIZATION / DEVIN_ORG."
            .into(),
    ))
}

pub(crate) fn normalize_bearer_token(raw: &str) -> Option<String> {
    let mut value = strip_wrapping_quotes(raw.trim())?.trim().to_string();
    loop {
        let trimmed = value.trim_start();
        if let Some(rest) = strip_case_insensitive_prefix(trimmed, "authorization:") {
            value = rest.trim_start().to_string();
            continue;
        }
        if let Some(rest) = strip_case_insensitive_prefix(trimmed, "bearer ") {
            value = rest.trim_start().to_string();
            continue;
        }
        break;
    }

    let token = value.trim();
    (!token.is_empty()).then_some(token.to_string())
}

pub(crate) fn normalize_organization(raw: &str) -> Option<DevinOrganization> {
    DevinOrganization::parse(raw)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DevinOrganization {
    display: String,
    request_url: Url,
    x_cog_org_id: Option<String>,
}

impl DevinOrganization {
    fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.contains("://") {
            return Self::from_url(trimmed);
        }

        Self::from_reference(trimmed)
    }

    fn from_url(raw: &str) -> Option<Self> {
        let url = Url::parse(raw).ok()?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }

        if url.host_str()? != "app.devin.ai" {
            return None;
        }

        let segments: Vec<_> = url.path_segments()?.collect();
        if segments.is_empty() {
            return None;
        }

        match segments.as_slice() {
            ["api", org_id, "billing", "quota", "usage"] if is_internal_org_id(org_id) => {
                Self::internal(org_id)
            }
            ["org", slug] if is_organization_slug(slug) => Self::slug(slug),
            ["organizations", org_id] if is_internal_org_id(org_id) => Self::internal(org_id),
            _ => None,
        }
    }

    fn from_reference(raw: &str) -> Option<Self> {
        if raw.contains("://") {
            return None;
        }

        let segments: Vec<_> = raw
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        match segments.as_slice() {
            [org_id] if is_internal_org_id(org_id) => Self::internal(org_id),
            [slug] if is_organization_slug(slug) => Self::slug(slug),
            ["org", slug] if is_organization_slug(slug) => Self::slug(slug),
            ["organizations", org_id] if is_internal_org_id(org_id) => Self::internal(org_id),
            _ => None,
        }
    }

    fn slug(slug: &str) -> Option<Self> {
        let slug = validate_identifier(slug)?;
        let path = format!("org/{slug}");
        Some(Self {
            display: format!("org/{slug}"),
            request_url: usage_url(&path),
            x_cog_org_id: None,
        })
    }

    fn internal(org_id: &str) -> Option<Self> {
        let org_id = validate_identifier(org_id)?;
        Some(Self {
            display: org_id.clone(),
            request_url: usage_url(&org_id),
            x_cog_org_id: Some(org_id),
        })
    }

    fn display(&self) -> &str {
        &self.display
    }

    fn request_url(&self) -> &Url {
        &self.request_url
    }

    fn status_request_url(&self) -> Url {
        self.endpoint_request_url("billing/status", None)
    }

    fn on_demand_request_url(&self) -> Url {
        self.endpoint_request_url("billing/quota/my-on-demand-usage", None)
    }

    fn x_cog_org_id(&self) -> Option<&str> {
        self.x_cog_org_id.as_deref()
    }

    fn endpoint_request_url(&self, suffix: &str, query: Option<&str>) -> Url {
        let mut url = self.request_url.clone();
        let path = url.path();
        let prefix = path.strip_suffix("/billing/quota/usage").unwrap_or(path);
        url.set_path(&format!("{prefix}/{suffix}"));
        url.set_query(query);
        url
    }
}

fn usage_url(path: &str) -> Url {
    let url = format!("{BASE_URL}{path}/billing/quota/usage");
    Url::parse(&url).unwrap_or_else(|e| panic!("invalid Devin quota URL {url}: {e}"))
}

fn validate_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || value.len() > 128
    {
        return None;
    }
    Some(value.to_string())
}

fn is_internal_org_id(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with("org_") || value.starts_with("org-")) && validate_identifier(value).is_some()
}

fn is_organization_slug(value: &str) -> bool {
    validate_identifier(value)
        .is_some_and(|value| !value.starts_with("org_") && !value.starts_with("org-"))
}

fn fetch_result_from_quota(
    value: &Value,
    org: &str,
    billing: DevinBillingEnrichment,
) -> ProviderFetchResult {
    let mut usage = snapshot_from_quota(value, org);
    if let Some(method) = billing
        .status
        .as_ref()
        .and_then(|status| status.plan_label.clone())
    {
        usage = usage.with_login_method(method);
    }

    let mut result = ProviderFetchResult::new(usage, "api");
    if let Some(cost) = cost_from_billing(
        billing.quota_balance,
        billing.status.as_ref().and_then(|status| status.balance),
        billing.on_demand.as_ref(),
    ) {
        result = result.with_cost(cost);
    }
    result
}

fn snapshot_from_quota(value: &Value, org: &str) -> UsageSnapshot {
    let daily = percent(value, &["daily_percentage", "dailyPercentage"])
        .unwrap_or_else(|| percent(value, &["used_percent", "usedPercent"]).unwrap_or(0.0));
    let daily_reset_at = timestamp(value, &["daily_reset_at", "dailyResetAt"]);
    let weekly_reset_at = timestamp(value, &["weekly_reset_at", "weeklyResetAt"]);

    let mut snapshot = UsageSnapshot::new(RateWindow::with_details(
        daily,
        Some(1440),
        daily_reset_at,
        None,
    ))
    .with_organization(org.to_string());

    if let Some(weekly) = percent(value, &["weekly_percentage", "weeklyPercentage"]) {
        snapshot = snapshot.with_secondary(RateWindow::with_details(
            weekly,
            Some(10080),
            weekly_reset_at,
            None,
        ));
    }

    snapshot
}

fn percent(value: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(v) = value.get(*key).and_then(Value::as_f64) {
            return Some(if v < 1.0 { v * 100.0 } else { v });
        }
    }

    let used = ["used", "usage", "used_count", "usedCount", "consumed"]
        .iter()
        .find_map(|k| value.get(*k).and_then(Value::as_f64));
    let limit = ["limit", "quota", "total", "max", "available"]
        .iter()
        .find_map(|k| value.get(*k).and_then(Value::as_f64));
    match (used, limit) {
        (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
        _ => None,
    }
}

fn timestamp(value: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_timestamp)
    })
}

fn parse_rfc3339_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn extra_usage_balance(value: &Value) -> Option<f64> {
    let dollars = [
        "overage_balance",
        "overageBalance",
        "extra_usage_balance",
        "extraUsageBalance",
    ]
    .iter()
    .find_map(|key| value.get(*key).and_then(Value::as_f64))
    .filter(|value| value.is_finite() && *value >= 0.0);
    dollars.or_else(|| {
        ["overage_balance_cents", "overageBalanceCents"]
            .iter()
            .find_map(|key| value.get(*key).and_then(Value::as_f64))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|cents| cents / 100.0)
    })
}

#[derive(Debug, Clone, PartialEq)]
struct DevinBillingStatus {
    balance: Option<f64>,
    plan_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct DevinOnDemandUsage {
    amount: Option<f64>,
    cycle_end: Option<DateTime<Utc>>,
}

fn parse_billing_status(value: &Value) -> Option<DevinBillingStatus> {
    let balance = value
        .get("overage_credits")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0);
    let plan_label = value
        .get("plan_slug")
        .and_then(Value::as_str)
        .map(format_plan_label)
        .filter(|label| !label.is_empty());

    if balance.is_none() && plan_label.is_none() {
        None
    } else {
        Some(DevinBillingStatus {
            balance,
            plan_label,
        })
    }
}

fn format_plan_label(plan_slug: &str) -> String {
    let plan = plan_slug.trim();
    if plan.is_empty() {
        return String::new();
    }

    let title = plan
        .split(['-', '_', ' '])
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        String::new()
    } else {
        format!("Devin {title}")
    }
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

fn parse_on_demand_usage(value: &Value) -> Option<DevinOnDemandUsage> {
    let amount = value
        .get("amount")
        .and_then(Value::as_f64)
        .filter(|amount| amount.is_finite() && *amount >= 0.0);
    let cycle_end = timestamp(value, &["cycle_end", "cycleEnd"]);
    if amount.is_none() && cycle_end.is_none() {
        None
    } else {
        Some(DevinOnDemandUsage { amount, cycle_end })
    }
}

fn cost_from_billing(
    quota_balance: Option<f64>,
    status_balance: Option<f64>,
    on_demand: Option<&DevinOnDemandUsage>,
) -> Option<CostSnapshot> {
    let used = on_demand.and_then(|usage| usage.amount);
    let balance = quota_balance.or(status_balance);
    if used.is_none() && balance.is_none() {
        return None;
    }

    let mut cost = CostSnapshot::new(used.unwrap_or(0.0), "USD", ON_DEMAND_PERIOD);
    if let Some(balance) = balance {
        cost = cost.with_balance(balance);
    }
    if let Some(resets_at) = on_demand.and_then(|usage| usage.cycle_end) {
        cost = cost.with_resets_at(resets_at);
    }
    Some(cost)
}

fn strip_wrapping_quotes(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        Some(value[1..value.len() - 1].trim().to_string())
    } else {
        Some(value.to_string())
    }
}

fn strip_case_insensitive_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value.len() < prefix.len() {
        return None;
    }
    let (head, tail) = value.split_at(prefix.len());
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

#[cfg(test)]
mod tests;
