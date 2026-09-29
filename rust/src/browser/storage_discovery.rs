//! Locates raw Chromium profile stores (Local Storage, Session Storage, IndexedDB) across the
//! installed Chromium-family browsers.
//!
//! Each consumer supplies its own decoder; this module only answers "which directories could hold
//! the data". It never opens, locks, or decrypts anything, so it is safe to run while the browser
//! is open and needs no credential-store access.
//!
//! The browser catalog is [`super::detection::BrowserType`] (via [`BrowserDetector`]); a
//! Chromium-family browser that is not in that catalog is not visited.

use std::path::{Path, PathBuf};

use super::detection::BrowserDetector;
use super::leveldb::local_storage::local_storage_dir;

const INDEXED_DB_DIR: &str = "IndexedDB";
const SESSION_STORAGE_DIR: &str = "Session Storage";
const INDEXED_DB_SUFFIX: &str = ".indexeddb.leveldb";

/// Which per-profile store to locate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    /// `<profile>/Local Storage/leveldb`
    LocalStorage,
    /// `<profile>/Session Storage`
    SessionStorage,
    /// `<profile>/IndexedDB/<origin>.indexeddb.leveldb` for every database whose directory name
    /// starts with one of `origin_prefixes` (for example `https_platform.minimax.io_`).
    IndexedDb {
        origin_prefixes: &'static [&'static str],
    },
}

impl StorageKind {
    fn label_suffix(self) -> &'static str {
        match self {
            Self::LocalStorage => "",
            Self::SessionStorage => " (Session Storage)",
            Self::IndexedDb { .. } => " (IndexedDB)",
        }
    }
}

/// One directory that may hold data of the requested [`StorageKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageCandidate {
    /// Human-readable source, such as `Google Chrome Default (Session Storage)`.
    pub label: String,
    pub path: PathBuf,
}

/// Candidates for `kind` in every installed Chromium-family browser, in catalog order.
pub fn discover(kind: StorageKind) -> Vec<StorageCandidate> {
    BrowserDetector::detect_all()
        .iter()
        .filter(|browser| browser.browser_type.is_chromium_based())
        .flat_map(|browser| {
            candidates_in_user_data_dir(
                &browser.user_data_dir,
                browser.browser_type.display_name(),
                kind,
            )
        })
        .collect()
}

/// Candidates for `kind` in one browser `User Data` directory, profiles sorted by name.
///
/// Only `Default`, `Profile *`, and `user-*` directories count as profiles; guest and system
/// profiles are ignored.
pub fn candidates_in_user_data_dir(
    user_data_dir: &Path,
    label_prefix: &str,
    kind: StorageKind,
) -> Vec<StorageCandidate> {
    sorted_child_dirs(user_data_dir, is_profile_dir_name)
        .into_iter()
        .flat_map(|(name, profile_dir)| {
            let label = format!("{label_prefix} {name}{}", kind.label_suffix());
            profile_store_paths(&profile_dir, kind)
                .into_iter()
                .map(move |path| StorageCandidate {
                    label: label.clone(),
                    path,
                })
        })
        .collect()
}

fn profile_store_paths(profile_dir: &Path, kind: StorageKind) -> Vec<PathBuf> {
    match kind {
        StorageKind::LocalStorage => existing(local_storage_dir(profile_dir)),
        StorageKind::SessionStorage => existing(profile_dir.join(SESSION_STORAGE_DIR)),
        StorageKind::IndexedDb { origin_prefixes } => {
            sorted_child_dirs(&profile_dir.join(INDEXED_DB_DIR), |name| {
                name.ends_with(INDEXED_DB_SUFFIX)
                    && origin_prefixes
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
            })
            .into_iter()
            .map(|(_, path)| path)
            .collect()
        }
    }
}

fn existing(path: PathBuf) -> Vec<PathBuf> {
    if path.exists() {
        vec![path]
    } else {
        Vec::new()
    }
}

fn is_profile_dir_name(name: &str) -> bool {
    name == "Default" || name.starts_with("Profile ") || name.starts_with("user-")
}

/// Child directories of `dir` whose (non-hidden, valid UTF-8) name passes `keep`, sorted by name.
/// A missing or unreadable `dir` yields nothing.
fn sorted_child_dirs(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut children: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            (is_dir && !name.starts_with('.') && keep(&name)).then(|| (name, entry.path()))
        })
        .collect();
    children.sort_by(|left, right| left.0.cmp(&right.0));
    children
}

#[cfg(test)]
mod tests;
