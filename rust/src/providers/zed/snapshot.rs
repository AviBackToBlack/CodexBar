//! Zed response validation and snapshot mapping shared by the editor and
//! browser-billing lanes (upstream v0.65.0 `zed.js` `fetchUsage`).
//!
//! Every value is validated; drifted shapes fail without publishing totals.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::Value;

use crate::core::{
    CostSnapshot, NamedRateWindow, ProviderDisplayDetail, ProviderError, ProviderFetchResult,
    RateWindow, SubscriptionMetadata, UsageSnapshot,
};

/// `Number.MAX_SAFE_INTEGER`, the upper bound upstream accepts for counts and cents.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const PARSE_MESSAGE: &str = "Could not parse Zed usage response. Its format may have changed.";
const SPEND_PERIOD: &str = "Current billing period";

fn parse_error() -> ProviderError {
    ProviderError::Parse(PARSE_MESSAGE.into())
}

/// A non-negative safe integer (upstream `count`). Stored as `f64` because a
/// safe integer is exactly representable and no truncating cast is needed.
#[derive(Debug, Clone, Copy)]
struct Count(f64);

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        if value.fract() == 0.0 && (0.0..=MAX_SAFE_INTEGER).contains(&value) {
            Ok(Self(value))
        } else {
            Err(D::Error::custom("not a safe non-negative integer"))
        }
    }
}

/// A finite, non-negative amount of cents no larger than the safe integer
/// range (upstream `cents`), converted to dollars on read.
#[derive(Debug, Clone, Copy)]
struct Cents(f64);

impl<'de> Deserialize<'de> for Cents {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        if value.is_finite() && (0.0..=MAX_SAFE_INTEGER).contains(&value) {
            Ok(Self(value))
        } else {
            Err(D::Error::custom("not a safe non-negative amount"))
        }
    }
}

impl Cents {
    fn dollars(self) -> f64 {
        self.0 / 100.0
    }
}

/// A non-blank string (upstream `text`).
#[derive(Debug, Clone)]
struct Text(String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.trim().is_empty() {
            Err(D::Error::custom("blank text"))
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RawLimit {
    Unlimited,
    Null,
    Count(Count),
}

#[derive(Deserialize)]
struct LimitedObject {
    limited: Count,
}

/// `"unlimited"`, `null`, an integer, or `{"limited": n}`; a missing key fails.
fn deserialize_limit<'de, D: Deserializer<'de>>(deserializer: D) -> Result<RawLimit, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::String(text) if text == "unlimited" => Ok(RawLimit::Unlimited),
        Value::Null => Ok(RawLimit::Null),
        value @ Value::Object(_) => LimitedObject::deserialize(value)
            .map(|object| RawLimit::Count(object.limited))
            .map_err(D::Error::custom),
        value => Count::deserialize(value)
            .map(RawLimit::Count)
            .map_err(D::Error::custom),
    }
}

#[derive(Deserialize)]
struct EditPredictions {
    used: Count,
    #[serde(deserialize_with = "deserialize_limit")]
    limit: RawLimit,
}

#[derive(Deserialize)]
struct TokenSpend {
    spend_in_cents: Cents,
    #[serde(default)]
    limit_in_cents: Option<Cents>,
}

#[derive(Deserialize)]
struct BillingUsage {
    edit_predictions: EditPredictions,
    token_spend: TokenSpend,
}

#[derive(Deserialize)]
struct BillingResponse {
    plan: Text,
    current_usage: BillingUsage,
}

#[derive(Deserialize)]
struct EditorUsage {
    edit_predictions: EditPredictions,
}

#[derive(Deserialize)]
struct SubscriptionPeriod {
    started_at: Text,
    ended_at: Text,
}

#[derive(Deserialize)]
struct EditorPlan {
    plan_v3: Text,
    usage: EditorUsage,
    has_overdue_invoices: bool,
    #[serde(default)]
    subscription_period: Option<SubscriptionPeriod>,
}

#[derive(Deserialize)]
struct EditorUser {
    #[serde(rename = "id")]
    _id: Count,
    github_login: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct EditorResponse {
    user: EditorUser,
    plan: EditorPlan,
}

fn parse_root<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ProviderError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| parse_error())?;
    if !value.is_object() {
        return Err(parse_error());
    }
    serde_json::from_value(value).map_err(|_| parse_error())
}

/// Browser billing lane: `GET /frontend/billing/usage`.
pub(super) fn web_result(body: &[u8]) -> Result<ProviderFetchResult, ProviderError> {
    let response: BillingResponse = parse_root(body)?;
    let usage = response.current_usage;
    let primary = primary_window(&usage.edit_predictions, true)?;
    let snapshot = UsageSnapshot::new(primary).with_login_method(plan_label(&response.plan.0));

    let spent = usage.token_spend.spend_in_cents.dollars();
    let cap = usage.token_spend.limit_in_cents.map(Cents::dollars);
    let mut result = ProviderFetchResult::new(snapshot, "web")
        .with_display_detail(ProviderDisplayDetail::new(
            "token-spend",
            "Token spend",
            format_usd(spent),
        ))
        .with_display_detail(ProviderDisplayDetail::new(
            "token-spend-limit",
            "Spend limit",
            cap.map_or_else(|| "Not reported".to_string(), format_usd),
        ));
    if let Some(cap) = cap {
        result = result
            .with_cost(CostSnapshot::new(spent, "USD", SPEND_PERIOD).with_limit(cap))
            .with_display_detail(ProviderDisplayDetail::new(
                "token-spend-remaining",
                "Remaining budget",
                format_usd((cap - spent).max(0.0)),
            ));
    }
    Ok(result)
}

/// Editor lane: `GET /client/users/me` with the editor credential.
pub(super) fn editor_result(
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<ProviderFetchResult, ProviderError> {
    let response: EditorResponse = parse_root(body)?;
    let EditorResponse { user, plan } = response;
    let primary = primary_window(&plan.usage.edit_predictions, false)?;
    let mut snapshot = UsageSnapshot::new(primary).with_login_method(plan_label(&plan.plan_v3.0));
    if !user.github_login.trim().is_empty() {
        snapshot = snapshot.with_email(user.github_login);
    }
    if let Some(name) = user.name.filter(|name| !name.trim().is_empty()) {
        snapshot = snapshot.with_organization(name);
    }
    if plan.has_overdue_invoices {
        snapshot.extra_rate_windows.push(
            NamedRateWindow::new(
                "zed.overdue-invoices",
                "Billing",
                RateWindow::with_details(100.0, None, None, Some("Overdue invoices".into())),
            )
            .with_usage_known(false),
        );
    }
    if let Some(period) = plan.subscription_period {
        let start = parse_instant(&period.started_at.0)?;
        let end = parse_instant(&period.ended_at.0)?;
        let elapsed_percent = if end > start {
            (now - start).num_milliseconds() as f64 / (end - start).num_milliseconds() as f64
                * 100.0
        } else {
            0.0
        };
        snapshot = snapshot
            .with_secondary(RateWindow::with_details(
                elapsed_percent,
                None,
                Some(end),
                Some(cycle_description((end - now).num_seconds())),
            ))
            .with_subscription(Some(SubscriptionMetadata::new(None, None, Some(end))));
    }
    Ok(ProviderFetchResult::new(snapshot, "api"))
}

fn parse_instant(value: &str) -> Result<DateTime<Utc>, ProviderError> {
    DateTime::parse_from_rfc3339(value)
        .map(|instant| instant.with_timezone(&Utc))
        .map_err(|_| parse_error())
}

/// Edit-prediction lane. `null` means unlimited only in the browser payload.
fn primary_window(
    predictions: &EditPredictions,
    null_is_unlimited: bool,
) -> Result<RateWindow, ProviderError> {
    let used = predictions.used.0;
    let limit = match predictions.limit {
        RawLimit::Unlimited => None,
        RawLimit::Null if null_is_unlimited => None,
        RawLimit::Null => return Err(parse_error()),
        RawLimit::Count(limit) => Some(limit.0),
    };
    Ok(match limit {
        None => RateWindow::with_details(0.0, None, None, Some("Unlimited".into())),
        Some(limit) if limit > 0.0 => RateWindow::with_details(
            used / limit * 100.0,
            None,
            None,
            Some(format!("{} / {limit} predictions", used.min(limit))),
        ),
        Some(_) => RateWindow::informational("No edit predictions included"),
    })
}

/// `zed_pro_trial` -> `Zed Pro Trial`.
fn plan_label(plan: &str) -> String {
    plan.split(['_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn cycle_description(remaining_seconds: i64) -> String {
    if remaining_seconds <= 0 {
        return "Cycle ended".into();
    }
    let hours = remaining_seconds / 3600;
    let minutes = remaining_seconds % 3600 / 60;
    if hours >= 24 {
        format!("Cycle ends in {}d {}h", hours / 24, hours % 24)
    } else if hours > 0 {
        format!("Cycle ends in {hours}h {minutes}m")
    } else {
        format!("Cycle ends in {minutes}m")
    }
}

fn format_usd(value: f64) -> String {
    CostSnapshot::new(value, "USD", SPEND_PERIOD).format_used()
}
