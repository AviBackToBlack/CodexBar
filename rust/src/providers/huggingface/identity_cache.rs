use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use super::IdentitySnapshot;

const IDENTITY_TTL: Duration = Duration::seconds(12 * 60 * 60);
const MAX_IDENTITY_ENTRIES: usize = 32;

#[derive(Debug, Clone)]
struct CachedIdentity {
    identity: IdentitySnapshot,
    fetched_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub(super) struct IdentityCache {
    entries: HashMap<String, CachedIdentity>,
}

impl IdentityCache {
    pub(super) fn get(&mut self, token: &str, now: DateTime<Utc>) -> Option<IdentitySnapshot> {
        let key = token_key(token);
        let fresh = self
            .entries
            .get(&key)
            .is_some_and(|entry| is_fresh(entry, &now));
        if !fresh {
            drop(self.entries.remove(&key));
            return None;
        }
        self.entries.get(&key).map(|entry| entry.identity.clone())
    }

    pub(super) fn insert(&mut self, token: &str, identity: IdentitySnapshot, now: DateTime<Utc>) {
        self.remove_expired(&now);
        let key = token_key(token);
        drop(self.entries.remove(&key));
        if self.entries.len() >= MAX_IDENTITY_ENTRIES {
            let oldest_key = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(key, _)| key.clone());
            if let Some(oldest_key) = oldest_key {
                drop(self.entries.remove(&oldest_key));
            }
        }
        self.entries.insert(
            key,
            CachedIdentity {
                identity,
                fetched_at: now,
            },
        );
    }

    fn remove_expired(&mut self, now: &DateTime<Utc>) {
        self.entries.retain(|_, entry| is_fresh(entry, now));
    }
}

fn is_fresh(entry: &CachedIdentity, now: &DateTime<Utc>) -> bool {
    let age = now.signed_duration_since(entry.fetched_at);
    age >= Duration::zero() && age < IDENTITY_TTL
}

fn token_key(token: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(token.as_bytes());
    let mut key = String::with_capacity(64);
    for byte in digest {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0x0f) as usize] as char);
    }
    key
}

static PROCESS_IDENTITY_CACHE: OnceLock<Mutex<IdentityCache>> = OnceLock::new();

pub(super) fn process_identity_cache() -> &'static Mutex<IdentityCache> {
    PROCESS_IDENTITY_CACHE.get_or_init(|| Mutex::new(IdentityCache::default()))
}

pub(super) async fn get_or_fetch_identity<F, Fut>(
    cache: &Mutex<IdentityCache>,
    token: &str,
    now: DateTime<Utc>,
    fetch: F,
) -> Option<IdentitySnapshot>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<IdentitySnapshot>>,
{
    if let Some(identity) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(token, now)
    {
        return Some(identity);
    }

    let identity = fetch().await?;
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(token, identity.clone(), now);
    Some(identity)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    fn identity(name: &str) -> IdentitySnapshot {
        IdentitySnapshot {
            user_id: None,
            name: Some(name.to_string()),
            email: None,
            plan: None,
        }
    }

    #[tokio::test]
    async fn identity_cache_is_isolated_per_token_and_expires_with_fetch_clock() {
        let cache = Mutex::new(IdentityCache::default());
        let calls = AtomicUsize::new(0);
        let t = at(1_800_000_000);

        let first = get_or_fetch_identity(&cache, "fixture-a", t, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(identity("fixture-a"))
        })
        .await
        .unwrap();
        let second = get_or_fetch_identity(&cache, "fixture-b", t, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(identity("fixture-b"))
        })
        .await
        .unwrap();
        let cached =
            get_or_fetch_identity(&cache, "fixture-a", t + Duration::seconds(60), || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Some(identity("unexpected"))
            })
            .await
            .unwrap();
        let refreshed =
            get_or_fetch_identity(&cache, "fixture-a", t + Duration::hours(13), || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Some(identity("fixture-a-refreshed"))
            })
            .await
            .unwrap();

        assert_eq!(first.name.as_deref(), Some("fixture-a"));
        assert_eq!(second.name.as_deref(), Some("fixture-b"));
        assert_eq!(cached, first);
        assert_eq!(refreshed.name.as_deref(), Some("fixture-a-refreshed"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let cache = cache.lock().unwrap();
        assert!(cache.entries.keys().all(|key| !key.contains("fixture-a")));
        assert!(cache.entries.keys().all(|key| !key.contains("fixture-b")));
        assert!(cache.entries.keys().all(|key| key.len() == 64));
    }

    #[test]
    fn identity_cache_expires_at_twelve_hours_and_when_clock_moves_backwards() {
        let mut cache = IdentityCache::default();
        let t = at(1_800_000_000);
        cache.insert("fixture", identity("cached"), t);
        assert_eq!(cache.get("fixture", t + Duration::hours(12)), None);

        cache.insert("fixture", identity("cached"), t);
        assert_eq!(cache.get("fixture", t - Duration::seconds(1)), None);
        assert!(cache.entries.is_empty());
    }

    #[tokio::test]
    async fn empty_identity_is_not_cached_but_name_only_identity_is() {
        let cache = Mutex::new(IdentityCache::default());
        let calls = AtomicUsize::new(0);
        let t = at(1_800_000_000);
        cache
            .lock()
            .unwrap()
            .insert("fixture", identity("stale"), t - IDENTITY_TTL);

        assert_eq!(
            get_or_fetch_identity(&cache, "fixture", t, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                None
            })
            .await,
            None
        );
        let named = get_or_fetch_identity(&cache, "fixture", t, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(identity("only-name"))
        })
        .await
        .unwrap();
        let cached = get_or_fetch_identity(&cache, "fixture", t + Duration::seconds(1), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(identity("unexpected"))
        })
        .await
        .unwrap();

        assert_eq!(named.name.as_deref(), Some("only-name"));
        assert_eq!(cached, named);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cache_evicts_old_entries_at_its_size_limit() {
        let mut cache = IdentityCache::default();
        let t = at(1_800_000_000);
        for index in 0..=MAX_IDENTITY_ENTRIES {
            cache.insert(
                &format!("token-{index}"),
                identity("name"),
                t + Duration::seconds(i64::try_from(index).unwrap()),
            );
        }
        assert_eq!(cache.entries.len(), MAX_IDENTITY_ENTRIES);
        assert_eq!(cache.get("token-0", t + Duration::seconds(40)), None);
        assert!(cache.get("token-32", t + Duration::seconds(40)).is_some());
    }
}
