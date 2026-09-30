//! Opt-in credential-expiry alerts (upstream 0.67.0 `handleCredentialOutcome`).
//!
//! One toast per (provider, account scope) failure episode. Only a typed
//! sign-in state ([`ProviderStateKind::needs_sign_in`]) starts an episode;
//! only a fresh successful fetch ends it. Cached or last-good fallbacks,
//! repeated failures and unrelated failures (network, timeout, parse, quota)
//! leave it untouched, so a flapping provider never re-alerts. Episodes live
//! in memory and start fresh when the app restarts.
//!
//! The toast names the provider only; it never carries the raw error, email
//! or account id.

use std::collections::HashSet;

use super::NotificationManager;
use crate::core::{ProviderId, ProviderStateKind};
use crate::locale::{self, LocaleKey};
use crate::settings::{Language, Settings};

/// Delivery settings captured at refresh time, so a refresh task does not
/// need the whole [`Settings`] value.
#[derive(Debug, Clone, Copy)]
pub struct CredentialAlertPolicy {
    enabled: bool,
    language: Language,
}

impl CredentialAlertPolicy {
    /// The master notification switch also silences credential alerts, like
    /// every other toast.
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            enabled: settings.show_notifications
                && settings.credential_expiry_notifications_enabled,
            language: settings.ui_language,
        }
    }
}

/// Open episodes keyed by provider and account scope. An empty scope means
/// no account evidence was available, so it acts as a wildcard for that
/// provider: an identity gap must neither hide a real recovery nor re-alert
/// on every refresh.
#[derive(Debug, Default)]
pub(super) struct CredentialEpisodes {
    open: HashSet<(ProviderId, String)>,
}

impl CredentialEpisodes {
    fn covers(&self, provider: ProviderId, account: &str) -> bool {
        if account.is_empty() {
            return self.open.iter().any(|(open, _)| *open == provider);
        }
        self.open.contains(&(provider, account.to_string()))
            || self.open.contains(&(provider, String::new()))
    }

    /// Reserve an episode; `false` when one already covers this account.
    fn begin(&mut self, provider: ProviderId, account: &str) -> bool {
        if self.covers(provider, account) {
            return false;
        }
        self.open.insert((provider, account.to_string()));
        true
    }

    fn release(&mut self, provider: ProviderId, account: &str) {
        self.open.remove(&(provider, account.to_string()));
    }

    fn end(&mut self, provider: ProviderId, account: &str) {
        if account.is_empty() {
            self.open.retain(|(open, _)| *open != provider);
        } else {
            self.open.remove(&(provider, account.to_string()));
            self.open.remove(&(provider, String::new()));
        }
    }
}

impl NotificationManager {
    /// Record a failed refresh for `account`. Posts one toast when `kind` is a
    /// sign-in state, alerts are enabled and no episode is open yet. Returns
    /// whether a toast was delivered.
    pub fn observe_credential_failure(
        &mut self,
        policy: CredentialAlertPolicy,
        provider: ProviderId,
        account: &str,
        kind: ProviderStateKind,
    ) -> bool {
        self.observe_credential_failure_with(policy, provider, account, kind, Self::show_toast)
    }

    fn observe_credential_failure_with(
        &mut self,
        policy: CredentialAlertPolicy,
        provider: ProviderId,
        account: &str,
        kind: ProviderStateKind,
        deliver: impl FnOnce(&Self, &str, &str) -> bool,
    ) -> bool {
        if !policy.enabled || !kind.needs_sign_in() {
            return false;
        }
        if !self.credential_episodes.begin(provider, account) {
            return false;
        }
        let title = locale::format_locale(
            policy.language,
            LocaleKey::CredentialExpiryTitle,
            &[provider.display_name()],
        );
        let body = locale::get_text(policy.language, LocaleKey::CredentialExpiryBody);
        let delivered = deliver(self, &title, &body);
        if !delivered {
            // A failed toast must not swallow the alert: the next refresh retries.
            self.credential_episodes.release(provider, account);
        }
        delivered
    }

    /// Record a fresh successful fetch (no error, not a last-good replay).
    /// Ends the episode even while alerts are disabled, so re-enabling them
    /// never replays a stale failure.
    pub fn observe_credential_recovery(&mut self, provider: ProviderId, account: &str) {
        self.credential_episodes.end(provider, account);
    }

    /// Forget episodes of providers that are no longer enabled.
    pub fn retire_credential_episodes_except(&mut self, enabled: &[ProviderId]) {
        self.credential_episodes
            .open
            .retain(|(provider, _)| enabled.contains(provider));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ProviderError;
    use std::cell::RefCell;

    const ACCOUNT: &str = "person@example.test";

    fn policy(enabled: bool) -> CredentialAlertPolicy {
        CredentialAlertPolicy {
            enabled,
            language: Language::English,
        }
    }

    /// Records delivered toasts; `succeed` scripts the delivery result.
    struct Toasts {
        sent: RefCell<Vec<(String, String)>>,
    }

    impl Toasts {
        fn new() -> Self {
            Self {
                sent: RefCell::new(Vec::new()),
            }
        }

        fn fail(
            &self,
            manager: &mut NotificationManager,
            provider: ProviderId,
            account: &str,
            kind: ProviderStateKind,
            succeed: bool,
        ) -> bool {
            manager.observe_credential_failure_with(
                policy(true),
                provider,
                account,
                kind,
                |_, title, body| {
                    self.sent
                        .borrow_mut()
                        .push((title.to_string(), body.to_string()));
                    succeed
                },
            )
        }

        fn count(&self) -> usize {
            self.sent.borrow().len()
        }
    }

    fn network_error() -> ProviderError {
        ProviderError::Network(
            reqwest::Client::new()
                .get("file:///private.example.test/unreachable")
                .build()
                .unwrap_err(),
        )
    }

    #[test]
    fn each_provider_error_variant_alerts_only_for_sign_in_states() {
        use ProviderError as E;
        let cases = [
            (E::NotInstalled("key missing".into()), true),
            (E::AuthRequired, true),
            (E::NoCookies, true),
            (E::OAuth("credentials not found".into()), true),
            (E::OAuthRevoked("revoked".into()), true),
            (E::OAuthExpired("expired".into()), true),
            (E::OAuthTransient("try again".into()), false),
            (E::Parse("bad body".into()), false),
            (network_error(), false),
            (E::Timeout, false),
            (E::UnsupportedSource(crate::core::SourceMode::Auto), false),
            (E::Other("API error 500".into()), false),
            (E::Other("quota exhausted".into()), false),
            (E::Other("403 permission denied".into()), false),
            (E::Other("429 rate limited".into()), false),
        ];
        for (error, expect_alert) in cases {
            let mut manager = NotificationManager::new();
            let toasts = Toasts::new();
            let alerted = toasts.fail(
                &mut manager,
                ProviderId::Claude,
                ACCOUNT,
                error.state_kind(),
                true,
            );
            assert_eq!(alerted, expect_alert, "{error}");
            assert_eq!(toasts.count(), usize::from(expect_alert), "{error}");
        }
    }

    #[test]
    fn provider_specific_offline_state_does_not_alert() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        assert!(!toasts.fail(
            &mut manager,
            ProviderId::Antigravity,
            "",
            ProviderStateKind::LocalRuntimeOffline,
            true,
        ));
        assert_eq!(toasts.count(), 0);
    }

    #[test]
    fn toast_names_the_provider_and_nothing_else() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        assert!(toasts.fail(
            &mut manager,
            ProviderId::Claude,
            ACCOUNT,
            ProviderStateKind::ExpiredSession,
            true,
        ));
        let sent = toasts.sent.borrow();
        assert_eq!(sent[0].0, "Claude needs sign-in");
        assert_eq!(
            sent[0].1,
            "Open CodexBar to review the account error and sign in again."
        );
        assert!(!sent[0].0.contains(ACCOUNT) && !sent[0].1.contains(ACCOUNT));
    }

    #[test]
    fn episode_ends_only_on_fresh_success() {
        // fail, fail, timeout, cached success (no recovery call), fresh
        // success, fail => two toasts in total.
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;
        let p = ProviderId::Codex;

        assert!(toasts.fail(&mut manager, p, ACCOUNT, auth, true));
        assert!(!toasts.fail(&mut manager, p, ACCOUNT, auth, true));
        assert!(!toasts.fail(&mut manager, p, ACCOUNT, ProviderStateKind::Unknown, true));
        // A cached / last-good replay is never reported as a recovery, so the
        // next sign-in failure is still the same episode.
        assert!(!toasts.fail(&mut manager, p, ACCOUNT, auth, true));

        manager.observe_credential_recovery(p, ACCOUNT);
        assert!(toasts.fail(&mut manager, p, ACCOUNT, auth, true));
        assert_eq!(toasts.count(), 2);
    }

    #[test]
    fn failed_toast_releases_the_reservation_for_retry() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;

        assert!(!toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, false));
        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        assert!(!toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        assert_eq!(toasts.count(), 2);
    }

    #[test]
    fn accounts_and_providers_have_independent_episodes() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;

        assert!(toasts.fail(
            &mut manager,
            ProviderId::Codex,
            "a@example.test",
            auth,
            true
        ));
        assert!(toasts.fail(
            &mut manager,
            ProviderId::Codex,
            "b@example.test",
            auth,
            true
        ));
        assert!(toasts.fail(
            &mut manager,
            ProviderId::Claude,
            "a@example.test",
            auth,
            true
        ));

        manager.observe_credential_recovery(ProviderId::Codex, "a@example.test");
        assert!(toasts.fail(
            &mut manager,
            ProviderId::Codex,
            "a@example.test",
            auth,
            true
        ));
        assert!(!toasts.fail(
            &mut manager,
            ProviderId::Codex,
            "b@example.test",
            auth,
            true
        ));
    }

    #[test]
    fn identity_gap_neither_realerts_nor_hides_recovery() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;
        let p = ProviderId::Codex;

        // No identity yet, then the same account surfaces without evidence.
        assert!(toasts.fail(&mut manager, p, "", auth, true));
        assert!(!toasts.fail(&mut manager, p, ACCOUNT, auth, true));
        // Failing with a known identity is covered by that episode too.
        manager.observe_credential_recovery(p, ACCOUNT);
        assert!(toasts.fail(&mut manager, p, "", auth, true));
        // A success with no identity ends every episode of the provider.
        manager.observe_credential_recovery(p, "");
        assert!(toasts.fail(&mut manager, p, ACCOUNT, auth, true));
        assert_eq!(toasts.count(), 3);
    }

    #[test]
    fn disabled_alerts_deliver_nothing_but_keep_open_episodes() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;
        let p = ProviderId::Codex;

        assert!(toasts.fail(&mut manager, p, ACCOUNT, auth, true));

        let disabled = manager.observe_credential_failure_with(
            policy(false),
            p,
            "other@example.test",
            auth,
            |_, _, _| panic!("no delivery while disabled"),
        );
        assert!(!disabled);

        // Off then on: the unresolved episode is remembered, so no replay...
        assert!(!toasts.fail(&mut manager, p, ACCOUNT, auth, true));
        // ...and the account that failed while disabled alerts once enabled.
        assert!(toasts.fail(&mut manager, p, "other@example.test", auth, true));
        assert_eq!(toasts.count(), 2);
    }

    #[test]
    fn recovery_while_disabled_still_ends_the_episode() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::ExpiredSession;

        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        manager.observe_credential_recovery(ProviderId::Codex, ACCOUNT);
        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
    }

    #[test]
    fn disabled_providers_retire_their_episodes() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;

        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        assert!(toasts.fail(&mut manager, ProviderId::Claude, ACCOUNT, auth, true));

        manager.retire_credential_episodes_except(&[ProviderId::Claude]);

        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        assert!(!toasts.fail(&mut manager, ProviderId::Claude, ACCOUNT, auth, true));
    }

    #[test]
    fn no_enabled_providers_retires_all_episodes() {
        let mut manager = NotificationManager::new();
        let toasts = Toasts::new();
        let auth = ProviderStateKind::NeedsAuthentication;

        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        manager.retire_credential_episodes_except(&[]);
        assert!(toasts.fail(&mut manager, ProviderId::Codex, ACCOUNT, auth, true));
        assert_eq!(toasts.count(), 2);
    }

    #[test]
    fn policy_requires_master_switch_and_credential_toggle() {
        let mut settings = Settings {
            show_notifications: true,
            credential_expiry_notifications_enabled: true,
            ..Settings::default()
        };
        assert!(CredentialAlertPolicy::from_settings(&settings).enabled);
        settings.show_notifications = false;
        assert!(!CredentialAlertPolicy::from_settings(&settings).enabled);
        settings.show_notifications = true;
        settings.credential_expiry_notifications_enabled = false;
        assert!(!CredentialAlertPolicy::from_settings(&settings).enabled);
    }
}
