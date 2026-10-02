//! Fetch-context routing for providers whose cookie source only scopes the
//! browser session (Charm Hyper): the usage source keeps routing, and off or
//! an empty manual source never hands the provider a browser session.

use std::collections::HashMap;

use codexbar::core::{FetchContext, ProviderId, SourceMode};
use codexbar::settings::{ApiKeys, Language, ManualCookies, Settings};

fn hyper_context(
    cookie_source: Option<&str>,
    usage_source: &str,
    cookie: Option<&str>,
) -> FetchContext {
    let mut settings = Settings::default();
    if let Some(cookie_source) = cookie_source {
        settings.set_cookie_source(ProviderId::Hyper, cookie_source);
    }
    settings.set_usage_source(ProviderId::Hyper, usage_source);
    let mut cookies = ManualCookies::default();
    if let Some(cookie) = cookie {
        cookies.set("hyper", cookie);
    }
    let mut api_keys = ApiKeys::default();
    api_keys.set("hyper", "fixture-key", None);
    super::build_fetch_context(
        ProviderId::Hyper,
        &settings,
        &cookies,
        &api_keys,
        &HashMap::new(),
    )
}

#[test]
fn hyper_defaults_to_automatic_session_with_the_provider_owning_the_browser_read() {
    // No stored cookie: the shell must not read the browser itself, so the
    // API source never touches cookies and Auto/Web import inside the provider.
    for (usage_source, expected) in [
        ("auto", SourceMode::Auto),
        ("web", SourceMode::Web),
        ("oauth", SourceMode::OAuth),
    ] {
        let ctx = hyper_context(None, usage_source, None);
        assert_eq!(ctx.source_mode, expected, "{usage_source}");
        assert!(ctx.manual_cookie_header.is_none());
        assert!(!ctx.manual_cookie_missing);
        assert_eq!(ctx.api_key.as_deref(), Some("fixture-key"));
    }
}

#[test]
fn hyper_manual_cookie_keeps_the_usage_source() {
    for (usage_source, expected) in [
        ("auto", SourceMode::Auto),
        ("web", SourceMode::Web),
        ("oauth", SourceMode::OAuth),
    ] {
        let ctx = hyper_context(Some("manual"), usage_source, Some("session=fixture"));
        assert_eq!(ctx.source_mode, expected, "{usage_source}");
        assert_eq!(ctx.manual_cookie_header.as_deref(), Some("session=fixture"));
        assert!(!ctx.manual_cookie_missing);
    }
}

#[test]
fn hyper_empty_manual_source_falls_back_to_the_key_without_a_browser_session() {
    let ctx = hyper_context(Some("manual"), "auto", None);
    assert_eq!(ctx.source_mode, SourceMode::Auto);
    assert!(ctx.manual_cookie_header.is_none());
    assert!(ctx.manual_cookie_missing);
    assert_eq!(ctx.api_key.as_deref(), Some("fixture-key"));
}

#[test]
fn hyper_cookie_off_ignores_a_stored_cookie_and_keeps_the_usage_source() {
    for (usage_source, expected) in [
        ("auto", SourceMode::Auto),
        ("web", SourceMode::Web),
        ("oauth", SourceMode::OAuth),
    ] {
        let ctx = hyper_context(Some("off"), usage_source, Some("session=fixture"));
        assert_eq!(ctx.source_mode, expected, "{usage_source}");
        assert!(ctx.manual_cookie_header.is_none());
        assert!(ctx.manual_cookie_missing);
    }
}

#[test]
fn hyper_unsupported_usage_source_routes_to_auto() {
    for cookie_source in ["off", "manual"] {
        let ctx = hyper_context(Some(cookie_source), "cli", None);
        assert_eq!(ctx.source_mode, SourceMode::Auto, "{cookie_source}");
        assert!(ctx.manual_cookie_missing);
    }
}

#[test]
fn hyper_exposes_upstream_cookie_source_picker() {
    let settings = Settings::default();
    assert_eq!(
        super::provider_cookie_source_lookup(&settings, "hyper").as_deref(),
        Some("auto")
    );
    let options = super::cookie_source_options_for("hyper", Language::English);
    let values: Vec<_> = options.iter().map(|option| option.value.as_str()).collect();
    assert_eq!(values, vec!["auto", "manual", "off"]);
    let descriptions: Vec<_> = options
        .iter()
        .map(|option| option.description.as_deref())
        .collect();
    assert_eq!(
        descriptions,
        vec![
            Some("Prefer a signed-in Hyper browser session, then fall back to an API key."),
            Some("Paste a Cookie header from hyper.charm.land."),
            Some("Use only the configured API key."),
        ]
    );
}
