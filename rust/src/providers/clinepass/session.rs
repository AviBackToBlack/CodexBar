//! Read-only discovery of an existing Cline session (upstream 0.68.0).
//!
//! `cline auth` stores its credentials in `providers.json`. When no explicit
//! API key exists, ClinePass reads that file to reuse the session. The token is
//! only ever held in memory for the request: it is never written, refreshed or
//! copied into CodexBar configuration.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

const OVERRIDE_FILE_ENV: &str = "CLINE_PROVIDER_SETTINGS_PATH";
const OVERRIDE_DATA_DIR_ENV: &str = "CLINE_DATA_DIR";
const OVERRIDE_DIR_ENV: &str = "CLINE_DIR";
const HOME_ENV: &str = "HOME";
const OAUTH_PREFIX: &str = "workos:";
/// Cline's settings file is a few KiB; refuse anything absurdly large.
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// A credential found in Cline's settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileCredential {
    pub(super) token: String,
    /// True when the token came from a browser (`cline auth`) session.
    pub(super) is_oauth: bool,
}

impl FileCredential {
    pub(super) fn login_method(&self) -> &'static str {
        if self.is_oauth { "Browser" } else { "API key" }
    }
}

/// Read the Cline session file from the process environment. A missing or
/// unreadable file simply means "no credential".
pub(super) fn read_file_credential() -> Option<FileCredential> {
    let env = |key: &str| std::env::var(key).ok();
    let path = providers_file_path(&env, dirs::home_dir())?;
    read_credential_at(&path)
}

pub(super) fn read_credential_at(path: &Path) -> Option<FileCredential> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return None;
    }
    parse_credential(&bytes)
}

fn cleaned(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Resolve the settings file path following Cline's documented overrides:
/// `CLINE_PROVIDER_SETTINGS_PATH`, then `CLINE_DATA_DIR`, then `CLINE_DIR`
/// (with `data/`), then `<home>/.cline/data`. `~` and `~/` expand against home,
/// which is `HOME` when set and the user profile otherwise.
pub(super) fn providers_file_path(
    env: &dyn Fn(&str) -> Option<String>,
    home_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    let home = cleaned(env(HOME_ENV)).map(PathBuf::from).or(home_dir);
    let expand = |key: &str| -> Option<PathBuf> {
        let value = cleaned(env(key))?;
        if value == "~" {
            return home.clone();
        }
        if let Some(rest) = value
            .strip_prefix("~/")
            .or_else(|| value.strip_prefix("~\\"))
        {
            return home.as_ref().map(|h| h.join(rest));
        }
        Some(PathBuf::from(value))
    };

    if let Some(file) = expand(OVERRIDE_FILE_ENV) {
        return Some(file);
    }
    let directory = match expand(OVERRIDE_DATA_DIR_ENV) {
        Some(dir) => dir,
        None => expand(OVERRIDE_DIR_ENV)
            .or_else(|| home.as_ref().map(|h| h.join(".cline")))?
            .join("data"),
    };
    Some(directory.join("settings").join("providers.json"))
}

fn cleaned_str(value: Option<&Value>) -> Option<String> {
    cleaned(value.and_then(Value::as_str).map(str::to_string))
}

/// Parse `providers.cline.settings`. Mirrors Cline's `getApiKey`: a browser
/// access token wins over API keys kept in the same settings entry.
pub(super) fn parse_credential(bytes: &[u8]) -> Option<FileCredential> {
    let root: Value = serde_json::from_slice(bytes).ok()?;
    let settings = root
        .get("providers")?
        .as_object()?
        .get("cline")?
        .as_object()?
        .get("settings")?
        .as_object()?;
    let auth = settings.get("auth").and_then(Value::as_object);

    if let Some(access) = cleaned_str(auth.and_then(|a| a.get("accessToken"))) {
        let token = if access.starts_with(OAUTH_PREFIX) {
            access
        } else {
            format!("{OAUTH_PREFIX}{access}")
        };
        return Some(FileCredential {
            token,
            is_oauth: true,
        });
    }
    let key = cleaned_str(settings.get("apiKey"))
        .or_else(|| cleaned_str(auth.and_then(|a| a.get("apiKey"))))?;
    Some(FileCredential {
        token: key,
        is_oauth: false,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn wrap(settings: &str) -> Vec<u8> {
        format!(r#"{{"providers":{{"cline":{{"settings":{settings}}},"unrelated":false}}}}"#)
            .into_bytes()
    }

    #[test]
    fn file_parsing_preserves_credential_kind() {
        let cases = [
            (
                r#"{"auth":{"accessToken":" fixture-access "}}"#,
                "workos:fixture-access",
                true,
            ),
            (
                r#"{"auth":{"accessToken":"workos:fixture-access"}}"#,
                "workos:fixture-access",
                true,
            ),
            (r#"{"apiKey":" fixture-key "}"#, "fixture-key", false),
            (r#"{"auth":{"apiKey":"nested-key"}}"#, "nested-key", false),
            (
                r#"{"apiKey":"old-key","auth":{"accessToken":"new-session"}}"#,
                "workos:new-session",
                true,
            ),
            (
                r#"{"apiKey":"fixture-key","auth":{"accessToken":"  "}}"#,
                "fixture-key",
                false,
            ),
            (
                r#"{"apiKey":"fixture-key","auth":{"accessToken":123}}"#,
                "fixture-key",
                false,
            ),
        ];
        for (settings, token, is_oauth) in cases {
            assert_eq!(
                parse_credential(&wrap(settings)),
                Some(FileCredential {
                    token: token.to_string(),
                    is_oauth
                }),
                "{settings}"
            );
        }
    }

    #[test]
    fn invalid_and_unrelated_sessions_stay_unavailable() {
        let cases = [
            "{bad",
            "[]",
            "{}",
            r#"{"providers":{"cline":{"settings":{}}}}"#,
            r#"{"providers":{"cline":{"settings":[]}}}"#,
            r#"{"providers":{"cline-pass":{"settings":{"auth":{"accessToken":"other"}}}}}"#,
            r#"{"providers":{"cline":{"settings":{"auth":{"accessToken":123}}}}}"#,
            r#"{"providers":{"cline":{"settings":{"apiKey":"  "}}}}"#,
        ];
        for json in cases {
            assert_eq!(parse_credential(json.as_bytes()), None, "{json}");
        }
    }

    fn path_for(vars: &[(&str, &str)], home: &str) -> PathBuf {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        providers_file_path(&|key| map.get(key).cloned(), Some(PathBuf::from(home))).unwrap()
    }

    #[test]
    fn paths_follow_the_cline_overrides_and_home() {
        let home = "/synthetic/home";
        let default = PathBuf::from(home)
            .join(".cline")
            .join("data")
            .join("settings")
            .join("providers.json");
        assert_eq!(path_for(&[], home), default);
        assert_eq!(
            path_for(&[("HOME", "/synthetic/other")], home),
            PathBuf::from("/synthetic/other/.cline/data/settings/providers.json")
        );
        assert_eq!(
            path_for(&[("CLINE_DIR", "~/cline")], home),
            PathBuf::from("/synthetic/home/cline/data/settings/providers.json")
        );
        assert_eq!(
            path_for(
                &[("CLINE_DATA_DIR", "/data"), ("CLINE_DIR", "/unused")],
                home
            ),
            PathBuf::from("/data/settings/providers.json")
        );
        assert_eq!(
            path_for(
                &[
                    ("CLINE_PROVIDER_SETTINGS_PATH", "~/session.json"),
                    ("CLINE_DATA_DIR", "/unused")
                ],
                home
            ),
            PathBuf::from("/synthetic/home/session.json")
        );
        assert_eq!(
            path_for(&[("CLINE_DIR", "  ")], home),
            default,
            "blank overrides are ignored"
        );
    }

    #[test]
    fn missing_home_and_overrides_yield_no_path() {
        assert!(providers_file_path(&|_| None, None).is_none());
    }

    #[test]
    fn reads_session_without_touching_the_file() {
        for (content, token, is_oauth) in [
            (
                r#"{"providers":{"cline":{"settings":{"auth":{"accessToken":"fixture-access"}}}}}"#,
                "workos:fixture-access",
                true,
            ),
            (
                r#"{"providers":{"cline":{"settings":{"apiKey":"fixture-key"}}}}"#,
                "fixture-key",
                false,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("providers.json");
            std::fs::write(&file, content).unwrap();
            let credential = read_credential_at(&file).unwrap();
            assert_eq!(credential.token, token);
            assert_eq!(credential.is_oauth, is_oauth);
            assert_eq!(
                credential.login_method(),
                if is_oauth { "Browser" } else { "API key" }
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), content);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn missing_file_is_no_credential() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_credential_at(&dir.path().join("providers.json")), None);
    }
}
