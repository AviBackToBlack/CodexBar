//! Preferred display currencies and safe conversion of provider cost amounts.

use std::collections::HashMap;
use std::time::Duration;

pub const SUPPORTED_CURRENCY_CODES: &[&str] = &[
    "USD", "GBP", "EUR", "CZK", "CNY", "JPY", "KRW", "CAD", "AUD", "HKD", "TWD", "SGD", "INR",
    "CHF", "AED", "TRY",
];

pub const FALLBACK_RATES: &[(&str, f64)] = &[
    ("USD", 1.0),
    ("GBP", 0.79),
    ("EUR", 0.92),
    ("CZK", 21.0),
    ("CNY", 7.27),
    ("JPY", 154.0),
    ("KRW", 1428.90),
    ("CAD", 1.38),
    ("AUD", 1.55),
    ("HKD", 7.80),
    ("TWD", 32.30),
    ("SGD", 1.34),
    ("INR", 84.50),
    ("CHF", 0.80),
    ("AED", 3.6725),
    ("TRY", 48.5),
];

/// The normalized display-currency preference. `AUTO` keeps every surface in
/// its source units; `Code` converts through the USD pivot. Serialized as the
/// same `AUTO` / ISO-code strings the settings file has always used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferredCurrency {
    Auto,
    Code(&'static str),
}

impl PreferredCurrency {
    /// Normalize raw input: `AUTO` (any case) or a supported ISO code wins,
    /// anything else reads as `AUTO` (mirrors `normalize_preferred_currency`).
    pub fn parse(value: &str) -> Self {
        let code = normalize_preferred_currency(value);
        if code == "AUTO" {
            return Self::Auto;
        }
        Self::Code(
            SUPPORTED_CURRENCY_CODES
                .iter()
                .copied()
                .find(|supported| *supported == code)
                .unwrap_or("USD"),
        )
    }

    /// The persisted form: `"AUTO"` or the ISO code.
    pub fn raw(&self) -> String {
        match self {
            Self::Auto => "AUTO".to_string(),
            Self::Code(code) => (*code).to_string(),
        }
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

pub fn normalize_preferred_currency(value: &str) -> String {
    let code = value.trim().to_ascii_uppercase();
    if code == "AUTO" || SUPPORTED_CURRENCY_CODES.contains(&code.as_str()) {
        code
    } else {
        "AUTO".to_string()
    }
}

/// Keep only finite positive rates for supported codes, with USD pinned to
/// 1.0 (the conversion pivot). Empty when USD is absent or off — a rate table
/// without a correct USD pivot is unusable for conversion.
pub fn sanitize_rates(rates: &HashMap<String, f64>) -> HashMap<String, f64> {
    let mut clean = HashMap::new();
    for code in SUPPORTED_CURRENCY_CODES {
        if let Some(rate) = rates.get(*code).copied()
            && rate.is_finite()
            && rate > 0.0
        {
            clean.insert((*code).to_string(), rate);
        }
    }
    if clean
        .get("USD")
        .is_none_or(|rate| (*rate - 1.0).abs() > f64::EPSILON)
    {
        return HashMap::new();
    }
    clean
}

pub fn fallback_rates() -> HashMap<String, f64> {
    FALLBACK_RATES
        .iter()
        .map(|(code, rate)| ((*code).to_string(), *rate))
        .collect()
}

/// Parse ExchangeRate-API's USD response, retaining only finite positive rates
/// for currencies the display converter knows how to format.
pub fn parse_exchange_rates(payload: &[u8]) -> Option<HashMap<String, f64>> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    if value.get("result")?.as_str()? != "success" || value.get("base_code")?.as_str()? != "USD" {
        return None;
    }
    let rates = value.get("rates")?.as_object()?;
    let mut parsed = HashMap::new();
    for code in SUPPORTED_CURRENCY_CODES {
        let Some(rate) = rates.get(*code).and_then(serde_json::Value::as_f64) else {
            continue;
        };
        if rate.is_finite() && rate > 0.0 {
            parsed.insert((*code).to_string(), rate);
        }
    }
    if parsed
        .get("USD")
        .is_none_or(|rate| (*rate - 1.0).abs() > f64::EPSILON)
    {
        return None;
    }
    Some(parsed)
}

/// Convert between ISO currencies through USD. Unknown/non-ISO units and
/// unavailable or malformed rates return `None` so callers keep source data.
pub fn convert_amount(
    amount: f64,
    source_code: &str,
    target_code: &str,
    rates: &HashMap<String, f64>,
) -> Option<f64> {
    if !amount.is_finite() {
        return None;
    }
    let source = source_code.trim().to_ascii_uppercase();
    let target = target_code.trim().to_ascii_uppercase();
    if !SUPPORTED_CURRENCY_CODES.contains(&source.as_str())
        || !SUPPORTED_CURRENCY_CODES.contains(&target.as_str())
    {
        return None;
    }
    if source == target {
        return Some(amount);
    }
    let source_rate = if source == "USD" {
        1.0
    } else {
        *rates.get(&source)?
    };
    let target_rate = if target == "USD" {
        1.0
    } else {
        *rates.get(&target)?
    };
    if !source_rate.is_finite()
        || source_rate <= 0.0
        || !target_rate.is_finite()
        || target_rate <= 0.0
    {
        return None;
    }
    let converted = (amount / source_rate) * target_rate;
    converted.is_finite().then_some(converted)
}

/// Fetches the shared USD pivot rates with the app's existing proxy setup.
pub async fn fetch_exchange_rates() -> Result<HashMap<String, f64>, String> {
    let builder = crate::core::apply_app_proxy(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));
    let client = builder.build().map_err(|error| error.to_string())?;
    let response = client
        .get("https://open.er-api.com/v6/latest/USD")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Exchange rate service returned HTTP {}",
            response.status()
        ));
    }
    let payload = response.bytes().await.map_err(|error| error.to_string())?;
    parse_exchange_rates(&payload).ok_or_else(|| "Invalid USD exchange-rate response".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_currency_defaults_and_rejects_unknown_values() {
        assert_eq!(normalize_preferred_currency(""), "AUTO");
        assert_eq!(normalize_preferred_currency(" auto "), "AUTO");
        assert_eq!(normalize_preferred_currency("try"), "TRY");
        assert_eq!(normalize_preferred_currency("BTC"), "AUTO");
    }

    #[test]
    fn parses_only_successful_usd_rates_that_are_finite_and_positive() {
        let rates = parse_exchange_rates(
            br#"{"result":"success","base_code":"USD","rates":{"USD":1,"TRY":48.5,"EUR":0.92,"GBP":0,"JPY":-1,"CAD":"bad","BTC":65000}}"#,
        )
        .unwrap();
        assert_eq!(rates.get("USD"), Some(&1.0));
        assert_eq!(rates.get("TRY"), Some(&48.5));
        assert!(!rates.contains_key("GBP"));
        assert!(!rates.contains_key("JPY"));
        assert!(!rates.contains_key("CAD"));
        assert!(!rates.contains_key("BTC"));
        assert!(
            parse_exchange_rates(br#"{"result":"error","base_code":"USD","rates":{"USD":1}}"#)
                .is_none()
        );
        assert!(
            parse_exchange_rates(br#"{"result":"success","base_code":"EUR","rates":{"USD":1}}"#)
                .is_none()
        );
    }

    #[test]
    fn typed_preference_round_trips_and_never_represents_invalid_codes() {
        assert_eq!(PreferredCurrency::parse(""), PreferredCurrency::Auto);
        assert_eq!(PreferredCurrency::parse(" auto "), PreferredCurrency::Auto);
        assert_eq!(
            PreferredCurrency::parse("try"),
            PreferredCurrency::Code("TRY")
        );
        assert_eq!(PreferredCurrency::parse("BTC"), PreferredCurrency::Auto);
        assert_eq!(PreferredCurrency::Code("TRY").raw(), "TRY");
        assert!(PreferredCurrency::Auto.is_auto());
        assert!(!PreferredCurrency::Code("TRY").is_auto());
    }

    #[test]
    fn sanitize_rates_pins_usd_and_drops_unsupported_or_invalid_entries() {
        let mut rates = fallback_rates();
        rates.insert("BTC".into(), 65000.0);
        rates.insert("TRY".into(), f64::NAN);
        rates.insert("USD".into(), 1.0);
        let clean = sanitize_rates(&rates);
        assert_eq!(clean.get("USD"), Some(&1.0));
        assert!(!clean.contains_key("BTC"));
        assert!(!clean.contains_key("TRY"));

        let mut broken = fallback_rates();
        broken.insert("USD".into(), 1.5);
        assert!(sanitize_rates(&broken).is_empty());
    }

    #[test]
    fn fallback_rates_and_usd_pivot_conversion_are_available_offline() {
        let rates = fallback_rates();
        assert_eq!(rates.get("TRY"), Some(&48.5));
        assert_eq!(convert_amount(10.0, "USD", "TRY", &rates), Some(485.0));
        let converted = convert_amount(10.0, "GBP", "TRY", &rates).unwrap();
        assert!((converted - 613.9240506329114).abs() < 1e-10);
    }

    #[test]
    fn unsupported_units_and_invalid_rates_never_convert() {
        let rates = fallback_rates();
        assert_eq!(convert_amount(12.0, "Credits", "TRY", &rates), None);
        assert_eq!(convert_amount(12.0, "USD", "BTC", &rates), None);
        let mut malformed = rates;
        malformed.insert("TRY".into(), f64::NAN);
        assert_eq!(convert_amount(12.0, "USD", "TRY", &malformed), None);
    }
}
