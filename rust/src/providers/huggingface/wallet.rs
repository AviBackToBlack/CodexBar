use std::future::Future;

use serde_json::Value;

use super::IdentitySnapshot;
use crate::core::{ProviderError, SourceMode};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WalletCandidate {
    pub(super) user_id: String,
    pub(super) balance: f64,
}

pub(super) fn matching_wallet_balance(
    identity: Option<&IdentitySnapshot>,
    candidate: Option<WalletCandidate>,
) -> Option<f64> {
    let expected_user_id = identity?.user_id.as_deref()?;
    let candidate = candidate?;
    (candidate.user_id == expected_user_id).then_some(candidate.balance)
}

pub(super) async fn fetch_matching_wallet_balance<F, Fut>(
    source_mode: SourceMode,
    identity: Option<&IdentitySnapshot>,
    load_candidate: F,
) -> Option<f64>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<WalletCandidate>>,
{
    if source_mode != SourceMode::Auto
        || identity
            .and_then(|identity| identity.user_id.as_deref())
            .is_none()
    {
        return None;
    }
    matching_wallet_balance(identity, load_candidate().await)
}

pub(super) fn parse_wallet_balance(html: &str) -> Result<f64, ProviderError> {
    let mut current = Vec::new();
    let mut legacy = Vec::new();
    let mut rest = html;
    while let Some(index) = rest.find("data-props") {
        rest = &rest[index + "data-props".len()..];
        let trimmed = rest.trim_start();
        let Some(after_equals) = trimmed.strip_prefix('=') else {
            continue;
        };
        let after_equals = after_equals.trim_start();
        let Some(quote) = after_equals
            .chars()
            .next()
            .filter(|value| matches!(value, '\'' | '"'))
        else {
            continue;
        };
        let payload = &after_equals[quote.len_utf8()..];
        let Some(end) = payload.find(quote) else {
            break;
        };
        let decoded = decode_html_entities(&payload[..end])?;
        rest = &payload[end + quote.len_utf8()..];
        let Ok(value) = serde_json::from_str::<Value>(&decoded) else {
            continue;
        };
        let Some(object) = value.as_object() else {
            continue;
        };
        if let Some(entity) = object.get("entity").and_then(Value::as_object)
            && entity.contains_key("currentBalanceUsd")
        {
            if entity.get("type").and_then(Value::as_str) != Some("user") {
                return Err(invalid_wallet("wallet entity type"));
            }
            current.push(wallet_number(
                entity.get("currentBalanceUsd"),
                "currentBalanceUsd",
            )?);
        }
        if object.contains_key("invoiceCreditsCents") {
            let cents = wallet_number(object.get("invoiceCreditsCents"), "invoiceCreditsCents")?;
            if cents.fract() != 0.0 {
                return Err(invalid_wallet("invoiceCreditsCents"));
            }
            legacy.push(cents / 100.0);
        }
    }
    match (current.as_slice(), legacy.as_slice()) {
        ([balance], _) => Ok(*balance),
        ([], [balance]) => Ok(*balance),
        ([], _) => Err(invalid_wallet("missing or ambiguous legacy wallet")),
        _ => Err(invalid_wallet("ambiguous current wallet")),
    }
}

fn wallet_number(value: Option<&Value>, field: &str) -> Result<f64, ProviderError> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| invalid_wallet(field))
}

fn invalid_wallet(field: &str) -> ProviderError {
    ProviderError::Parse(format!("Hugging Face wallet field '{field}' was invalid."))
}

fn decode_html_entities(raw: &str) -> Result<String, ProviderError> {
    let mut output = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index + 1..];
        let Some(end) = rest.find(';') else {
            return Err(invalid_wallet("HTML entity"));
        };
        let entity = &rest[..end];
        let decoded = match entity {
            "amp" => '&',
            "apos" => '\'',
            "gt" => '>',
            "lt" => '<',
            "nbsp" => '\u{00a0}',
            "quot" => '"',
            value if value.starts_with("#x") || value.starts_with("#X") => {
                u32::from_str_radix(&value[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| invalid_wallet("HTML entity"))?
            }
            value if value.starts_with('#') => value[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| invalid_wallet("HTML entity"))?,
            _ => return Err(invalid_wallet("HTML entity")),
        };
        output.push(decoded);
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{DateTime, Duration, Utc};

    use super::*;
    use crate::providers::huggingface::identity_cache::{IdentityCache, get_or_fetch_identity};

    fn identity(user_id: Option<&str>) -> IdentitySnapshot {
        IdentitySnapshot {
            user_id: user_id.map(str::to_string),
            name: Some("fixture".to_string()),
            email: None,
            plan: None,
        }
    }

    fn candidate(user_id: &str, balance: f64) -> WalletCandidate {
        WalletCandidate {
            user_id: user_id.to_string(),
            balance,
        }
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    #[test]
    fn candidate_is_attached_only_to_the_matching_token_identity() {
        let identity = IdentitySnapshot {
            user_id: Some("user-a".to_string()),
            name: None,
            email: None,
            plan: None,
        };
        let candidate = WalletCandidate {
            user_id: "user-a".to_string(),
            balance: 12.5,
        };
        assert_eq!(
            matching_wallet_balance(Some(&identity), Some(candidate.clone())),
            Some(12.5)
        );

        let other_identity = IdentitySnapshot {
            user_id: Some("user-b".to_string()),
            ..identity.clone()
        };
        assert_eq!(
            matching_wallet_balance(Some(&other_identity), Some(candidate.clone())),
            None
        );
        assert_eq!(matching_wallet_balance(None, Some(candidate)), None);
    }

    #[tokio::test]
    async fn wallet_loader_runs_only_for_auto_with_a_user_id() {
        let calls = AtomicUsize::new(0);
        let api_only_identity = identity(Some("user-a"));
        assert_eq!(
            fetch_matching_wallet_balance(SourceMode::OAuth, Some(&api_only_identity), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(candidate("user-a", 12.5)))
            })
            .await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let auto_identity = identity(Some("user-a"));
        assert_eq!(
            fetch_matching_wallet_balance(SourceMode::Auto, Some(&auto_identity), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(candidate("user-a", 12.5)))
            })
            .await,
            Some(12.5)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let no_id_identity = identity(None);
        assert_eq!(
            fetch_matching_wallet_balance(SourceMode::Auto, Some(&no_id_identity), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(candidate("user-a", 12.5)))
            })
            .await,
            None
        );
        assert_eq!(
            fetch_matching_wallet_balance(SourceMode::Auto, None, || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(candidate("user-a", 12.5)))
            })
            .await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wallet_loader_rejects_mismatched_browser_identity() {
        let identity = identity(Some("api-user"));
        let calls = AtomicUsize::new(0);
        assert_eq!(
            fetch_matching_wallet_balance(SourceMode::Auto, Some(&identity), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(candidate("browser-user", 40.0)))
            })
            .await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cached_identity_cannot_keep_wallet_after_browser_account_changes() {
        let token = "fixture-token";
        let now = at(1_800_000_000);
        let cache = std::sync::Mutex::new(IdentityCache::default());
        cache
            .lock()
            .unwrap()
            .insert(token, identity(Some("api-user")), now);
        let cached_identity =
            get_or_fetch_identity(&cache, token, now + Duration::minutes(1), || async {
                panic!("fresh identity should be served from cache")
            })
            .await
            .unwrap();
        let calls = AtomicUsize::new(0);

        let balance =
            fetch_matching_wallet_balance(SourceMode::Auto, Some(&cached_identity), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(candidate("new-browser-user", 40.0)))
            })
            .await;

        assert_eq!(balance, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parser_prefers_unique_current_balance_and_supports_legacy_cents() {
        let current = r#"<div data-props="{&quot;entity&quot;:{&quot;type&quot;:&quot;user&quot;,&quot;currentBalanceUsd&quot;:12.5}}">"#;
        assert_eq!(parse_wallet_balance(current).unwrap(), 12.5);

        let legacy = r#"<div data-props='{"invoiceCreditsCents":725}'">"#;
        assert_eq!(parse_wallet_balance(legacy).unwrap(), 7.25);
    }

    #[test]
    fn parser_rejects_ambiguous_or_non_user_balances() {
        let ambiguous = r#"<div data-props='{"entity":{"type":"user","currentBalanceUsd":1}}'><div data-props='{"entity":{"type":"user","currentBalanceUsd":2}}'>"#;
        assert!(parse_wallet_balance(ambiguous).is_err());
        let organization = r#"<div data-props='{"entity":{"type":"org","currentBalanceUsd":1}}'>"#;
        assert!(parse_wallet_balance(organization).is_err());
    }
}
