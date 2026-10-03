//! DeepSeek Platform balance read through a signed-in web session.
//!
//! The platform dashboard exposes the account wallets at
//! `GET /api/v0/users/get_user_summary`, authorized with the session's
//! `userToken`. This module only reads the balance; it never reads token-level
//! usage or cost.

use std::collections::BTreeMap;

use reqwest::{Client, StatusCode};
use serde::Deserialize;

use super::{BalanceInfo, BalanceResponse, FlexibleF64, FlexibleI64, select_balance_info};
use crate::core::ProviderError;

const PLATFORM_USER_SUMMARY_URL: &str =
    "https://platform.deepseek.com/api/v0/users/get_user_summary";

/// Envelope error codes the platform uses for a missing or expired session.
const AUTH_ERROR_CODES: [i64; 2] = [40002, 40003];

/// Top-level envelope. Error envelopes are not schema-stable, so `data` stays
/// raw until the envelope code has been checked.
#[derive(Debug, Deserialize)]
struct UserSummaryResponse {
    code: Option<FlexibleI64>,
    data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct UserSummaryData {
    biz_code: Option<FlexibleI64>,
    biz_data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct UserSummary {
    #[serde(default)]
    normal_wallets: Vec<Wallet>,
    #[serde(default)]
    bonus_wallets: Vec<Wallet>,
}

#[derive(Debug, Deserialize)]
struct Wallet {
    currency: String,
    balance: FlexibleF64,
}

/// Fetch the wallet balance for one platform session token.
///
/// `ProviderError::AuthRequired` means the platform rejected the session
/// (HTTP 401/403 or an auth envelope code). Transport failures keep their
/// reqwest classification so callers can tell an outage from a rejection.
pub(super) async fn fetch_platform_balance(
    client: &Client,
    token: &str,
) -> Result<BalanceResponse, ProviderError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(ProviderError::AuthRequired);
    }
    let response = client
        .get(PLATFORM_USER_SUMMARY_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("x-client-platform", "web")
        .send()
        .await?;
    let status = response.status();
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Err(ProviderError::AuthRequired);
    }
    if status != StatusCode::OK {
        return Err(ProviderError::Other(format!(
            "DeepSeek Platform returned status {status}"
        )));
    }
    let body = response.bytes().await?;
    parse_platform_balance(&body)
}

pub(super) fn parse_platform_balance(body: &[u8]) -> Result<BalanceResponse, ProviderError> {
    let parse_error = |error: serde_json::Error| {
        ProviderError::Parse(format!(
            "Failed to parse DeepSeek Platform balance: {error}"
        ))
    };
    let envelope: UserSummaryResponse = serde_json::from_slice(body).map_err(parse_error)?;
    check_code(envelope.code, "user summary code")?;

    let data: UserSummaryData = envelope
        .data
        .map(serde_json::from_value)
        .transpose()
        .map_err(parse_error)?
        .ok_or_else(|| ProviderError::Parse("Missing DeepSeek Platform user summary".into()))?;
    check_code(data.biz_code, "user summary biz_code")?;

    let summary: UserSummary = data
        .biz_data
        .map(serde_json::from_value)
        .transpose()
        .map_err(parse_error)?
        .ok_or_else(|| ProviderError::Parse("Missing DeepSeek Platform biz_data".into()))?;
    Ok(balance_response(&summary))
}

fn check_code(code: Option<FlexibleI64>, label: &str) -> Result<(), ProviderError> {
    match code.map(|code| code.0) {
        None | Some(0) => Ok(()),
        Some(code) if AUTH_ERROR_CODES.contains(&code) => Err(ProviderError::AuthRequired),
        Some(code) => Err(ProviderError::Other(format!(
            "DeepSeek Platform {label} {code}"
        ))),
    }
}

/// Sum wallets per currency and express them as the API-key balance shape, so
/// both lanes render through the same snapshot code.
fn balance_response(summary: &UserSummary) -> BalanceResponse {
    let mut totals: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for wallet in &summary.normal_wallets {
        totals.entry(wallet.currency.as_str()).or_default().0 += wallet.balance.0;
    }
    for wallet in &summary.bonus_wallets {
        totals.entry(wallet.currency.as_str()).or_default().1 += wallet.balance.0;
    }

    let balance_infos: Vec<BalanceInfo> = totals
        .into_iter()
        .map(|(currency, (topped_up, granted))| BalanceInfo {
            currency: currency.to_string(),
            total_balance: (topped_up + granted).to_string(),
            granted_balance: granted.to_string(),
            topped_up_balance: topped_up.to_string(),
        })
        .collect();

    let Some(selected) = select_balance_info(&balance_infos) else {
        return BalanceResponse {
            is_available: false,
            balance_infos: vec![BalanceInfo {
                currency: "USD".into(),
                total_balance: "0".into(),
                granted_balance: "0".into(),
                topped_up_balance: "0".into(),
            }],
        };
    };
    let is_available = selected.total_balance.parse::<f64>().unwrap_or(0.0) > 0.0;
    BalanceResponse {
        is_available,
        balance_infos,
    }
}

#[cfg(test)]
#[path = "platform_balance_tests.rs"]
mod tests;
