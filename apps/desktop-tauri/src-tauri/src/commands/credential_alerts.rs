//! Bridges provider refresh outcomes to the core credential-expiry episode
//! tracker (`codexbar::notifications`). Classification stays typed: the
//! failure kind is captured from the fresh fetch before any last-good
//! fallback replaces the published snapshot, so cached data can neither end
//! nor restart an episode.

use codexbar::core::{ProviderId, ProviderStateKind};
use codexbar::notifications::{CredentialAlertPolicy, NotificationManager};

use super::bridge::ProviderUsageSnapshot;
use super::providers::quota_notification_account_identity;

/// Result of the fresh fetch, before last-good preservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FetchAttempt {
    Succeeded,
    Failed(ProviderStateKind),
}

impl FetchAttempt {
    pub(super) fn of(fresh: &ProviderUsageSnapshot) -> Self {
        if fresh.error.is_none() {
            Self::Succeeded
        } else {
            Self::Failed(fresh.error_state)
        }
    }
}

/// Report one accepted refresh outcome. `published` is the snapshot that
/// replaced the cache; `last_good` is the previous error-free snapshot, the
/// only source of account identity when the fetch itself failed.
pub(super) fn observe_attempt(
    manager: &mut NotificationManager,
    policy: Option<CredentialAlertPolicy>,
    id: ProviderId,
    token_account_id: Option<uuid::Uuid>,
    attempt: FetchAttempt,
    published: &ProviderUsageSnapshot,
    last_good: Option<&ProviderUsageSnapshot>,
) {
    match attempt {
        FetchAttempt::Succeeded => {
            let account = quota_notification_account_identity(published, token_account_id);
            manager.observe_credential_recovery(id, &account);
        }
        FetchAttempt::Failed(kind) => {
            if let Some(policy) = policy {
                let account = quota_notification_account_identity(
                    last_good.unwrap_or(published),
                    token_account_id,
                );
                manager.observe_credential_failure(policy, id, &account, kind);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codexbar::core::{ProviderError, instantiate_provider};
    use codexbar::settings::Settings;

    fn failed(kind: ProviderStateKind) -> ProviderUsageSnapshot {
        ProviderUsageSnapshot::from_error(
            ProviderId::Codex,
            instantiate_provider(ProviderId::Codex).metadata(),
            "Authentication required".to_string(),
            kind,
        )
    }

    fn succeeded() -> ProviderUsageSnapshot {
        let mut snapshot = failed(ProviderStateKind::Ready);
        snapshot.error = None;
        snapshot.account_email = Some("person@example.test".to_string());
        snapshot
    }

    #[test]
    fn attempt_reads_the_fresh_error_state() {
        assert_eq!(
            FetchAttempt::of(&failed(ProviderStateKind::ExpiredSession)),
            FetchAttempt::Failed(ProviderStateKind::ExpiredSession)
        );
        assert_eq!(FetchAttempt::of(&succeeded()), FetchAttempt::Succeeded);
    }

    #[test]
    fn typed_state_matches_provider_error_classification() {
        let kind = ProviderError::AuthRequired.state_kind();
        assert_eq!(
            FetchAttempt::of(&failed(kind)),
            FetchAttempt::Failed(ProviderStateKind::NeedsAuthentication)
        );
    }

    #[test]
    fn observing_outcomes_with_alerts_off_never_delivers() {
        // Default settings leave credential alerts off, so no toast can fire.
        let policy = CredentialAlertPolicy::from_settings(&Settings::default());
        let mut manager = NotificationManager::new();
        let good = succeeded();
        let failure = failed(ProviderStateKind::NeedsAuthentication);

        observe_attempt(
            &mut manager,
            Some(policy),
            ProviderId::Codex,
            None,
            FetchAttempt::of(&failure),
            &failure,
            Some(&good),
        );
        observe_attempt(
            &mut manager,
            Some(policy),
            ProviderId::Codex,
            None,
            FetchAttempt::Succeeded,
            &good,
            None,
        );
    }
}
