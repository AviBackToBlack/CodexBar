//! xKiro daily free-token provider (upstream 0.67.0, #3729).
//!
//! `GET https://api.xkiro.com/v1/usage` with `Authorization: Bearer <key>`.
//! The endpoint is documented as free to call and is account-wide. Only the
//! `free_tokens` counters feed the meter; paid plan windows and wallet
//! balances never count toward free-token headroom. The daily counter resets
//! at 00:00 UTC. Response bodies are never echoed into error messages.
//!
//! Live account behavior is unverified upstream: the fixtures are the
//! documented pay-as-you-go example with a synthetic identity.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use reqwest::header::HeaderMap;
use reqwest::{Client, StatusCode};
use serde_json::{Map, Value};

use crate::core::{
    FetchContext, Provider, ProviderDisplayDetail, ProviderError, ProviderFetchResult, ProviderId,
    ProviderMetadata, RateWindow, SourceMode, UsageSnapshot,
};
use crate::providers::{BoundedBodyError, read_bounded_response};

const USAGE_URL: &str = "https://api.xkiro.com/v1/usage";
const DASHBOARD_URL: &str = "https://xkiro.com";
const CREDENTIAL_TARGET: &str = "codexbar-xkiro";
const ENV_KEYS: &[&str] = &["XKIRO_API_KEY"];
const REQUEST_TIMEOUT_SECS: u64 = 15;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const DAY_MINUTES: u32 = 24 * 60;
/// Largest integer a JSON number can carry without loss (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
const PLAN_PAY_AS_YOU_GO: &str = "Pay as you go";

/// How one `free_tokens` counter appears in the response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counter {
    Absent,
    Null,
    Value(u64),
}

impl Counter {
    fn value(self) -> Option<u64> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::Null => None,
        }
    }
}

/// The parts of a usage response that feed the free-token meter.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FreeTokenUsage {
    used_today: Counter,
    limit_per_day: Counter,
    remaining: Counter,
    email: Option<String>,
    /// `Some(None)` is an explicit `"plan": null` (pay as you go).
    plan: Option<Option<String>>,
}

pub struct XKiroProvider {
    metadata: ProviderMetadata,
    client: Client,
    usage_url: String,
}

impl XKiroProvider {
    pub fn new() -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::XKiro,
                display_name: "xKiro",
                session_label: "Daily free tokens",
                weekly_label: "Weekly",
                supports_opus: false,
                supports_credits: false,
                default_enabled: false,
                is_primary: false,
                dashboard_url: Some(DASHBOARD_URL),
                status_page_url: None,
                tertiary_label_key: None,
            },
            client: crate::core::credentialed_http_client_builder()
                .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            usage_url: USAGE_URL.to_string(),
        }
    }

    /// Point the provider at a local mock server; the production origin stays fixed.
    #[cfg(test)]
    fn with_usage_url(mut self, usage_url: String) -> Self {
        self.client = Client::builder()
            .no_proxy()
            .build()
            .expect("the test client should build");
        self.usage_url = usage_url;
        self
    }

    async fn fetch_api(
        &self,
        api_key: &str,
        now: DateTime<Utc>,
    ) -> Result<ProviderFetchResult, ProviderError> {
        let response = self
            .client
            .get(&self.usage_url)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = response.status();
        validate_status(status, response.headers())?;
        let body = read_bounded_response(response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                BoundedBodyError::TooLarge => unrecognized_response(),
                BoundedBodyError::Read(error) => ProviderError::Network(error),
            })?;
        let usage = parse_usage(&body)?;
        Ok(result_from_usage(&usage, now))
    }
}

impl Default for XKiroProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for XKiroProvider {
    fn id(&self) -> ProviderId {
        ProviderId::XKiro
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::OAuth => {
                let key = crate::providers::resolve_api_key(
                    ctx.api_key.as_deref(),
                    CREDENTIAL_TARGET,
                    ENV_KEYS,
                )?;
                self.fetch_api(&key, Utc::now()).await
            }
            SourceMode::Web | SourceMode::Cli => {
                Err(ProviderError::UnsupportedSource(ctx.source_mode))
            }
        }
    }

    /// Upstream source modes are `auto` + `api`; the API-key path uses the OAuth slot.
    fn available_sources(&self) -> Vec<SourceMode> {
        vec![SourceMode::Auto, SourceMode::OAuth]
    }
}

/// Map an HTTP status to a friendly error. The response body is never read
/// for error statuses, so it cannot leak into a message.
fn validate_status(status: StatusCode, headers: &HeaderMap) -> Result<(), ProviderError> {
    match status {
        StatusCode::OK => Ok(()),
        StatusCode::UNAUTHORIZED => Err(ProviderError::AuthRequired),
        StatusCode::FORBIDDEN => Err(ProviderError::Other(
            "xKiro denied this API key (HTTP 403). Check the key's permissions.".to_string(),
        )),
        StatusCode::TOO_MANY_REQUESTS => {
            let retry_after = retry_after_seconds(
                headers
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok()),
            );
            Err(ProviderError::Other(format!(
                "xKiro rate limit reached; retry after {retry_after:.0}s."
            )))
        }
        status if status.is_server_error() => Err(ProviderError::Other(format!(
            "xKiro is unavailable (HTTP {}).",
            status.as_u16()
        ))),
        status => Err(ProviderError::Other(format!(
            "xKiro returned HTTP {}.",
            status.as_u16()
        ))),
    }
}

/// `Retry-After` in seconds: a non-negative number capped at 10, default 1.
fn retry_after_seconds(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map_or(1.0, |value| value.min(10.0))
}

fn unrecognized_response() -> ProviderError {
    ProviderError::Parse("xKiro returned an unrecognized free-token usage response.".to_string())
}

fn as_record(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

/// A non-negative safe integer, mirroring `Number.isSafeInteger(n) && n >= 0`
/// (so `5.0` is accepted, as JSON.parse makes it the integer `5`).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the float is integral and within 0..=2^53-1 before the cast"
)]
fn safe_counter(number: &serde_json::Number) -> Option<u64> {
    let value = number.as_u64().or_else(|| {
        number
            .as_f64()
            .filter(|float| {
                float.fract() == 0.0 && *float >= 0.0 && *float <= MAX_SAFE_INTEGER as f64
            })
            .map(|float| float as u64)
    })?;
    (value <= MAX_SAFE_INTEGER).then_some(value)
}

fn read_counter(free_tokens: &Map<String, Value>, name: &str) -> Result<Counter, ProviderError> {
    match free_tokens.get(name) {
        None => Ok(Counter::Absent),
        Some(Value::Null) => Ok(Counter::Null),
        Some(Value::Number(number)) => safe_counter(number)
            .map(Counter::Value)
            .ok_or_else(unrecognized_response),
        Some(_) => Err(unrecognized_response()),
    }
}

fn trimmed_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_usage(body: &[u8]) -> Result<FreeTokenUsage, ProviderError> {
    let root: Value = serde_json::from_slice(body).map_err(|_| unrecognized_response())?;
    let root = as_record(Some(&root)).ok_or_else(unrecognized_response)?;
    let free_tokens = as_record(root.get("free_tokens")).ok_or_else(unrecognized_response)?;
    if root.get("object").and_then(Value::as_str) != Some("usage") {
        return Err(unrecognized_response());
    }

    let used_today = read_counter(free_tokens, "used_today")?;
    let limit_per_day = read_counter(free_tokens, "limit_per_day")?;
    let remaining = read_counter(free_tokens, "remaining")?;
    // A body with no usable counter is only meaningful for an explicitly
    // uncapped account (`limit_per_day: null`).
    if used_today.value().is_none()
        && limit_per_day.value().is_none()
        && remaining.value().is_none()
        && limit_per_day != Counter::Null
    {
        return Err(unrecognized_response());
    }

    Ok(FreeTokenUsage {
        used_today,
        limit_per_day,
        remaining,
        email: trimmed_text(as_record(root.get("user")).and_then(|user| user.get("email"))),
        plan: match root.get("plan") {
            Some(Value::Null) => Some(None),
            Some(plan) => trimmed_text(Some(plan)).map(Some),
            None => None,
        },
    })
}

/// The next 00:00 UTC strictly after `now`.
fn next_utc_midnight(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_utc()
        + Duration::days(1)
}

fn format_count(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, digit) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out.chars().rev().collect()
}

/// Percent of the daily allowance used. A zero allowance is exhausted.
fn used_percent(used: u64, limit: u64) -> f64 {
    if limit == 0 {
        100.0
    } else {
        (used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0)
    }
}

fn primary_window(usage: &FreeTokenUsage, now: DateTime<Utc>) -> RateWindow {
    match (usage.used_today.value(), usage.limit_per_day.value()) {
        (Some(used), Some(limit)) => RateWindow::with_details(
            used_percent(used, limit),
            Some(DAY_MINUTES),
            Some(next_utc_midnight(now)),
            None,
        ),
        // Missing counters stay unknown: no percentage is invented.
        _ => RateWindow::informational(match (usage.remaining.value(), usage.limit_per_day) {
            (Some(remaining), _) => format!("{} tokens remaining", format_count(remaining)),
            (None, Counter::Null) => "No daily cap reported".to_string(),
            (None, _) => "Daily free-token usage unavailable".to_string(),
        }),
    }
}

fn result_from_usage(usage: &FreeTokenUsage, now: DateTime<Utc>) -> ProviderFetchResult {
    let mut snapshot = UsageSnapshot::new(primary_window(usage, now));
    if let Some(email) = &usage.email {
        snapshot = snapshot.with_email(email.clone());
    }
    match &usage.plan {
        Some(None) => snapshot = snapshot.with_login_method(PLAN_PAY_AS_YOU_GO),
        Some(Some(plan)) => snapshot = snapshot.with_login_method(plan.clone()),
        None => {}
    }

    let mut result = ProviderFetchResult::new(snapshot, "api");
    if let Some(used) = usage.used_today.value() {
        result = result.with_display_detail(ProviderDisplayDetail::new(
            "tokens-used-today",
            "Tokens used today",
            format_count(used),
        ));
    }
    match usage.limit_per_day {
        Counter::Value(limit) => {
            result = result.with_display_detail(ProviderDisplayDetail::new(
                "daily-allowance",
                "Daily allowance",
                format_count(limit),
            ));
        }
        Counter::Null => {
            result = result.with_display_detail(ProviderDisplayDetail::new(
                "daily-allowance",
                "Daily allowance",
                "No cap reported",
            ));
        }
        Counter::Absent => {}
    }
    if let Some(remaining) = usage.remaining.value() {
        result = result.with_display_detail(ProviderDisplayDetail::new(
            "tokens-remaining",
            "Tokens remaining",
            format_count(remaining),
        ));
    }
    result.with_display_detail(ProviderDisplayDetail::new(
        "daily-reset",
        "Daily reset",
        "00:00 UTC",
    ))
}

#[cfg(test)]
mod tests;
