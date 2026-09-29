//! Kimi web access tokens from Chromium local storage (upstream #3923).
//!
//! Upstream `KimiCookieImporter.localStorageTokens(region:)` reads `access_token` from each
//! Chromium profile's `Local Storage/leveldb` for the selected region's web origin and keeps only
//! current three-segment ASCII JWTs. It never reads or refreshes refresh tokens, and it runs after
//! cookie discovery, so a manual credential or a cookie session always takes precedence.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::KimiRegion;
use crate::browser::detection::BrowserDetector;
use crate::browser::leveldb::local_storage::{
    LocalStorageEntry, local_storage_dir, read_local_storage_entries,
};
use crate::codex_accounts::api::jwt_payload;

const ACCESS_TOKEN_KEY: &str = "access_token";

/// Current Kimi web access tokens stored by Chromium browsers for `region`, in browser and
/// profile detection order, without duplicates.
pub(super) fn local_storage_tokens(region: KimiRegion) -> Vec<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    tokens_from_profiles(&chromium_profile_dirs(), region.web_base_url(), now)
}

fn chromium_profile_dirs() -> Vec<PathBuf> {
    BrowserDetector::detect_all()
        .into_iter()
        .filter(|browser| browser.browser_type.is_chromium_based())
        .flat_map(|browser| browser.profiles)
        .map(|profile| profile.path)
        .collect()
}

fn tokens_from_profiles(profiles: &[PathBuf], origin: &str, now_unix: f64) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tokens = Vec::new();
    for profile in profiles {
        let dir = local_storage_dir(profile);
        if !dir.is_dir() {
            continue;
        }
        match read_local_storage_entries(&dir, origin) {
            Ok(entries) => {
                for token in access_tokens(&entries, now_unix) {
                    if seen.insert(token.clone()) {
                        tokens.push(token);
                    }
                }
            }
            Err(error) => tracing::debug!(%error, "Kimi local storage is not readable"),
        }
    }
    tokens
}

fn access_tokens(entries: &[LocalStorageEntry], now_unix: f64) -> impl Iterator<Item = String> {
    entries
        .iter()
        .filter(|entry| entry.key == ACCESS_TOKEN_KEY)
        .map(|entry| normalized_value(&entry.value))
        .filter(move |token| is_current_jwt(token, now_unix))
}

/// Web storage may hold the token as a JSON string (`"eyJ..."`) or as bare text.
fn normalized_value(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.trim().to_string())
}

fn is_current_jwt(token: &str, now_unix: f64) -> bool {
    token.split('.').count() == 3
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && jwt_payload(token)
            .and_then(|payload| payload.get("exp").and_then(serde_json::Value::as_f64))
            .is_some_and(|exp| exp.is_finite() && exp > now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000.0;
    // Fixture from upstream `KimiLocalStorageTests`: payload {"exp":1800003600}.
    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjE4MDAwMDM2MDB9.signature";
    // Payload {"exp":1}.
    const EXPIRED: &str = "eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjF9.signature";

    fn entry(key: &str, value: &str) -> LocalStorageEntry {
        LocalStorageEntry {
            key: key.into(),
            value: value.into(),
        }
    }

    fn tokens(entries: &[LocalStorageEntry], now: f64) -> Vec<String> {
        access_tokens(entries, now).collect()
    }

    #[test]
    fn only_current_access_tokens_are_kept() {
        let entries = [
            entry("refresh_token", TOKEN),
            entry("access_token", EXPIRED),
            entry("access_token", "not-a-token"),
            entry("access_token", "a.b.c"),
            entry("access_token", &format!("{TOKEN}; kimi-auth=x")),
            entry("access_token", &format!(" {TOKEN}\n")),
            entry("access_token", &format!("\"{TOKEN}\"")),
        ];

        assert_eq!(tokens(&entries, NOW), vec![TOKEN, TOKEN]);
        assert!(tokens(&entries, NOW + 3600.0).is_empty());
    }

    #[test]
    fn non_finite_or_missing_expiry_is_rejected() {
        // Payloads {"exp":"soon"} and {"sub":"x"}.
        for payload in ["eyJleHAiOiJzb29uIn0", "eyJzdWIiOiJ4In0"] {
            let token = format!("eyJhbGciOiJIUzI1NiJ9.{payload}.signature");
            assert!(tokens(&[entry("access_token", &token)], NOW).is_empty());
        }
    }

    /// Build a one-record LevelDB write-ahead log holding one Local Storage `access_token`.
    fn write_log(origin: &str, value: &str) -> Vec<u8> {
        let mut key = format!("_{origin}\0").into_bytes();
        key.push(1);
        key.extend_from_slice(ACCESS_TOKEN_KEY.as_bytes());
        let mut value_bytes = vec![1];
        value_bytes.extend_from_slice(value.as_bytes());

        let mut batch = 1u64.to_le_bytes().to_vec();
        batch.extend_from_slice(&1u32.to_le_bytes());
        batch.push(1);
        batch.push(u8::try_from(key.len()).unwrap());
        batch.extend_from_slice(&key);
        batch.push(u8::try_from(value_bytes.len()).unwrap());
        batch.extend_from_slice(&value_bytes);

        let mut record = vec![0, 0, 0, 0];
        record.extend_from_slice(&u16::try_from(batch.len()).unwrap().to_le_bytes());
        record.push(1);
        record.extend_from_slice(&batch);
        record
    }

    fn profile_with(origin: &str, value: &str) -> tempfile::TempDir {
        let profile = tempfile::tempdir().unwrap();
        let dir = local_storage_dir(profile.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("000003.log"), write_log(origin, value)).unwrap();
        profile
    }

    #[test]
    fn profiles_are_read_for_the_selected_region_only_and_deduplicated() {
        let first = profile_with(KimiRegion::International.web_base_url(), TOKEN);
        let second = profile_with(KimiRegion::International.web_base_url(), TOKEN);
        let china = profile_with(KimiRegion::China.web_base_url(), TOKEN);
        let empty = tempfile::tempdir().unwrap();
        let profiles = [
            empty.path().to_path_buf(),
            first.path().to_path_buf(),
            second.path().to_path_buf(),
            china.path().to_path_buf(),
        ];

        assert_eq!(
            tokens_from_profiles(&profiles, KimiRegion::International.web_base_url(), NOW),
            vec![TOKEN]
        );
        assert_eq!(
            tokens_from_profiles(&profiles, KimiRegion::China.web_base_url(), NOW),
            vec![TOKEN]
        );
        assert!(
            tokens_from_profiles(&profiles[..3], KimiRegion::China.web_base_url(), NOW).is_empty()
        );
    }
}
