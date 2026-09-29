//! MiniMax LocalStorage Importer
//!
//! Extracts session data from Chromium browser storage for the MiniMax platform.
//! Storage directories come from `browser::storage_discovery` (every installed Chromium-family
//! browser and profile: Local Storage, then Session Storage, then MiniMax IndexedDB).

use crate::browser::storage_discovery::{self, StorageCandidate, StorageKind};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Session data extracted from MiniMax localStorage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiniMaxSession {
    pub access_token: Option<String>,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub plan_type: Option<String>,
    pub source_label: String,
}

/// Error type for localStorage import
#[derive(Debug)]
pub enum ImportError {
    BrowserNotFound,
    StorageNotFound,
    ParseError(String),
    AccessDenied(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::BrowserNotFound => write!(f, "No supported browser found"),
            ImportError::StorageNotFound => write!(f, "localStorage data not found"),
            ImportError::ParseError(e) => write!(f, "Parse error: {}", e),
            ImportError::AccessDenied(e) => write!(f, "Access denied: {}", e),
        }
    }
}

impl std::error::Error for ImportError {}

/// Origin prefixes of the MiniMax IndexedDB databases (`<scheme>_<host>_<port>`).
const INDEXED_DB_ORIGIN_PREFIXES: &[&str] = &[
    "https_platform.minimax.io_",
    "https_www.minimax.io_",
    "https_minimax.io_",
    "https_platform.minimaxi.com_",
    "https_minimaxi.com_",
    "https_www.minimaxi.com_",
];

/// Stores tried in order; a later store is read only when earlier ones yield no session.
const STORAGE_ORDER: [StorageKind; 3] = [
    StorageKind::LocalStorage,
    StorageKind::SessionStorage,
    StorageKind::IndexedDb {
        origin_prefixes: INDEXED_DB_ORIGIN_PREFIXES,
    },
];

/// MiniMax localStorage importer
pub struct MiniMaxLocalStorageImporter;

impl MiniMaxLocalStorageImporter {
    /// Import a MiniMax session from Chromium browser storage.
    ///
    /// Visits every installed Chromium-family browser and profile: Local Storage first, then
    /// Session Storage, then MiniMax-origin IndexedDB when the earlier stores yield nothing.
    pub fn import_session() -> Result<MiniMaxSession, ImportError> {
        Self::import_with(storage_discovery::discover)
    }

    fn import_with(
        discover: impl Fn(StorageKind) -> Vec<StorageCandidate>,
    ) -> Result<MiniMaxSession, ImportError> {
        let mut found_any_store = false;
        for kind in STORAGE_ORDER {
            for candidate in discover(kind) {
                found_any_store = true;
                if let Ok(session) = Self::extract_from_path(&candidate.path, &candidate.label) {
                    return Ok(session);
                }
            }
        }

        Err(if found_any_store {
            ImportError::StorageNotFound
        } else {
            ImportError::BrowserNotFound
        })
    }

    /// Extract session from a storage directory
    fn extract_from_path(path: &Path, source_label: &str) -> Result<MiniMaxSession, ImportError> {
        // Look for .ldb or .log files
        let entries =
            std::fs::read_dir(path).map_err(|e| ImportError::AccessDenied(e.to_string()))?;

        let mut minimax_data: Option<serde_json::Value> = None;

        for entry in entries.flatten() {
            let entry_path = entry.path();
            if let Some(ext) = entry_path.extension()
                && (ext == "ldb" || ext == "log")
            {
                // Read file and search for MiniMax data
                if let Ok(contents) = std::fs::read(&entry_path) {
                    // Search for MiniMax-related JSON in the binary content
                    if let Some(data) = Self::extract_minimax_json(&contents) {
                        minimax_data = Some(data);
                        break;
                    }
                }
            }
        }

        match minimax_data {
            Some(json) => Self::parse_session_from_json(&json, source_label),
            None => Err(ImportError::StorageNotFound),
        }
    }

    /// Extract MiniMax JSON from binary localStorage data
    fn extract_minimax_json(data: &[u8]) -> Option<serde_json::Value> {
        // Convert to string, handling binary data
        let content = String::from_utf8_lossy(data);

        // Look for patterns that indicate MiniMax session data
        let patterns = [
            "minimax_user",
            "minimax_session",
            "platform.minimaxi.com",
            "mm_token",
            "mm_user_info",
        ];

        for pattern in patterns {
            if let Some(parsed) = Self::extract_json_after_pattern(&content, pattern) {
                return Some(parsed);
            }
        }

        None
    }

    fn extract_json_after_pattern(content: &str, pattern: &str) -> Option<serde_json::Value> {
        let json_start = Self::json_start_after_pattern(content, pattern)?;
        let remaining = &content[json_start..];
        let end_idx = Self::matching_json_object_end(remaining)?;
        serde_json::from_str(&remaining[..end_idx]).ok()
    }

    fn json_start_after_pattern(content: &str, pattern: &str) -> Option<usize> {
        let start_idx = content.find(pattern)?;
        content[start_idx..]
            .find('{')
            .map(|offset| start_idx + offset)
    }

    fn matching_json_object_end(content: &str) -> Option<usize> {
        let mut depth = 0;
        for (i, c) in content.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Parse session from extracted JSON
    fn parse_session_from_json(
        json: &serde_json::Value,
        source_label: &str,
    ) -> Result<MiniMaxSession, ImportError> {
        let access_token = json
            .get("access_token")
            .or_else(|| json.get("token"))
            .or_else(|| json.get("mm_token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let user_id = json
            .get("user_id")
            .or_else(|| json.get("userId"))
            .or_else(|| json.get("id"))
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            });

        let group_id = json
            .get("group_id")
            .or_else(|| json.get("groupId"))
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            });

        let email = json
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let phone = json
            .get("phone")
            .or_else(|| json.get("mobile"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let plan_type = json
            .get("plan_type")
            .or_else(|| json.get("planType"))
            .or_else(|| json.get("plan"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Need at least access_token or user_id
        if access_token.is_none() && user_id.is_none() {
            return Err(ImportError::ParseError(
                "No valid session data found".to_string(),
            ));
        }

        Ok(MiniMaxSession {
            access_token,
            user_id,
            group_id,
            email,
            phone,
            plan_type,
            source_label: source_label.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_session_json() {
        let json = serde_json::json!({
            "access_token": "test_token",
            "user_id": "12345",
            "group_id": "67890",
            "email": "test@example.com"
        });

        let result = MiniMaxLocalStorageImporter::parse_session_from_json(&json, "Chrome");
        assert!(result.is_ok());

        let session = result.unwrap();
        assert_eq!(session.access_token, Some("test_token".to_string()));
        assert_eq!(session.user_id, Some("12345".to_string()));
    }

    #[test]
    fn test_parse_session_empty() {
        let json = serde_json::json!({
            "foo": "bar"
        });

        let result = MiniMaxLocalStorageImporter::parse_session_from_json(&json, "Chrome");
        assert!(result.is_err());
    }

    fn candidate(label: &str, path: &Path) -> StorageCandidate {
        StorageCandidate {
            label: label.to_string(),
            path: path.to_path_buf(),
        }
    }

    fn write_store(dir: &Path, token: Option<&str>) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let body = match token {
            Some(token) => {
                format!(" minimax_user{{\"access_token\":\"{token}\",\"user_id\":\"1\"}}")
            }
            None => " unrelated".to_string(),
        };
        std::fs::write(dir.join("000003.log"), body).unwrap();
        dir.to_path_buf()
    }

    fn discover_from(
        local: Vec<StorageCandidate>,
        session: Vec<StorageCandidate>,
        indexed: Vec<StorageCandidate>,
    ) -> impl Fn(StorageKind) -> Vec<StorageCandidate> {
        move |kind| match kind {
            StorageKind::LocalStorage => local.clone(),
            StorageKind::SessionStorage => session.clone(),
            StorageKind::IndexedDb { origin_prefixes } => {
                assert_eq!(origin_prefixes, INDEXED_DB_ORIGIN_PREFIXES);
                indexed.clone()
            }
        }
    }

    #[test]
    fn import_prefers_local_storage_over_later_stores() {
        let root = tempfile::tempdir().unwrap();
        let local = write_store(&root.path().join("local"), Some("from-local"));
        let session = write_store(&root.path().join("session"), Some("from-session"));

        let found = MiniMaxLocalStorageImporter::import_with(discover_from(
            vec![candidate("Chrome Default", &local)],
            vec![candidate("Chrome Default (Session Storage)", &session)],
            Vec::new(),
        ))
        .unwrap();

        assert_eq!(found.access_token.as_deref(), Some("from-local"));
        assert_eq!(found.source_label, "Chrome Default");
    }

    #[test]
    fn import_falls_back_to_session_then_indexed_db_when_earlier_stores_are_empty() {
        let root = tempfile::tempdir().unwrap();
        let local = write_store(&root.path().join("local"), None);
        let session = write_store(&root.path().join("session"), None);
        let indexed = write_store(&root.path().join("indexed"), Some("from-indexed"));

        let found = MiniMaxLocalStorageImporter::import_with(discover_from(
            vec![candidate("Edge Default", &local)],
            vec![candidate("Edge Default (Session Storage)", &session)],
            vec![candidate("Edge Default (IndexedDB)", &indexed)],
        ))
        .unwrap();
        assert_eq!(found.access_token.as_deref(), Some("from-indexed"));
        assert_eq!(found.source_label, "Edge Default (IndexedDB)");

        let session_token = write_store(&root.path().join("session2"), Some("from-session"));
        let found = MiniMaxLocalStorageImporter::import_with(discover_from(
            vec![candidate("Edge Default", &local)],
            vec![candidate("Edge Default (Session Storage)", &session_token)],
            vec![candidate("Edge Default (IndexedDB)", &indexed)],
        ))
        .unwrap();
        assert_eq!(found.access_token.as_deref(), Some("from-session"));
    }

    #[test]
    fn import_distinguishes_no_browser_from_no_session() {
        let root = tempfile::tempdir().unwrap();
        let empty = write_store(&root.path().join("empty"), None);

        let none = MiniMaxLocalStorageImporter::import_with(discover_from(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        assert!(matches!(none, Err(ImportError::BrowserNotFound)));

        let no_session = MiniMaxLocalStorageImporter::import_with(discover_from(
            vec![candidate("Brave Default", &empty)],
            Vec::new(),
            Vec::new(),
        ));
        assert!(matches!(no_session, Err(ImportError::StorageNotFound)));
    }
}
