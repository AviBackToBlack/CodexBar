//! `errorKind` on the provider error rows of `usage --json` (and `--format
//! toon`) and `serve` `/usage` (Issue #640 item 8).
//!
//! The `error` string stays for people. `errorKind` lets a caller act on a
//! failure without parsing English text: it reuses the desktop's provider
//! state names (`needsAuthentication`, `expiredSession`,
//! `localRuntimeOffline`), adds `timeout` and `browserSignInRequired`, and
//! falls back to `unknown`. A `browserSignInRequired` row also carries
//! `signInUrl`, the page that restores the provider's browser session.

use serde_json::{Value, json};

use crate::core::{Provider, ProviderError, ProviderId, ProviderStateKind, instantiate_provider};

/// `errorKind` of a fetch that ran out of time.
pub(crate) const ERROR_KIND_TIMEOUT: &str = "timeout";
/// `errorKind` when nothing more specific is known.
pub(crate) const ERROR_KIND_UNKNOWN: &str = "unknown";
/// `errorKind` of a failure that only a browser sign-in can fix.
const ERROR_KIND_BROWSER_SIGN_IN: &str = "browserSignInRequired";

/// A JSON error row: `{provider, error, errorKind}`, plus `signInUrl` when
/// given.
pub(crate) fn error_row(
    provider_id: ProviderId,
    error: &str,
    kind: &str,
    sign_in_url: Option<&str>,
) -> Value {
    let mut row = json!({
        "provider": provider_id.cli_name(),
        "error": error,
        "errorKind": kind,
    });
    if let Some(url) = sign_in_url {
        row["signInUrl"] = json!(url);
    }
    row
}

/// The error row for a failed provider fetch, classified the way the desktop
/// classifies it (`Provider::error_state_kind`).
pub(crate) fn provider_error_row(provider: &dyn Provider, error: &ProviderError) -> Value {
    classified_error_row(provider.id(), error, |error| {
        provider.error_state_kind(error)
    })
}

/// The error row for a failed `usage` fetch. Failures before the provider
/// runs (account selection, stored keys) are not provider errors and stay
/// `unknown`.
pub(crate) fn usage_error_row(provider_id: ProviderId, error: &anyhow::Error) -> Value {
    match error.downcast_ref::<ProviderError>() {
        Some(provider_error) => {
            provider_error_row(instantiate_provider(provider_id).as_ref(), provider_error)
        }
        None => error_row(provider_id, &error.to_string(), ERROR_KIND_UNKNOWN, None),
    }
}

fn classified_error_row(
    provider_id: ProviderId,
    error: &ProviderError,
    state_kind: impl FnOnce(&ProviderError) -> ProviderStateKind,
) -> Value {
    let (kind, sign_in_url) = match error.without_failure_owner() {
        ProviderError::BrowserSignInRequired { sign_in_url, .. } => {
            (ERROR_KIND_BROWSER_SIGN_IN, Some(sign_in_url.as_str()))
        }
        ProviderError::Timeout => (ERROR_KIND_TIMEOUT, None),
        ProviderError::Network(network) if network.is_timeout() => (ERROR_KIND_TIMEOUT, None),
        other => (state_kind_name(state_kind(other)), None),
    };
    error_row(provider_id, &error.to_string(), kind, sign_in_url)
}

fn state_kind_name(kind: ProviderStateKind) -> &'static str {
    match kind {
        ProviderStateKind::NeedsAuthentication => "needsAuthentication",
        ProviderStateKind::ExpiredSession => "expiredSession",
        ProviderStateKind::LocalRuntimeOffline => "localRuntimeOffline",
        // A failed fetch is never `ready`.
        ProviderStateKind::Ready | ProviderStateKind::Unknown => ERROR_KIND_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::{classified_error_row, error_row, state_kind_name, usage_error_row};
    use crate::core::{ProviderError, ProviderId, ProviderStateKind};
    use serde_json::{Value, json};

    fn kind_of(error: ProviderError) -> Value {
        classified_error_row(ProviderId::Codex, &error, ProviderError::state_kind)["errorKind"]
            .clone()
    }

    #[test]
    fn kinds_reuse_the_desktop_state_names() {
        for kind in [
            ProviderStateKind::NeedsAuthentication,
            ProviderStateKind::ExpiredSession,
            ProviderStateKind::LocalRuntimeOffline,
            ProviderStateKind::Unknown,
        ] {
            assert_eq!(json!(state_kind_name(kind)), json!(kind));
        }
        assert_eq!(state_kind_name(ProviderStateKind::Ready), "unknown");
    }

    #[test]
    fn provider_errors_get_the_kind_the_desktop_shows() {
        assert_eq!(kind_of(ProviderError::AuthRequired), "needsAuthentication");
        assert_eq!(kind_of(ProviderError::NoCookies), "needsAuthentication");
        assert_eq!(
            kind_of(ProviderError::OAuthExpired("Token expired.".to_string())),
            "expiredSession"
        );
        assert_eq!(
            kind_of(ProviderError::Parse("bad body".to_string())),
            "unknown"
        );
        assert_eq!(
            kind_of(ProviderError::Other("API error 500".to_string())),
            "unknown"
        );
        assert_eq!(kind_of(ProviderError::Timeout), "timeout");
        assert_eq!(
            kind_of(ProviderError::OwnedTransport {
                owner: None,
                source: Box::new(ProviderError::Timeout),
            }),
            "timeout"
        );
    }

    #[test]
    fn the_provider_classification_wins_over_the_default() {
        let row = classified_error_row(
            ProviderId::Claude,
            &ProviderError::NotInstalled("Claude CLI not found.".to_string()),
            |_| ProviderStateKind::LocalRuntimeOffline,
        );
        assert_eq!(
            row,
            json!({
                "provider": "claude",
                "error": "Provider not installed: Claude CLI not found.",
                "errorKind": "localRuntimeOffline",
            })
        );
    }

    #[test]
    fn browser_sign_in_rows_name_the_page_to_open() {
        let row = classified_error_row(
            ProviderId::Claude,
            &ProviderError::BrowserSignInRequired {
                message: "Claude usage failed from all configured sources. Sign in.".to_string(),
                sign_in_url: "https://claude.ai/login".to_string(),
            },
            ProviderError::state_kind,
        );
        assert_eq!(
            row,
            json!({
                "provider": "claude",
                "error": "Claude usage failed from all configured sources. Sign in.",
                "errorKind": "browserSignInRequired",
                "signInUrl": "https://claude.ai/login",
            })
        );
        assert!(
            kind_of(ProviderError::AuthRequired) != "browserSignInRequired",
            "only the typed error asks for a browser"
        );
    }

    #[test]
    fn usage_errors_outside_the_provider_stay_unknown() {
        assert_eq!(
            usage_error_row(
                ProviderId::Kimi,
                &anyhow::anyhow!("No token accounts configured for Kimi")
            ),
            json!({
                "provider": "kimi",
                "error": "No token accounts configured for Kimi",
                "errorKind": "unknown",
            })
        );
        assert_eq!(
            error_row(ProviderId::Pi, "pi usage timed out", "timeout", None),
            json!({ "provider": "pi", "error": "pi usage timed out", "errorKind": "timeout" })
        );
    }
}
