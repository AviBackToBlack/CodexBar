//! Chutes quota payload parsing.
//!
//! Mirrors upstream `chutes.js` (`chutesParse`, `chutesQuota`, `chutesSnapshot`)
//! at v0.66.0: payload keys are matched case- and punctuation-insensitively
//! against fixed key lists, quota-shaped objects are collected in traversal
//! order, and the rolling (4-hour) and monthly quotas are picked by payload
//! key, label, or window length.

use std::collections::HashSet;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Map, Value};

use crate::core::{RateWindow, SubscriptionMetadata, UsageSnapshot};

type Object = Map<String, Value>;

const ROLLING_MINUTES: u32 = 240;
const MONTHLY_MINUTES: u32 = 43_200;
/// Windows of at least four weeks are classified as monthly.
const MONTHLY_MIN_WINDOW_MINUTES: u32 = 40_320;

const ROLLING_PAYLOAD_KEYS: &[&str] = &[
    "rolling",
    "rollingwindow",
    "rolling4h",
    "fourhour",
    "fourhourusage",
    "window4h",
];
const MONTHLY_PAYLOAD_KEYS: &[&str] = &[
    "monthly",
    "monthlyusage",
    "subscription",
    "subscriptionusage",
    "billingperiod",
];
const QUOTA_CONTAINER_KEYS: &[&str] = &[
    "quotas",
    "quota",
    "quotausage",
    "limits",
    "usage",
    "entries",
    "subscriptionusage",
];
const LABEL_KEYS: &[&str] = &[
    "label",
    "name",
    "title",
    "type",
    "quotatype",
    "period",
    "window",
    "windowname",
    "chuteid",
];
const LIMIT_KEYS: &[&str] = &[
    "limit",
    "cap",
    "max",
    "maximum",
    "quota",
    "quotalimit",
    "monthlycap",
    "monthlylimit",
    "requestlimit",
    "tokenlimit",
    "hardlimit",
    "total",
];
const USED_KEYS: &[&str] = &[
    "used",
    "usage",
    "usedamount",
    "consumed",
    "consumedamount",
    "current",
    "currentusage",
    "requests",
    "requestcount",
    "tokens",
    "tokenusage",
    "monthlyusage",
];
const REMAINING_KEYS: &[&str] = &[
    "remaining",
    "available",
    "balance",
    "left",
    "remainingamount",
    "availableamount",
];
const PERCENT_USED_KEYS: &[&str] = &[
    "percentused",
    "usagepercent",
    "usedpercent",
    "utilization",
    "utilizationpercent",
];
const PERCENT_REMAINING_KEYS: &[&str] = &["percentremaining", "remainingpercent"];
const RESET_KEYS: &[&str] = &[
    "resetat",
    "resetsat",
    "resettime",
    "nextresetat",
    "renewsat",
    "renewalat",
    "periodend",
    "currentperiodend",
    "expiresat",
    "windowend",
    "endtime",
];
const UNIT_KEYS: &[&str] = &["unit", "units", "currency", "quotaunit"];
const ACTIVE_KEYS: &[&str] = &[
    "active",
    "isactive",
    "subscriptionactive",
    "hassubscription",
];
const STATUS_KEYS: &[&str] = &["status", "state", "subscriptionstatus"];
const PLAN_KEYS: &[&str] = &[
    "planname",
    "plan",
    "tier",
    "subscriptionplan",
    "subscriptiontier",
];
const SUBSCRIPTION_OBJECT_KEYS: &[&str] = &[
    "subscription",
    "subscriptionusage",
    "currentsubscription",
    "plan",
];
const WRAPPER_KEYS: &[&str] = &["data", "result"];
const WINDOW_MINUTE_KEYS: &[&str] = &["windowminutes", "periodminutes", "durationminutes"];
const WINDOW_HOUR_KEYS: &[&str] = &["windowhours", "periodhours", "durationhours"];
const WINDOW_DAY_KEYS: &[&str] = &["windowdays", "perioddays", "durationdays"];
const WINDOW_SECOND_KEYS: &[&str] = &["windowseconds", "periodseconds", "durationseconds"];
const WINDOW_TEXT_KEYS: &[&str] = &["window", "period", "interval", "duration"];

/// A quota-shaped object projected into a display window.
#[derive(Debug, Clone)]
pub(super) struct QuotaWindow {
    label: Option<String>,
    unit: String,
    /// Canonical form of the source object; identifies the same quota when it
    /// is reachable both by payload key and by traversal.
    raw_key: String,
    rate: RateWindow,
}

/// Quotas selected from one Chutes payload.
#[derive(Debug, Default)]
pub(super) struct ParsedQuotas {
    rolling: Option<QuotaWindow>,
    monthly: Option<QuotaWindow>,
    fallback: Vec<QuotaWindow>,
    plan: Option<String>,
    active: Option<bool>,
    renewal: Option<DateTime<Utc>>,
}

impl ParsedQuotas {
    /// Keep lanes this payload already has and take the rest from `other`
    /// (upstream `??=` merge of `quotas` enrichment).
    pub(super) fn fill_missing_from(&mut self, other: ParsedQuotas) {
        if self.rolling.is_none() {
            self.rolling = other.rolling;
        }
        if self.monthly.is_none() {
            self.monthly = other.monthly;
        }
        self.fallback.extend(other.fallback);
    }

    pub(super) fn has_rolling_and_monthly(&self) -> bool {
        self.rolling.is_some() && self.monthly.is_some()
    }

    pub(super) fn lanes(&self) -> Lanes {
        let with_default_minutes = |window: &QuotaWindow, default: u32| {
            let mut rate = window.rate.clone();
            rate.window_minutes = rate.window_minutes.or(Some(default));
            rate
        };
        let rolling = self
            .rolling
            .as_ref()
            .map(|window| with_default_minutes(window, ROLLING_MINUTES));
        let monthly = self
            .monthly
            .as_ref()
            .map(|window| with_default_minutes(window, MONTHLY_MINUTES));
        let fallback: Vec<&RateWindow> = self.fallback.iter().map(|window| &window.rate).collect();
        let primary = rolling.clone().or_else(|| {
            if monthly.is_none() {
                fallback.first().map(|window| (*window).clone())
            } else {
                None
            }
        });
        // Without a rolling lane the first fallback already became the
        // primary, so the secondary takes the next one.
        let secondary = monthly.or_else(|| {
            let index = usize::from(rolling.is_none());
            fallback.get(index).map(|window| (*window).clone())
        });
        let login_method = self.plan.clone().or_else(|| {
            if self.active == Some(false) {
                Some("No active subscription".to_string())
            } else if self.active.is_none() && primary.is_none() && secondary.is_none() {
                Some("No usage data".to_string())
            } else {
                None
            }
        });
        Lanes {
            primary,
            secondary,
            renewal: self.renewal,
            login_method,
        }
    }
}

/// Primary and secondary windows plus identity resolved from [`ParsedQuotas`]
/// (upstream `chutesSnapshot`).
#[derive(Debug)]
pub(super) struct Lanes {
    primary: Option<RateWindow>,
    secondary: Option<RateWindow>,
    renewal: Option<DateTime<Utc>>,
    login_method: Option<String>,
}

impl Lanes {
    pub(super) fn has_windows(&self) -> bool {
        self.primary.is_some() || self.secondary.is_some()
    }

    pub(super) fn into_usage(self) -> UsageSnapshot {
        // UsageSnapshot requires a primary window; an absent 4-hour lane is
        // shown as informational rather than a fake 0% quota.
        let primary = self
            .primary
            .unwrap_or_else(|| RateWindow::informational("No quota reported"));
        let mut usage = UsageSnapshot::new(primary);
        if let Some(secondary) = self.secondary {
            usage = usage.with_secondary(secondary);
        }
        if let Some(login_method) = self.login_method {
            usage = usage.with_login_method(login_method);
        }
        // Only an explicit subscription date counts; a quota reset is not one.
        if let Some(renewal) = self.renewal {
            usage =
                usage.with_subscription(Some(SubscriptionMetadata::new(None, None, Some(renewal))));
        }
        usage
    }
}

/// Selected quotas as a snapshot, for callers that only need the result.
#[cfg(test)]
pub(super) fn snapshot_from_usage(value: &Value) -> UsageSnapshot {
    parse(value).lanes().into_usage()
}

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Value of the first of `keys` that is present, matching payload keys after
/// lowercasing and dropping non-alphanumerics (upstream `chutesValue`).
fn lookup<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| {
        object
            .iter()
            .find(|(candidate, _)| normalized_key(candidate) == *key)
            .map(|(_, value)| value)
    })
}

fn as_object(value: Option<&Value>) -> Option<&Object> {
    value.and_then(Value::as_object)
}

fn number(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => {
            let cleaned: String = text
                .trim()
                .chars()
                .filter(|c| !matches!(c, ',' | '$' | '%'))
                .collect();
            cleaned.trim().parse::<f64>().ok()
        }
        Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
        _ => None,
    };
    parsed.filter(|number| number.is_finite())
}

pub(super) fn text(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::Bool(flag) => (if *flag { "1" } else { "0" }).to_string(),
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn flag(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => number.as_f64().map(|number| number != 0.0),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "active" => Some(true),
            "false" | "0" | "no" | "inactive" | "none" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn date(value: Option<&Value>) -> Option<DateTime<Utc>> {
    match value? {
        Value::String(raw) => {
            let raw = raw.trim();
            // ISO-8601 / RFC3339 only for strings that look like dates.
            if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
                return Some(parsed.with_timezone(&Utc));
            }
            epoch_to_datetime(raw.parse::<f64>().ok()?)
        }
        Value::Number(number) => number.as_f64().and_then(epoch_to_datetime),
        _ => None,
    }
}

fn epoch_to_datetime(value: f64) -> Option<DateTime<Utc>> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    // Reject small integers that are clearly quota counts, not unix epochs.
    // Unix seconds ~1e9; ms ~1e12. Quota counts are typically << 1e8.
    if value < 1_000_000_000.0 {
        return None;
    }
    let seconds = if value > 10_000_000_000.0 {
        value / 1000.0
    } else {
        value
    };
    // Epoch seconds; the sub-second fraction is below timestamp resolution.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "epoch seconds; sub-second fraction below timestamp resolution"
    )]
    let whole_seconds = seconds as i64;
    Utc.timestamp_opt(whole_seconds, 0).single()
}

fn window_minutes(payload: &Object) -> Option<u32> {
    for (keys, multiplier) in [
        (WINDOW_MINUTE_KEYS, 1.0),
        (WINDOW_HOUR_KEYS, 60.0),
        (WINDOW_DAY_KEYS, 24.0 * 60.0),
        (WINDOW_SECOND_KEYS, 1.0 / 60.0),
    ] {
        if let Some(value) = number(lookup(payload, keys)) {
            return rounded_window_minutes(value * multiplier);
        }
    }
    text(lookup(payload, WINDOW_TEXT_KEYS)).and_then(|raw| parse_window_duration_text(&raw))
}

fn rounded_window_minutes(value: f64) -> Option<u32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let rounded = value.round();
    if rounded <= 0.0 || rounded > u32::MAX as f64 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounded value is bounded by u32::MAX"
    )]
    Some(rounded as u32)
}

fn parse_window_duration_text(raw: &str) -> Option<u32> {
    let compact: String = raw
        .chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    let split_at = compact.find(|character: char| {
        !character.is_ascii_digit() && !matches!(character, '.' | '+' | '-' | 'e')
    })?;
    let (number, suffix) = compact.split_at(split_at);
    let value = number.parse::<f64>().ok()?;
    let multiplier = if suffix.starts_with("min") || suffix == "m" {
        1.0
    } else if suffix.starts_with("hour") || suffix.starts_with("hr") || suffix == "h" {
        60.0
    } else if suffix.starts_with("day") || suffix == "d" {
        24.0 * 60.0
    } else if suffix.starts_with("month") || suffix == "mo" {
        f64::from(MONTHLY_MINUTES)
    } else {
        return None;
    };
    rounded_window_minutes(value * multiplier)
}

pub(super) fn format_quota_amount(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_string();
    }
    let rounded = value.round();
    if (value - rounded).abs() < 0.0001 && rounded >= i64::MIN as f64 && rounded < i64::MAX as f64 {
        // Guarded above: value is within 0.0001 of a whole number, so the
        // fractional part is zero and the rounded value fits in i64.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "finite rounded value is bounded to the i64 range above"
        )]
        let whole = rounded as i64;
        format!("{whole}")
    } else {
        let mut text = format!("{value:.2}");
        while text.contains('.') && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        text
    }
}

/// Build a display window from one quota object (upstream `chutesQuota`).
///
/// Quota counts (`used`/`limit`) go into `reset_description` as detail text
/// (e.g. `"25/100 credits"`). They are never treated as reset schedules.
/// Percent fields are whole percents in 0..=100, never 0..=1 fractions
/// (#408: a real 1% must not become 100%).
fn quota(payload: &Object, label: Option<&str>, minutes: Option<u32>) -> Option<QuotaWindow> {
    let read = |keys| number(lookup(payload, keys));
    let limit = read(LIMIT_KEYS);
    let used = read(USED_KEYS);
    let remaining = read(REMAINING_KEYS);
    let percent_used = read(PERCENT_USED_KEYS).map(|value| value.clamp(0.0, 100.0));
    let percent_remaining =
        read(PERCENT_REMAINING_KEYS).map(|value| 100.0 - value.clamp(0.0, 100.0));
    let total = limit.or_else(|| {
        used.zip(remaining)
            .map(|(used, remaining)| used + remaining)
    });
    let consumed = used.or_else(|| {
        total
            .zip(remaining)
            .map(|(total, remaining)| total - remaining)
    });
    let percent = percent_used.or(percent_remaining).or_else(|| {
        let (total, consumed) = total.zip(consumed)?;
        (total > 0.0).then(|| consumed / total * 100.0)
    })?;
    let unit = text(lookup(payload, UNIT_KEYS)).unwrap_or_else(|| "credits".to_string());
    let described_used = used.or_else(|| {
        limit
            .zip(remaining)
            .map(|(limit, remaining)| (limit - remaining).max(0.0))
    });
    let detail = limit
        .filter(|limit| *limit > 0.0)
        .zip(described_used)
        .map(|(limit, used)| {
            format!(
                "{}/{} {unit}",
                format_quota_amount(used),
                format_quota_amount(limit)
            )
        });
    Some(QuotaWindow {
        label: text(lookup(payload, LABEL_KEYS)).or_else(|| label.map(str::to_string)),
        unit,
        raw_key: Value::Object(payload.clone()).to_string(),
        rate: RateWindow::with_details(
            percent.clamp(0.0, 100.0),
            window_minutes(payload).or(minutes),
            date(lookup(payload, RESET_KEYS)),
            detail,
        ),
    })
}

fn collect_windows(value: &Value, seen: &mut HashSet<String>, windows: &mut Vec<QuotaWindow>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_windows(item, seen, windows);
            }
        }
        Value::Object(object) => collect_object(object, seen, windows),
        _ => {}
    }
}

fn collect_object(object: &Object, seen: &mut HashSet<String>, windows: &mut Vec<QuotaWindow>) {
    if let Some(window) = quota(object, None, None)
        && seen.insert(window.raw_key.clone())
    {
        windows.push(window);
    }
    for value in object.values() {
        collect_windows(value, seen, windows);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuotaKind {
    Rolling,
    Monthly,
}

fn quota_kind(window: &QuotaWindow) -> Option<QuotaKind> {
    let label = format!("{} {}", window.label.as_deref().unwrap_or(""), window.unit).to_lowercase();
    let minutes = window.rate.window_minutes;
    if ["rolling", "4h", "4 h", "4-hour", "four hour", "four-hour"]
        .iter()
        .any(|needle| label.contains(needle))
        || minutes == Some(ROLLING_MINUTES)
    {
        return Some(QuotaKind::Rolling);
    }
    if ["month", "billing", "subscription"]
        .iter()
        .any(|needle| label.contains(needle))
        || minutes.unwrap_or(0) >= MONTHLY_MIN_WINDOW_MINUTES
    {
        return Some(QuotaKind::Monthly);
    }
    None
}

/// Objects searched, in order, for subscription-level fields.
struct Sources<'a> {
    root: &'a Object,
    data: &'a Object,
    subscription: &'a Object,
}

impl Sources<'_> {
    fn context<T>(&self, keys: &[&str], convert: fn(Option<&Value>) -> Option<T>) -> Option<T> {
        convert(lookup(self.root, keys))
            .or_else(|| convert(lookup(self.data, keys)))
            .or_else(|| convert(lookup(self.subscription, keys)))
    }
}

/// Parse a Chutes usage or quota payload (upstream `chutesParse`).
pub(super) fn parse(value: &Value) -> ParsedQuotas {
    let wrapped;
    let root = match value {
        Value::Object(object) => object,
        Value::Array(_) => {
            wrapped = Object::from_iter([("quotas".to_string(), value.clone())]);
            &wrapped
        }
        _ => {
            wrapped = Object::new();
            &wrapped
        }
    };
    let data = as_object(lookup(root, WRAPPER_KEYS)).unwrap_or(root);
    let dictionary =
        |keys: &[&str]| as_object(lookup(root, keys)).or(as_object(lookup(data, keys)));
    let empty = Object::new();
    let sources = Sources {
        root,
        data,
        subscription: dictionary(SUBSCRIPTION_OBJECT_KEYS).unwrap_or(&empty),
    };

    let mut seen = HashSet::new();
    let mut windows = Vec::new();
    for container in [
        lookup(root, QUOTA_CONTAINER_KEYS),
        lookup(data, QUOTA_CONTAINER_KEYS),
    ]
    .into_iter()
    .flatten()
    {
        collect_windows(container, &mut seen, &mut windows);
    }
    collect_object(data, &mut seen, &mut windows);
    collect_object(root, &mut seen, &mut windows);

    let select = |payload_keys: &[&str], label: &str, minutes: u32, kind: QuotaKind| {
        dictionary(payload_keys)
            .and_then(|payload| quota(payload, Some(label), Some(minutes)))
            .or_else(|| {
                windows
                    .iter()
                    .find(|window| quota_kind(window) == Some(kind))
                    .cloned()
            })
    };
    let rolling = select(
        ROLLING_PAYLOAD_KEYS,
        "4-hour quota",
        ROLLING_MINUTES,
        QuotaKind::Rolling,
    );
    let monthly = select(
        MONTHLY_PAYLOAD_KEYS,
        "Monthly quota",
        MONTHLY_MINUTES,
        QuotaKind::Monthly,
    );

    let mut active = sources.context(ACTIVE_KEYS, flag);
    if active.is_none() {
        let status = sources
            .context(STATUS_KEYS, text)
            .unwrap_or_default()
            .to_lowercase();
        if status.contains("active") && !status.contains("inactive") {
            active = Some(true);
        } else if ["free", "inactive", "cancel", "none", "expired"]
            .iter()
            .any(|needle| status.contains(needle))
        {
            active = Some(false);
        }
    }

    let is_selected = |window: &QuotaWindow| {
        [&rolling, &monthly]
            .into_iter()
            .flatten()
            .any(|selected| selected.raw_key == window.raw_key)
    };
    let fallback = windows
        .iter()
        .filter(|window| !is_selected(window))
        .cloned()
        .collect();
    ParsedQuotas {
        plan: sources.context(PLAN_KEYS, text),
        renewal: sources.context(RESET_KEYS, date),
        rolling,
        monthly,
        fallback,
        active,
    }
}
