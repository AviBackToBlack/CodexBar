//! Aixy `GET /v1/usage` wire model and validation.
//!
//! The `key.usage` contract is validated strictly so an unrecognised,
//! cross-key or internally inconsistent payload can never publish a
//! misleading balance. Every failure is a generic parse error; response text
//! is never echoed. Amount and count fields stay as raw JSON until the code
//! path that needs them validates them, mirroring upstream, which only checks
//! the branch (hard availability or monitor spend) it actually reads.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::core::ProviderError;

const MAX_BUDGETS: usize = 64;
const MAX_TEXT_CHARS: usize = 120;
/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Deserialize)]
struct WireRoot {
    object: Option<String>,
    currency: Option<String>,
    as_of: Option<Value>,
    key: WireKey,
    budgets: Vec<WireBudget>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireKey {
    id: Option<Value>,
    name: Option<Value>,
    project_id: Option<Value>,
    project_name: Option<Value>,
}

#[derive(Deserialize)]
struct WireBudget {
    id: Option<Value>,
    scope: Option<Value>,
    interval: Option<Value>,
    enforcement: Option<String>,
    shared: Option<bool>,
    limit_usd: Option<Value>,
    applies_to: Option<Vec<WireAppliesTo>>,
    availability: WireAvailability,
    spend_status: Option<Value>,
    spend_usd: Option<Value>,
    remaining_usd: Option<Value>,
    starts_at: Option<Value>,
    resets_at: Option<Value>,
}

#[derive(Deserialize)]
struct WireAppliesTo {
    api_key_id: Option<Value>,
    project_id: Option<Value>,
}

#[derive(Deserialize)]
struct WireAvailability {
    status: Option<Value>,
    spent_usd: Option<Value>,
    reserved_usd: Option<Value>,
    remaining_usd: Option<Value>,
}

#[derive(Deserialize)]
struct WireUsage {
    window: Option<String>,
    requests: Option<Value>,
    total_tokens: Option<Value>,
    attributed_requests: Option<Value>,
    partial_requests: Option<Value>,
    spend_usd: Option<Value>,
}

/// One budget that applies to the reporting key.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Budget {
    pub id: String,
    pub title: String,
    pub known: bool,
    pub hard: bool,
    pub spent: Option<f64>,
    pub reserved: f64,
    pub remaining: Option<f64>,
    pub limit: f64,
    pub used_percent: f64,
    pub window_minutes: Option<u32>,
    pub resets_at: Option<DateTime<Utc>>,
}

/// The key's own last-seven-days totals.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Totals {
    pub requests: u64,
    pub total_tokens: u64,
    pub attributed_requests: u64,
    pub partial_requests: u64,
    pub spend_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct KeyUsage {
    pub key_id: String,
    pub key_label: String,
    pub project_label: String,
    pub as_of: DateTime<Utc>,
    /// Sorted: hard first, known first, higher utilisation first, then id.
    pub budgets: Vec<Budget>,
    pub totals: Option<Totals>,
}

pub(super) fn parse_error(reason: &str) -> ProviderError {
    ProviderError::Parse(format!(
        "Aixy returned an unrecognized usage response ({reason})."
    ))
}

pub(super) fn parse_key_usage(body: &str) -> Result<KeyUsage, ProviderError> {
    let root: WireRoot =
        serde_json::from_str(body).map_err(|_| parse_error("unexpected JSON shape"))?;
    if root.object.as_deref() != Some("key.usage") || root.currency.as_deref() != Some("USD") {
        return Err(parse_error("unsupported usage contract"));
    }
    let as_of =
        date(root.as_of.as_ref())?.ok_or_else(|| parse_error("missing observation time"))?;
    let key_id = text(root.key.id.as_ref())?;
    if root.budgets.len() > MAX_BUDGETS {
        return Err(parse_error("too many budgets"));
    }

    let mut seen = std::collections::HashSet::new();
    let mut budgets = Vec::with_capacity(root.budgets.len());
    for wire in &root.budgets {
        let budget = validate_budget(wire, &root.key, &key_id)?;
        if !seen.insert(budget.id.clone()) {
            return Err(parse_error("duplicate budget"));
        }
        budgets.push(budget);
    }
    budgets.sort_by(|a, b| {
        b.hard
            .cmp(&a.hard)
            .then(b.known.cmp(&a.known))
            .then(b.used_percent.total_cmp(&a.used_percent))
            .then_with(|| a.id.cmp(&b.id))
    });

    let key_label = optional_text(root.key.name.as_ref())?.unwrap_or_else(|| key_id.clone());
    let project_label = match optional_text(root.key.project_name.as_ref())? {
        Some(name) => name,
        None => text(root.key.project_id.as_ref())?,
    };
    let totals = root.usage.as_ref().map(validate_totals).transpose()?;

    Ok(KeyUsage {
        key_label: bounded(&key_label),
        project_label: bounded(&project_label),
        key_id,
        as_of,
        budgets,
        totals,
    })
}

fn validate_budget(
    wire: &WireBudget,
    key: &WireKey,
    key_id: &str,
) -> Result<Budget, ProviderError> {
    let id = text(wire.id.as_ref())?;
    let scope = text(wire.scope.as_ref())?;
    let interval = text(wire.interval.as_ref())?;
    let scope_label = match scope.as_str() {
        "organization" => "Organization",
        "project" => "Project",
        "team" => "Team",
        "user" => "User",
        "api_key" => "Key",
        _ => return Err(parse_error("invalid budget scope or interval")),
    };
    let interval_label = match interval.as_str() {
        "daily" => "Daily",
        "weekly" => "Weekly",
        "monthly" => "Monthly",
        "lifetime" => "Lifetime",
        _ => return Err(parse_error("invalid budget scope or interval")),
    };
    let hard = match wire.enforcement.as_deref() {
        Some("hard") => true,
        Some("monitor") => false,
        _ => return Err(parse_error("invalid enforcement")),
    };
    let shared = wire
        .shared
        .ok_or_else(|| parse_error("invalid shared scope"))?;
    let limit = amount(wire.limit_usd.as_ref())?
        .filter(|limit| *limit > 0.0)
        .ok_or_else(|| parse_error("invalid budget limit"))?;

    let key_project = key_string(key.project_id.as_ref())?;
    let applies = wire
        .applies_to
        .as_deref()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| parse_error("budget belongs to another key"))?;
    for item in applies {
        if key_string(item.api_key_id.as_ref())?.as_deref() != Some(key_id)
            || key_string(item.project_id.as_ref())? != key_project
        {
            return Err(parse_error("budget belongs to another key"));
        }
    }

    let availability = &wire.availability;
    let status = text(availability.status.as_ref())?;
    if !matches!(
        status.as_str(),
        "available" | "unavailable" | "not_enforced"
    ) {
        return Err(parse_error("invalid availability"));
    }
    let spend_status = text(wire.spend_status.as_ref())?;
    if !matches!(spend_status.as_str(), "available" | "unavailable") {
        return Err(parse_error("invalid spend status"));
    }
    let known = if hard {
        status == "available"
    } else {
        spend_status == "available"
    };
    let (spent, reserved, remaining) = if hard {
        (
            amount(availability.spent_usd.as_ref())?,
            amount(availability.reserved_usd.as_ref())?,
            amount(availability.remaining_usd.as_ref())?,
        )
    } else {
        (
            amount(wire.spend_usd.as_ref())?,
            Some(0.0),
            amount(wire.remaining_usd.as_ref())?,
        )
    };
    let used = if known {
        let (Some(spent), Some(reserved), Some(_)) = (spent, reserved, remaining) else {
            return Err(parse_error("incomplete budget balance"));
        };
        let used = spent + reserved;
        if !used.is_finite() {
            return Err(parse_error("budget amount overflow"));
        }
        Some(used)
    } else {
        None
    };

    let starts_at = date(wire.starts_at.as_ref())?;
    let resets_at = date(wire.resets_at.as_ref())?;
    if let (Some(starts), Some(resets)) = (starts_at, resets_at)
        && resets <= starts
    {
        return Err(parse_error("invalid budget period"));
    }
    let window_minutes = starts_at.zip(resets_at).and_then(|(starts, resets)| {
        let millis = (resets - starts).num_milliseconds();
        (millis > 0 && millis % 60_000 == 0)
            .then(|| u32::try_from(millis / 60_000).ok())
            .flatten()
    });

    let title = bounded(&format!(
        "{scope_label} · {interval_label} · {} · {}",
        if shared { "Shared" } else { "Personal" },
        if hard { "Hard" } else { "Monitor" },
    ));
    Ok(Budget {
        id,
        title,
        known,
        hard,
        spent,
        reserved: reserved.unwrap_or(0.0),
        remaining,
        limit,
        used_percent: used.map_or(0.0, |used| percent(used, limit)),
        window_minutes,
        resets_at,
    })
}

fn validate_totals(wire: &WireUsage) -> Result<Totals, ProviderError> {
    if wire.window.as_deref() != Some("7d") {
        return Err(parse_error("unsupported reporting period"));
    }
    let requests = count(wire.requests.as_ref())?;
    let total_tokens = count(wire.total_tokens.as_ref())?;
    let attributed_requests = count(wire.attributed_requests.as_ref())?;
    let partial_requests = count(wire.partial_requests.as_ref())?;
    let spend_usd = amount(wire.spend_usd.as_ref())?;
    if attributed_requests > requests
        || partial_requests > attributed_requests
        || spend_usd.is_some_and(|spend| spend > 0.0 && attributed_requests == 0)
    {
        return Err(parse_error("invalid attribution coverage"));
    }
    Ok(Totals {
        requests,
        total_tokens,
        attributed_requests,
        partial_requests,
        spend_usd,
    })
}

/// A required, non-blank string, trimmed.
fn text(value: Option<&Value>) -> Result<String, ProviderError> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
        _ => Err(parse_error("invalid text")),
    }
}

/// Absent or null is `None`; anything else must be a valid non-blank string.
fn optional_text(value: Option<&Value>) -> Result<Option<String>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        value => text(value).map(Some),
    }
}

/// Raw identifier comparison value: absent and null are equivalent.
fn key_string(value: Option<&Value>) -> Result<Option<String>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(parse_error("invalid identifier")),
    }
}

/// A finite, non-negative number, or a decimal string (`^\d+(\.\d+)?$`).
/// Absent and null are `None`.
fn amount(value: Option<&Value>) -> Result<Option<f64>, ProviderError> {
    let parsed = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) if is_decimal(text) => text.parse::<f64>().ok(),
        Some(_) => None,
    };
    parsed
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(Some)
        .ok_or_else(|| parse_error("invalid amount"))
}

fn is_decimal(text: &str) -> bool {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    digits(whole) && fraction.is_none_or(digits)
}

/// A required amount that must also be a safe integer.
fn count(value: Option<&Value>) -> Result<u64, ProviderError> {
    match amount(value)? {
        Some(value) if value.fract() == 0.0 && value <= MAX_SAFE_INTEGER =>
        {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "value is a non-negative integer no larger than 2^53 - 1"
            )]
            Ok(value as u64)
        }
        _ => Err(parse_error("invalid count")),
    }
}

/// Absent or null is `None`; otherwise an RFC 3339 timestamp.
fn date(value: Option<&Value>) -> Result<Option<DateTime<Utc>>, ProviderError> {
    if matches!(value, None | Some(Value::Null)) {
        return Ok(None);
    }
    let raw = text(value).map_err(|_| parse_error("invalid date"))?;
    DateTime::parse_from_rfc3339(&raw)
        .map(|date| Some(date.with_timezone(&Utc)))
        .map_err(|_| parse_error("invalid date"))
}

fn percent(used: f64, limit: f64) -> f64 {
    if used.is_finite() && limit.is_finite() && limit > 0.0 {
        (used / limit * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// Bound provider-supplied display text and strip control characters.
pub(super) fn bounded(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT_CHARS)
        .collect()
}
