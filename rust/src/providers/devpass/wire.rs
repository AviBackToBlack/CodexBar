//! DevPass `/v1/key` wire shapes. Every amount is a plain decimal string and
//! anything else (signs, `NaN`, exponents, hex, empty) fails the parse rather
//! than showing as zero usage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

/// A non-negative, finite amount that arrived as `^\d+(\.\d+)?$`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct Money(f64);

impl Money {
    pub(super) fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<String> for Money {
    type Error = &'static str;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let (whole, fraction) = raw.split_once('.').unwrap_or((&raw, "0"));
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if !(digits(whole) && digits(fraction)) {
            return Err("not a plain decimal amount");
        }
        match raw.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(Self(value)),
            _ => Err("amount is out of range"),
        }
    }
}

/// An ISO-8601 instant with an explicit `Z` or numeric offset.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(try_from = "String")]
struct ResetInstant(DateTime<Utc>);

impl TryFrom<String> for ResetInstant {
    type Error = &'static str;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        // chrono also accepts a lowercase `t`/`z` and a leap second, which
        // the upstream strict shape and `Date` parsing reject.
        if raw.as_bytes().get(10) != Some(&b'T') || raw.ends_with('z') {
            return Err("not a strict ISO-8601 instant");
        }
        let instant = DateTime::parse_from_rfc3339(&raw).map_err(|_| "invalid instant")?;
        if instant.timestamp_subsec_nanos() >= 1_000_000_000 {
            return Err("leap seconds are not valid instants");
        }
        Ok(Self(instant.with_timezone(&Utc)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum DevPlan {
    None,
    Lite,
    Pro,
    Max,
}

impl DevPlan {
    pub(super) fn login_method(self) -> &'static str {
        match self {
            Self::None => "Pay as you go",
            Self::Lite => "DevPass Lite",
            Self::Pro => "DevPass Pro",
            Self::Max => "DevPass Max",
        }
    }
}

/// A field that must be present but may be `null`; a missing field is a
/// parse failure, not `None`.
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn required_nullable_instant<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(required_nullable::<D, ResetInstant>(deserializer)?.map(|instant| instant.0))
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

/// Fields every key returns, including pay-as-you-go keys.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeyData {
    pub(super) usage: Money,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) limit: Option<Money>,
    pub(super) dev_plan: DevPlan,
}

/// Plan fields, required only when the key has a DevPass plan.
#[derive(Debug, Deserialize)]
pub(super) struct PlanFields {
    #[serde(rename = "devPlanCreditsUsed")]
    pub(super) credits_used: Money,
    #[serde(rename = "devPlanCreditsLimit")]
    pub(super) credits_limit: Money,
    #[serde(rename = "devPlanCreditsRemaining")]
    pub(super) credits_remaining: Money,
    #[serde(rename = "devPlanPremiumWeeklyLimit")]
    pub(super) premium_limit: Money,
    #[serde(rename = "devPlanPremiumCreditsUsed")]
    pub(super) premium_used: Money,
    #[serde(
        rename = "devPlanPremiumWeekResetsAt",
        deserialize_with = "required_nullable_instant"
    )]
    pub(super) premium_resets_at: Option<DateTime<Utc>>,
}

pub(super) fn parse_key(body: &[u8]) -> Option<KeyData> {
    serde_json::from_slice::<Envelope<KeyData>>(body)
        .ok()
        .map(|envelope| envelope.data)
}

pub(super) fn parse_plan(body: &[u8]) -> Option<PlanFields> {
    serde_json::from_slice::<Envelope<PlanFields>>(body)
        .ok()
        .map(|envelope| envelope.data)
}
