//! Raycast AI credits payload parsing and snapshot mapping.
//!
//! Amounts arrive as numbers or numeric strings. A malformed amount is a parse
//! failure, never an exact zero.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::core::{
    ProviderDisplayDetail, ProviderError, ProviderFetchResult, RateWindow, SubscriptionMetadata,
    UsageSnapshot,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Credits {
    remaining: Option<f64>,
    total: Option<f64>,
    renewal: Option<DateTime<Utc>>,
    plan: Option<String>,
}

fn invalid(field: &str) -> ProviderError {
    ProviderError::Parse(format!("Invalid Raycast credits response: {field}"))
}

/// `null` and absent keys read as an empty object; any other non-object is invalid.
fn object<'a>(
    value: Option<&'a Value>,
    field: &str,
) -> Result<Option<&'a Map<String, Value>>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(invalid(field)),
    }
}

/// Accepts JSON numbers and strings shaped like `[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?`.
/// The character screen keeps Rust's float parser from accepting `inf` or `nan`.
fn amount(value: Option<&Value>, field: &str) -> Result<Option<f64>, ProviderError> {
    let parsed = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => {
            let text = text.trim();
            let plausible = text
                .chars()
                .all(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E'));
            if plausible {
                text.parse::<f64>().ok()
            } else {
                None
            }
        }
        Some(_) => None,
    };
    parsed
        .filter(|number| number.is_finite())
        .map(Some)
        .ok_or_else(|| invalid(field))
}

fn non_negative_amount(value: Option<&Value>, field: &str) -> Result<Option<f64>, ProviderError> {
    match amount(value, field)? {
        Some(number) if number < 0.0 => Err(invalid(field)),
        other => Ok(other),
    }
}

fn plan_label(tier: &str) -> String {
    match tier {
        "pro" => "Pro".to_string(),
        "pro_plus" => "Pro+".to_string(),
        "max" => "Max".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn parse_credits(body: &str) -> Result<Credits, ProviderError> {
    let root: Value = serde_json::from_str(body).map_err(|_| invalid("expected JSON"))?;
    let root = object(Some(&root), "expected an object")?;
    let field = |name: &str| root.and_then(|map| map.get(name));

    let remaining = non_negative_amount(
        field("remaining_balance_credits"),
        "remaining_balance_credits",
    )?;
    let total = non_negative_amount(field("total_balance_credits"), "total_balance_credits")?;
    if remaining.is_none() && total.is_none() {
        return Err(invalid("no credit amounts"));
    }

    let renewal = match field("next_credits_at") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => Some(
            DateTime::parse_from_rfc3339(raw.trim())
                .map(|date| date.with_timezone(&Utc))
                .map_err(|_| invalid("next_credits_at"))?,
        ),
        Some(_) => return Err(invalid("next_credits_at")),
    };

    let funding = object(field("funding_subscription"), "funding_subscription")?;
    let plan = funding
        .and_then(|map| map.get("tier"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|tier| !tier.is_empty())
        .map(plan_label);

    Ok(Credits {
        remaining,
        total,
        renewal,
        plan,
    })
}

/// No thousands grouping: the meter footnote must match the source amount.
fn format_amount(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    if value.fract() == 0.0 {
        return format!("{value}");
    }
    let fixed = format!("{value:.2}");
    fixed
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

impl Credits {
    /// A positive total with a known balance is one meter; otherwise Left and
    /// Total rows are the only place the amounts appear.
    fn meter(&self) -> Option<(f64, f64)> {
        match (self.remaining, self.total) {
            (Some(remaining), Some(total)) if total > 0.0 => Some((remaining, total)),
            _ => None,
        }
    }

    fn summary(&self) -> String {
        match (self.remaining, self.total) {
            (Some(remaining), Some(total)) => {
                format!(
                    "{} / {} credits left",
                    format_amount(remaining),
                    format_amount(total)
                )
            }
            (Some(remaining), None) => format!("{} credits left", format_amount(remaining)),
            (None, Some(total)) => format!("{} credits total", format_amount(total)),
            (None, None) => String::new(),
        }
    }

    pub(super) fn into_result(self, source_label: &str) -> ProviderFetchResult {
        let description = self.summary();
        let meter = self.meter();
        let primary = match meter {
            Some((remaining, total)) => {
                let used = (total - remaining).max(0.0);
                RateWindow::with_details(
                    (used / total * 100.0).clamp(0.0, 100.0),
                    None,
                    self.renewal,
                    Some(description),
                )
            }
            None => RateWindow::informational(description),
        };
        let mut usage = UsageSnapshot::new(primary);
        if let Some(plan) = &self.plan {
            usage = usage.with_login_method(plan.clone());
        }
        if meter.is_none() && self.renewal.is_some() {
            usage =
                usage.with_subscription(Some(SubscriptionMetadata::new(None, None, self.renewal)));
        }

        let mut result = ProviderFetchResult::new(usage, source_label);
        if meter.is_none() {
            for (id, title, value) in [
                ("credits-left", "Left", self.remaining),
                ("credits-total", "Total", self.total),
            ] {
                if let Some(value) = value {
                    result = result.with_display_detail(ProviderDisplayDetail::new(
                        id,
                        title,
                        format_amount(value),
                    ));
                }
            }
        }
        result
    }
}
