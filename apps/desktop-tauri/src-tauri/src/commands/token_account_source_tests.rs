//! Source-mode resolution for selected token accounts.

use std::collections::HashMap;

use codexbar::core::{
    ProviderAccountData, ProviderId, SourceMode, TokenAccount, instantiate_provider,
};
use codexbar::settings::{ApiKeys, ManualCookies, Settings};

fn token_accounts(id: ProviderId, token: &str) -> HashMap<ProviderId, ProviderAccountData> {
    let mut data = ProviderAccountData::new();
    data.add_account(TokenAccount::new("Work", token));
    HashMap::from([(id, data)])
}

fn context_with_usage_source(
    id: ProviderId,
    usage_source: &str,
    accounts: &HashMap<ProviderId, ProviderAccountData>,
) -> codexbar::core::FetchContext {
    let mut settings = Settings::default();
    settings.set_usage_source(id, usage_source);
    super::build_fetch_context(
        id,
        &settings,
        &ManualCookies::default(),
        &ApiKeys::default(),
        accounts,
    )
}

#[test]
fn huggingface_token_account_keeps_auto_so_the_wallet_is_still_read() {
    let accounts = token_accounts(ProviderId::HuggingFace, "hf_account_token");
    let ctx = context_with_usage_source(ProviderId::HuggingFace, "auto", &accounts);

    assert_eq!(ctx.source_mode, SourceMode::Auto);
    assert_eq!(ctx.api_key.as_deref(), Some("hf_account_token"));
}

#[test]
fn huggingface_token_account_with_explicit_api_source_stays_api_only() {
    let accounts = token_accounts(ProviderId::HuggingFace, "hf_account_token");
    let ctx = context_with_usage_source(ProviderId::HuggingFace, "oauth", &accounts);

    assert_eq!(ctx.source_mode, SourceMode::OAuth);
    assert_eq!(ctx.api_key.as_deref(), Some("hf_account_token"));
}

#[test]
fn huggingface_token_account_with_an_unsupported_stored_source_falls_back_to_api() {
    let accounts = token_accounts(ProviderId::HuggingFace, "hf_account_token");
    for stale in ["web", "cli"] {
        let ctx = context_with_usage_source(ProviderId::HuggingFace, stale, &accounts);
        assert_eq!(ctx.source_mode, SourceMode::OAuth, "{stale}");
    }
}

#[test]
fn huggingface_without_a_token_account_follows_the_usage_source() {
    let ctx = context_with_usage_source(ProviderId::HuggingFace, "auto", &HashMap::new());
    assert_eq!(ctx.source_mode, SourceMode::Auto);
}

#[test]
fn only_huggingface_keeps_auto_for_token_accounts() {
    for id in ProviderId::all() {
        assert_eq!(
            instantiate_provider(*id).token_account_preserves_auto_source(),
            *id == ProviderId::HuggingFace,
            "{id:?}"
        );
    }
}
