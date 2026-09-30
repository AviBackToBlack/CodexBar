//! Optional LiteLLM per-model activity for the last 30 UTC days.
//!
//! Mirrors upstream 0.67.0 `litellm.ts`: up to three pages of
//! `GET /user/daily/activity`, every page strictly validated. Any failure
//! (HTTP status, timeout, malformed or incomplete data) omits the section
//! instead of failing the budget fetch, and never surfaces the response body.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use chrono::{Days, NaiveDate};
use reqwest::{Client, StatusCode, Url};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::{Number, Value};

use crate::core::ProviderDisplayDetail;
use crate::providers::{BoundedBodyError, read_bounded_response};

pub(super) const SECTION_TITLE: &str = "Model activity \u{b7} 30d UTC";
const WINDOW_DAYS: u64 = 29;
const MAX_PAGES: u64 = 3;
const PAGE_SIZE: &str = "1000";
const MAX_DAYS_PER_PAGE: usize = 31;
const MAX_MODELS: usize = 1000;
const MAX_ROWS: usize = 20;
const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// `Number.MAX_SAFE_INTEGER`, the bound upstream applies to every counter.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Fetch and summarise model activity ending on `today` (UTC). Returns no rows
/// when the feature is unavailable, malformed, or the window is empty.
pub(super) async fn fetch(
    client: &Client,
    endpoint: &Url,
    api_key: &str,
    user_id: &str,
    today: NaiveDate,
) -> Vec<ProviderDisplayDetail> {
    match collect(client, endpoint, api_key, user_id, today).await {
        Ok(totals) => detail_rows(totals),
        Err(reason) => {
            tracing::debug!(reason, "LiteLLM model activity omitted");
            Vec::new()
        }
    }
}

async fn collect(
    client: &Client,
    endpoint: &Url,
    api_key: &str,
    user_id: &str,
    today: NaiveDate,
) -> Result<BTreeMap<String, Totals>, &'static str> {
    let start = today
        .checked_sub_days(Days::new(WINDOW_DAYS))
        .ok_or("invalid activity window")?
        .format("%Y-%m-%d")
        .to_string();
    let end = today.format("%Y-%m-%d").to_string();
    let mut totals = BTreeMap::new();

    for page in 1..=MAX_PAGES {
        let mut url = endpoint.clone();
        url.query_pairs_mut()
            .clear()
            .append_pair("user_id", user_id)
            .append_pair("start_date", &start)
            .append_pair("end_date", &end)
            .append_pair("page", &page.to_string())
            .append_pair("page_size", PAGE_SIZE);
        let response = client
            .get(url)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|_| "model activity request failed")?;
        if response.status() != StatusCode::OK {
            return Err("model activity unavailable");
        }
        let body = read_bounded_response(response, MAX_PAGE_BYTES)
            .await
            .map_err(|error| match error {
                BoundedBodyError::TooLarge => "model activity page too large",
                BoundedBodyError::Read(_) => "model activity read failed",
            })?;
        let parsed: Object<Page> =
            serde_json::from_slice(&body).map_err(|_| "invalid activity page")?;
        let Page { results, metadata } = parsed.0;
        if results.len() > MAX_DAYS_PER_PAGE {
            return Err("invalid activity page");
        }
        for row in results {
            accumulate_day(&mut totals, row.0, &start, &end)?;
        }

        let metadata = metadata.map_or_else(Metadata::default, |wrapped| wrapped.0);
        if metadata.page.is_some_and(|reported| reported.0 != page) {
            return Err("repeated activity page");
        }
        let total_pages = metadata.total_pages.map_or(1, |pages| pages.0);
        if !metadata.has_more.unwrap_or(false) && page >= total_pages {
            return Ok(totals);
        }
    }
    Err("incomplete model activity")
}

fn accumulate_day(
    totals: &mut BTreeMap<String, Totals>,
    row: Day,
    start: &str,
    end: &str,
) -> Result<(), &'static str> {
    if !is_iso_day(&row.date) || row.date.as_str() < start || row.date.as_str() > end {
        return Err("invalid activity day");
    }
    for (name, entry) in row.breakdown.0.models.0 {
        if !ProviderDisplayDetail::is_valid_title(&name) {
            return Err("invalid activity model name");
        }
        let counters = entry_counters(&entry)?;
        totals
            .entry(name)
            .or_default()
            .add(&counters)
            .ok_or("invalid activity count")?;
        if totals.len() > MAX_MODELS {
            return Err("too many activity models");
        }
    }
    Ok(())
}

/// Counters live under `metrics` when present, otherwise on the entry itself.
fn entry_counters(entry: &Value) -> Result<Counters, &'static str> {
    let object = entry.as_object().ok_or("invalid activity count")?;
    let source = match object.get("metrics") {
        None | Some(Value::Null) => entry,
        Some(metrics) => metrics,
    };
    Object::<Counters>::deserialize(source)
        .map(|counters| counters.0)
        .map_err(|_| "invalid activity count")
}

fn is_iso_day(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
        && NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

fn detail_rows(totals: BTreeMap<String, Totals>) -> Vec<ProviderDisplayDetail> {
    let mut ranked: Vec<(String, Totals)> = totals.into_iter().collect();
    // `BTreeMap` iterates by name, and the sort is stable, so ties keep name order.
    ranked.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.total));
    ranked
        .into_iter()
        .take(MAX_ROWS)
        .enumerate()
        .filter_map(|(index, (name, totals))| {
            ProviderDisplayDetail::new(
                format!("litellm-model-{index}"),
                name,
                format!(
                    "{} tokens \u{b7} {} requests",
                    totals.total, totals.requests
                ),
            )?
            .with_secondary_value(format!(
                "Input {} \u{b7} Output {}",
                totals.input, totals.output
            ))?
            .with_section(SECTION_TITLE)
        })
        .collect()
}

#[derive(Default)]
struct Totals {
    input: u64,
    output: u64,
    total: u64,
    requests: u64,
}

impl Totals {
    /// Adds one day's counters; `None` if any running sum leaves the safe
    /// integer range.
    fn add(&mut self, counters: &Counters) -> Option<()> {
        let sum = |current: u64, next: u64| {
            current
                .checked_add(next)
                .filter(|value| *value <= MAX_SAFE_INTEGER)
        };
        let input = sum(self.input, counters.prompt_tokens.0)?;
        let output = sum(self.output, counters.completion_tokens.0)?;
        let total = sum(self.total, counters.total_tokens.0)?;
        let requests = sum(self.requests, counters.api_requests.0)?;
        *self = Self {
            input,
            output,
            total,
            requests,
        };
        Some(())
    }
}

/// Deserializes `T` only from a JSON object, so a struct is never filled
/// positionally from an array.
struct Object<T>(T);

impl<'de, T: DeserializeOwned> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if !value.is_object() {
            return Err(D::Error::custom("expected a JSON object"));
        }
        T::deserialize(value).map(Self).map_err(D::Error::custom)
    }
}

/// A non-negative integer no larger than `Number.MAX_SAFE_INTEGER`. Integral
/// floats such as `30.0` are accepted, like a JavaScript number.
#[derive(Clone, Copy)]
struct Count(u64);

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let number = Number::deserialize(deserializer)?;
        safe_count(&number)
            .map(Self)
            .ok_or_else(|| D::Error::custom("invalid count"))
    }
}

fn safe_count(number: &Number) -> Option<u64> {
    if let Some(value) = number.as_u64() {
        return (value <= MAX_SAFE_INTEGER).then_some(value);
    }
    let value = number.as_f64()?;
    if !(value >= 0.0 && value.fract() == 0.0 && value <= MAX_SAFE_INTEGER as f64) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "value is a non-negative integer no larger than 2^53 - 1"
    )]
    let count = value as u64;
    Some(count)
}

#[derive(Deserialize)]
struct Counters {
    prompt_tokens: Count,
    completion_tokens: Count,
    total_tokens: Count,
    api_requests: Count,
}

#[derive(Deserialize)]
struct Page {
    results: Vec<Object<Day>>,
    #[serde(default)]
    metadata: Option<Object<Metadata>>,
}

#[derive(Deserialize)]
struct Day {
    date: String,
    breakdown: Object<Breakdown>,
}

#[derive(Deserialize)]
struct Breakdown {
    models: Object<HashMap<String, Value>>,
}

#[derive(Default, Deserialize)]
struct Metadata {
    page: Option<Count>,
    total_pages: Option<Count>,
    has_more: Option<bool>,
}

#[cfg(test)]
#[path = "model_activity_tests.rs"]
mod tests;
