//! Retry back-off for forbidden Cursor cost requests (upstream 0.66.0
//! #3910 / #3918).
//!
//! Cursor answers HTTP 403 on `get-filtered-usage-events` for geo and
//! permission blocks that do not affect the quota endpoints. Retrying that
//! request on every refresh only adds a failing 20 s round trip, so one 403
//! parks the request for [`FORBIDDEN_COST_COOLDOWN`] per credential.
//!
//! The state is process-wide and in memory only, because the shell builds a
//! fresh provider for every refresh. Keys are SHA-256 fingerprints of the
//! Cookie header, so raw cookies are never stored and a new cookie or account
//! retries immediately. A restart clears everything.
//!
//! Manual-refresh bypass (upstream: manual and menu-open refreshes skip the
//! cooldown) is deferred: `FetchContext` carries no force signal, and adding
//! one is cross-provider plumbing.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// Upstream `UsageStore.tokenFetchFailureRetryDelay`: `max(cadence, 6 h)`.
/// The slowest refresh interval Win-CodexBar offers is 30 minutes, so the
/// six-hour floor is always the binding term.
pub(super) const FORBIDDEN_COST_COOLDOWN: Duration = Duration::from_secs(6 * 60 * 60);

/// Bounds the map if a user cycles through many credentials in one session.
const MAX_TRACKED_CREDENTIALS: usize = 32;

#[derive(Debug, Default)]
pub(super) struct CostCooldown {
    retry_after: Mutex<HashMap<String, Instant>>,
}

impl CostCooldown {
    /// The process-wide store shared by every `CursorProvider` instance.
    pub(super) fn shared() -> Arc<Self> {
        static SHARED: LazyLock<Arc<CostCooldown>> =
            LazyLock::new(|| Arc::new(CostCooldown::default()));
        Arc::clone(&SHARED)
    }

    pub(super) fn is_cooling_down(&self, credential: &str, now: Instant) -> bool {
        self.lock()
            .get(credential)
            .is_some_and(|retry_after| now < *retry_after)
    }

    pub(super) fn record_forbidden(&self, credential: &str, now: Instant) {
        let mut entries = self.lock();
        entries.retain(|_, retry_after| now < *retry_after);
        if !entries.contains_key(credential)
            && entries.len() >= MAX_TRACKED_CREDENTIALS
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, retry_after)| **retry_after)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(credential.to_string(), now + FORBIDDEN_COST_COOLDOWN);
    }

    /// A successful events request proves the block is gone.
    pub(super) fn clear(&self, credential: &str) {
        self.lock().remove(credential);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        // The map holds plain timestamps, so a poisoned lock is still valid.
        self.retry_after
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Stable per-credential key that never contains the cookie itself.
pub(super) fn credential_fingerprint(cookie_header: &str) -> String {
    let digest = Sha256::digest(cookie_header.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cools_down_for_six_hours_then_expires() {
        let cooldown = CostCooldown::default();
        let start = Instant::now();
        cooldown.record_forbidden("a", start);

        assert!(cooldown.is_cooling_down("a", start));
        assert!(cooldown.is_cooling_down(
            "a",
            start + FORBIDDEN_COST_COOLDOWN - Duration::from_secs(1)
        ));
        assert!(!cooldown.is_cooling_down("a", start + FORBIDDEN_COST_COOLDOWN));
    }

    #[test]
    fn other_credentials_are_not_cooled_down() {
        let cooldown = CostCooldown::default();
        let start = Instant::now();
        cooldown.record_forbidden("a", start);

        assert!(!cooldown.is_cooling_down("b", start));
    }

    #[test]
    fn clear_lifts_the_cooldown() {
        let cooldown = CostCooldown::default();
        let start = Instant::now();
        cooldown.record_forbidden("a", start);
        cooldown.clear("a");

        assert!(!cooldown.is_cooling_down("a", start));
    }

    #[test]
    fn refreshing_an_existing_cooldown_does_not_evict_another_credential() {
        let cooldown = CostCooldown::default();
        let start = Instant::now();
        cooldown.record_forbidden("oldest", start);
        for index in 0..(MAX_TRACKED_CREDENTIALS - 1) {
            cooldown.record_forbidden(
                &format!("credential-{index}"),
                start + Duration::from_secs(index as u64 + 1),
            );
        }

        // A concurrent fetch for this credential may have passed the initial
        // cooldown check before another request recorded its 403.
        let retry = start + Duration::from_secs(MAX_TRACKED_CREDENTIALS as u64);
        cooldown.record_forbidden("credential-0", retry);

        assert_eq!(cooldown.lock().len(), MAX_TRACKED_CREDENTIALS);
        assert!(cooldown.is_cooling_down("oldest", retry));
    }

    #[test]
    fn tracked_credentials_are_bounded_and_expired_entries_pruned() {
        let cooldown = CostCooldown::default();
        let start = Instant::now();
        for index in 0..(MAX_TRACKED_CREDENTIALS + 8) {
            cooldown.record_forbidden(
                &format!("credential-{index}"),
                start + Duration::from_secs(index as u64),
            );
        }
        assert_eq!(cooldown.lock().len(), MAX_TRACKED_CREDENTIALS);
        // The oldest deadlines were evicted, the newest kept.
        assert!(!cooldown.is_cooling_down("credential-0", start));
        assert!(cooldown.is_cooling_down(
            &format!("credential-{}", MAX_TRACKED_CREDENTIALS + 7),
            start + Duration::from_secs(MAX_TRACKED_CREDENTIALS as u64 + 7)
        ));

        cooldown.record_forbidden("late", start + FORBIDDEN_COST_COOLDOWN * 2);
        assert_eq!(cooldown.lock().len(), 1, "expired entries are pruned");
    }

    #[test]
    fn fingerprint_hides_the_cookie_and_separates_credentials() {
        let first = credential_fingerprint("WorkosCursorSessionToken=user%3A%3Asecret-one");
        let second = credential_fingerprint("WorkosCursorSessionToken=user%3A%3Asecret-two");

        assert_eq!(first.len(), 64);
        assert!(!first.contains("secret"));
        assert_ne!(first, second);
        assert_eq!(
            first,
            credential_fingerprint("WorkosCursorSessionToken=user%3A%3Asecret-one")
        );
    }
}
