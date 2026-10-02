use std::{collections::HashMap, path::PathBuf};

use codexbar::currency::{
    CURRENCIES, PreferredCurrency, convert_amount, fetch_exchange_rates, sanitize_rates,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::Mutex;

const CACHE_MAX_AGE_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRates {
    fetched_at_unix: i64,
    rates: HashMap<String, f64>,
}

/// Everything the web UI needs to display preferred currencies. Rust owns the
/// model (supported codes, fallback rates, sanitizing, USD-pivot conversion);
/// the frontend formats with `Intl` only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyRatesSnapshot {
    /// Merged offline-fallback + persisted/fetched rates, sanitized.
    pub rates: HashMap<String, f64>,
    /// Codes `format_display_currency` accepts for `preferredCode`.
    pub supported_codes: Vec<&'static str>,
    /// Normalized preference the snapshot was built for ("AUTO" or a code).
    pub preferred_code: String,
}

/// Shared cache behind the settings command and the tray's cost formatting.
/// Owns the merged rate table and the preference so both paths agree between
/// a fetch and the next read (no second source of truth on disk).
#[derive(Default)]
pub struct CurrencyRateCache {
    inner: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    loaded: bool,
    persisted: Option<PersistedRates>,
    /// Merged, sanitized rates (fallback + persisted), cached alongside.
    rates: HashMap<String, f64>,
    /// Normalized preference the cached table was built for.
    preferred_code: String,
}

#[tauri::command]
pub async fn get_currency_rates(
    app: tauri::AppHandle,
    cache: State<'_, CurrencyRateCache>,
    preferred_currency_code: String,
) -> Result<CurrencyRatesSnapshot, String> {
    let preferred = PreferredCurrency::parse(&preferred_currency_code);
    let mut state = cache.inner.lock().await;
    if !state.loaded {
        state.persisted = read_persisted_rates();
        state.rates = merged_rates(state.persisted.as_ref());
        state.loaded = true;
    }

    if !preferred.is_auto() {
        let now = unix_now();
        let fresh = state.persisted.as_ref().is_some_and(|cached| {
            let age = now.saturating_sub(cached.fetched_at_unix);
            (0..CACHE_MAX_AGE_SECS).contains(&age)
        });
        if !fresh {
            match fetch_exchange_rates().await {
                Ok(rates) => {
                    let entry = PersistedRates {
                        fetched_at_unix: now,
                        rates,
                    };
                    persist_rates(&entry);
                    state.persisted = Some(entry);
                    state.rates = merged_rates(state.persisted.as_ref());
                    crate::tray_bridge::refresh_tray_presentation(&app);
                }
                Err(error) => {
                    tracing::debug!(%error, "currency rates unavailable; using cached or offline rates")
                }
            }
        }
    }
    state.preferred_code = preferred.raw();

    Ok(CurrencyRatesSnapshot {
        rates: state.rates.clone(),
        supported_codes: CURRENCIES.iter().map(|c| c.code).collect(),
        preferred_code: preferred.raw(),
    })
}

/// Read the shared cache for the tray path (blocking on the async mutex; the
/// tray runs off the async runtime). Returns merged rates plus the last
/// command-observed preference, or `None` when the cache was never loaded —
/// the caller then falls back to offline fallback rates via `merged_rates`.
pub(crate) fn cached_rates_and_preference(
    cache: &CurrencyRateCache,
) -> Option<(HashMap<String, f64>, String)> {
    let state = cache.inner.blocking_lock();
    (!state.rates.is_empty() || state.loaded)
        .then(|| (state.rates.clone(), state.preferred_code.clone()))
        .filter(|_| state.loaded)
}

/// Convert `amount` from `source_code` to the saved preferred currency for
/// the tray. Reads the shared cache (synced by `get_currency_rates`); falls
/// back to persisted disk rates when the cache was never touched so a tray
/// render before the first settings load still converts consistently.
pub(crate) fn convert_preferred_amount(
    cache: Option<&CurrencyRateCache>,
    amount: f64,
    source_code: &str,
) -> Option<(f64, String)> {
    let PreferredCurrency::Code(target) =
        PreferredCurrency::parse(&codexbar::settings::Settings::load().preferred_currency_code)
    else {
        return None;
    };
    let rates = match cache {
        Some(cache) => match cached_rates_and_preference(cache) {
            Some((rates, _)) if !rates.is_empty() => rates,
            _ => merged_rates(read_persisted_rates().as_ref()),
        },
        None => merged_rates(read_persisted_rates().as_ref()),
    };
    convert_amount(amount, source_code, target, &rates)
        .map(|converted| (converted, (*target).to_string()))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or_default()
}

fn cache_path() -> Option<PathBuf> {
    codexbar::settings::Settings::settings_path()?
        .parent()
        .map(|parent| parent.join("currency-rates.json"))
}

fn read_persisted_rates() -> Option<PersistedRates> {
    let path = cache_path()?;
    let bytes = std::fs::read(path).ok()?;
    let mut cached: PersistedRates = serde_json::from_slice(&bytes).ok()?;
    // Sanitizing is core-owned (single model, see the thermo review): the
    // shell only delegates.
    cached.rates = sanitize_rates(&cached.rates);
    (!cached.rates.is_empty()).then_some(cached)
}

fn merged_rates(cached: Option<&PersistedRates>) -> HashMap<String, f64> {
    let mut rates = codexbar::currency::fallback_rates();
    if let Some(cached) = cached {
        for (code, rate) in sanitize_rates(&cached.rates) {
            rates.insert(code, rate);
        }
    }
    rates
}

fn persist_rates(cached: &PersistedRates) {
    let Some(path) = cache_path() else { return };
    let Some(parent) = path.parent() else { return };
    let write_result = (|| -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec(cached)?;
        codexbar::atomic_file::write_atomic(&path, &bytes)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        tracing::debug!(%error, "could not persist currency rate cache");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_table_covers_every_preferred_currency() {
        let rates = codexbar::currency::fallback_rates();
        assert_eq!(rates.len(), CURRENCIES.len());
        for currency in CURRENCIES {
            assert!(
                rates
                    .get(currency.code)
                    .is_some_and(|rate| rate.is_finite() && *rate > 0.0)
            );
        }
    }

    #[test]
    fn persisted_rates_are_sanitized_before_merging() {
        let rates = HashMap::from([
            ("USD".to_string(), 1.0),
            ("TRY".to_string(), 48.0),
            ("EUR".to_string(), f64::NAN),
            ("BTC".to_string(), 100.0),
        ]);
        let clean = sanitize_rates(&rates);
        assert_eq!(clean.get("TRY"), Some(&48.0));
        assert!(!clean.contains_key("EUR"));
        assert!(!clean.contains_key("BTC"));
    }

    #[test]
    fn merged_rates_prefer_cached_values_over_fallback() {
        let cached = PersistedRates {
            fetched_at_unix: unix_now(),
            rates: HashMap::from([("USD".to_string(), 1.0), ("TRY".to_string(), 40.0)]),
        };
        let rates = merged_rates(Some(&cached));
        assert_eq!(rates.get("TRY"), Some(&40.0));
        // Unsupported codes in the cache never widen the table.
        assert_eq!(rates.len(), CURRENCIES.len());
    }

    #[test]
    fn cache_reads_return_loaded_state_and_skip_when_empty() {
        let cache = CurrencyRateCache::default();
        assert!(cached_rates_and_preference(&cache).is_none());
    }
}
