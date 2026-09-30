//! Import DeepSeek Platform sessions from Chrome's local storage.
//!
//! The platform dashboard keeps the signed-in session as a `userToken` item in
//! `localStorage` for `https://platform.deepseek.com`. Each Chrome profile holds
//! its own copy, so a candidate is identified by `chrome:<profile directory>`.

use std::fmt;

use serde_json::Value;

use crate::browser::detection::{BrowserDetector, BrowserType};
use crate::browser::leveldb::local_storage::{local_storage_dir, read_local_storage_entries};

const PLATFORM_ORIGIN: &str = "https://platform.deepseek.com";
const TOKEN_KEY: &str = "userToken";
const TOKEN_FIELDS: [&str; 5] = ["value", "token", "access_token", "accessToken", "userToken"];
const MIN_TOKEN_CHARS: usize = 20;

/// Environment variable that pins the Chrome profile to read when several
/// profiles hold a DeepSeek session. It accepts `chrome:<directory>` or a
/// profile directory path.
pub(super) const PROFILE_ID_ENV: &str = "CODEXBAR_DEEPSEEK_PROFILE_ID";

/// A session token found in one Chrome profile.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct TokenInfo {
    pub(super) id: String,
    pub(super) token: String,
    pub(super) label: String,
}

impl fmt::Debug for TokenInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenInfo")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .field("label", &self.label)
            .finish()
    }
}

/// Which profile the user asked for, if any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ProfileSelection {
    pub(super) profile_id: Option<String>,
}

impl ProfileSelection {
    pub(super) fn from_env() -> Self {
        Self::from_value(std::env::var(PROFILE_ID_ENV).ok().as_deref())
    }

    pub(super) fn from_value(value: Option<&str>) -> Self {
        Self {
            profile_id: value
                .map(canonical_profile_id)
                .filter(|profile| !profile.is_empty()),
        }
    }
}

/// Normalize a profile setting: a directory path becomes `chrome:<leaf>`.
pub(super) fn canonical_profile_id(raw: &str) -> String {
    let value = raw.trim();
    let is_path = value.starts_with('/') || value.contains('\\') || value.contains(":/");
    if !is_path {
        return value.to_string();
    }
    let leaf = value
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    if leaf.is_empty() {
        value.to_string()
    } else {
        format!("chrome:{leaf}")
    }
}

/// Read the DeepSeek session token of every Chrome profile that has one.
///
/// Blocking: reads LevelDB files from disk. Never logs token values.
pub(super) fn import_tokens() -> Vec<TokenInfo> {
    let Some(chrome) = BrowserDetector::detect(BrowserType::Chrome) else {
        tracing::debug!("deepseek chrome session: Chrome profiles not found");
        return Vec::new();
    };

    let mut tokens = Vec::new();
    for profile in &chrome.profiles {
        let id = format!("chrome:{}", profile.name);
        let dir = local_storage_dir(&profile.path);
        let entries = match read_local_storage_entries(&dir, PLATFORM_ORIGIN) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::debug!(profile = %id, %error, "deepseek chrome session: local storage unreadable");
                continue;
            }
        };
        let Some(token) = entries
            .iter()
            .find(|entry| entry.key == TOKEN_KEY)
            .and_then(|entry| extract_user_token(&entry.value))
        else {
            tracing::debug!(profile = %id, "deepseek chrome session: no userToken");
            continue;
        };
        tracing::debug!(profile = %id, "deepseek chrome session: found userToken");
        tokens.push(TokenInfo {
            label: format!("Google Chrome {}", profile.name),
            id,
            token,
        });
    }
    tokens.sort_by(|a, b| a.id.cmp(&b.id));
    tokens
}

/// Extract the token from a `userToken` value: a JSON object carrying the
/// token in one of the known fields, or a bare (optionally quoted) token.
pub(super) fn extract_user_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str::<Value>(trimmed)
    {
        return token_from_json(&value);
    }

    let unquoted = if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed
            .get(1..trimmed.len().saturating_sub(1))
            .unwrap_or_default()
    } else {
        trimmed
    };
    is_plausible_token(unquoted).then(|| unquoted.to_string())
}

fn token_from_json(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    TOKEN_FIELDS.iter().find_map(|field| {
        let token = object.get(*field)?.as_str()?;
        is_plausible_token(token).then(|| token.to_string())
    })
}

fn is_plausible_token(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.chars().count() >= MIN_TOKEN_CHARS && !trimmed.chars().any(char::is_whitespace)
}

#[cfg(test)]
#[path = "chrome_session_tests.rs"]
mod tests;
