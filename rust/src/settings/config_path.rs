//! `CODEXBAR_CONFIG` (upstream parity): a CLI process can point its settings
//! file somewhere else, for isolated runs and embedding apps.
//!
//! Upstream keeps provider enablement, order, API keys, cookie headers and
//! token accounts in one config file. Here they live in `settings.json`,
//! `api_keys.json`, `manual_cookies.json` and `token-accounts.json`, so the
//! override replaces the settings file and moves those three stores beside
//! it. Only the `codexbar` CLI applies the override
//! ([`apply_config_path_env`]); the desktop shell never reads the variable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Environment variable naming the settings file to use instead of
/// `%APPDATA%\CodexBar\settings.json`.
pub const CONFIG_PATH_ENV: &str = "CODEXBAR_CONFIG";

static SETTINGS_FILE_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Resolve a raw `CODEXBAR_CONFIG` value. Surrounding whitespace is trimmed,
/// an empty value is ignored, a leading `~` becomes the home directory, and
/// a relative path is anchored at `cwd`.
pub fn resolve_config_path(
    raw: Option<&OsStr>,
    home: Option<&Path>,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let raw = raw?;
    let path = match raw.to_str() {
        Some(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            expand_home(text, home)
        }
        // Not valid Unicode: use it verbatim rather than guess.
        None => PathBuf::from(raw),
    };
    if path.is_absolute() {
        return Some(path);
    }
    Some(match cwd {
        Some(cwd) => cwd.join(path),
        None => path,
    })
}

fn expand_home(text: &str, home: Option<&Path>) -> PathBuf {
    let rest = if text == "~" {
        Some("")
    } else {
        text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\"))
    };
    match (rest, home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(text),
    }
}

/// Apply `CODEXBAR_CONFIG` to this process. The `codexbar` CLI calls this
/// once at startup, before any settings access; it returns the settings file
/// in effect when the variable is set.
pub fn apply_config_path_env() -> Option<&'static Path> {
    let resolved = resolve_config_path(
        std::env::var_os(CONFIG_PATH_ENV).as_deref(),
        dirs::home_dir().as_deref(),
        std::env::current_dir().ok().as_deref(),
    )?;
    Some(SETTINGS_FILE_OVERRIDE.get_or_init(|| resolved).as_path())
}

/// The settings file chosen through `CODEXBAR_CONFIG`, when the CLI applied one.
pub fn settings_file_override() -> Option<&'static Path> {
    SETTINGS_FILE_OVERRIDE.get().map(PathBuf::as_path)
}

/// Directory holding `api_keys.json`, `manual_cookies.json` and
/// `token-accounts.json` for this process.
pub fn config_store_dir() -> Option<PathBuf> {
    store_dir_for(settings_file_override(), crate::logging::config_root())
}

pub(super) fn settings_file_for(
    override_file: Option<&Path>,
    config_root: Option<PathBuf>,
) -> Option<PathBuf> {
    match override_file {
        Some(file) => Some(file.to_path_buf()),
        None => config_root.map(|root| root.join("settings.json")),
    }
}

fn store_dir_for(override_file: Option<&Path>, config_root: Option<PathBuf>) -> Option<PathBuf> {
    match override_file {
        Some(file) => Some(file.parent().map(Path::to_path_buf).unwrap_or_default()),
        None => config_root,
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_config_path, settings_file_for, store_dir_for};
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\fixture")
        } else {
            PathBuf::from("/fixture")
        }
    }

    fn resolve(raw: &str) -> Option<PathBuf> {
        let home = root().join("home");
        let cwd = root().join("work");
        resolve_config_path(Some(OsStr::new(raw)), Some(&home), Some(&cwd))
    }

    #[test]
    fn unset_or_blank_values_keep_the_default_settings_file() {
        assert_eq!(resolve_config_path(None, None, None), None);
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("  \t "), None);
    }

    #[test]
    fn absolute_values_are_used_as_the_settings_file() {
        let file = root().join("isolated").join("codexbar.json");
        assert_eq!(
            resolve(&format!("  {}  ", file.display())),
            Some(file.clone())
        );
    }

    #[test]
    fn tilde_expands_to_home_and_relative_paths_anchor_at_cwd() {
        let home = root().join("home");
        assert_eq!(resolve("~"), Some(home.clone()));
        assert_eq!(
            resolve("~/codexbar/config.json"),
            Some(home.join("codexbar/config.json"))
        );
        assert_eq!(
            resolve("~\\codexbar\\config.json"),
            Some(home.join("codexbar\\config.json"))
        );
        assert_eq!(
            resolve("settings.json"),
            Some(root().join("work").join("settings.json"))
        );
        assert_eq!(
            resolve_config_path(Some(OsStr::new("~/c.json")), None, None),
            Some(PathBuf::from("~/c.json")),
            "without a home directory the value stays literal"
        );
    }

    #[test]
    fn override_moves_the_settings_file_and_its_sibling_stores() {
        let config_root = root().join("AppData").join("CodexBar");
        assert_eq!(
            settings_file_for(None, Some(config_root.clone())),
            Some(config_root.join("settings.json"))
        );
        assert_eq!(
            store_dir_for(None, Some(config_root.clone())),
            Some(config_root.clone())
        );

        let file = root().join("isolated").join("codexbar.json");
        assert_eq!(
            settings_file_for(Some(&file), Some(config_root.clone())),
            Some(file.clone())
        );
        assert_eq!(
            store_dir_for(Some(&file), Some(config_root)),
            Some(root().join("isolated"))
        );
        assert_eq!(
            settings_file_for(Some(Path::new("x.json")), None),
            Some(PathBuf::from("x.json"))
        );
    }
}
