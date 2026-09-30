//! Chromium `Local Storage` layer on top of the LevelDB reader.
//!
//! Chromium stores each origin's `localStorage` items under the key
//! `_<origin>\0<key>` where `<origin>` is the serialized origin (`https://www.kimi.ai`) and
//! `<key>` starts with a format byte: `0x00` for UTF-16LE text, `0x01` for Latin-1. Values use
//! the same format byte. Other keys in the database (`VERSION`, `META:<origin>`,
//! `METAACCESS:<origin>`) are bookkeeping and are ignored.

use super::{Entry, LevelDbError, MAX_TOTAL_SCAN_BYTES, read_entries_with_budget};
use std::path::{Path, PathBuf};

const KEY_PREFIX: u8 = b'_';
const ORIGIN_TERMINATOR: u8 = 0;
const FORMAT_UTF16LE: u8 = 0;
const FORMAT_LATIN1: u8 = 1;

/// One decoded `localStorage` item.
#[derive(Clone, PartialEq, Eq)]
pub struct LocalStorageEntry {
    pub key: String,
    pub value: String,
}

impl std::fmt::Debug for LocalStorageEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalStorageEntry")
            .field("key", &self.key)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Directory holding Local Storage for a Chromium profile directory (`Default`, `Profile 1`, ...).
pub fn local_storage_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join("Local Storage").join("leveldb")
}

/// Read the `localStorage` items stored for `origin` (for example `https://www.kimi.ai`) in the
/// LevelDB directory `dir`, sorted by key. A trailing slash on `origin` is ignored.
pub fn read_local_storage_entries(
    dir: &Path,
    origin: &str,
) -> Result<Vec<LocalStorageEntry>, LevelDbError> {
    read_local_storage_entries_with_budget(dir, origin, MAX_TOTAL_SCAN_BYTES)
        .map(|(entries, _)| entries)
}

pub(crate) fn read_local_storage_entries_with_budget(
    dir: &Path,
    origin: &str,
    max_total_bytes: u64,
) -> Result<(Vec<LocalStorageEntry>, u64), LevelDbError> {
    let prefix = origin_key_prefix(origin);
    let (entries, scanned_bytes) =
        read_entries_with_budget(dir, max_total_bytes, Some(prefix.as_slice()))?;
    Ok((decode_origin_entries(&entries, origin), scanned_bytes))
}

pub(super) fn decode_origin_entries(entries: &[Entry], origin: &str) -> Vec<LocalStorageEntry> {
    let prefix = origin_key_prefix(origin);

    entries
        .iter()
        .filter_map(|entry| {
            let key = decode_text(entry.key.strip_prefix(prefix.as_slice())?)?;
            let value = decode_text(&entry.value)?;
            Some(LocalStorageEntry { key, value })
        })
        .collect()
}

fn origin_key_prefix(origin: &str) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(origin.len() + 2);
    prefix.push(KEY_PREFIX);
    prefix.extend_from_slice(origin.trim_end_matches('/').as_bytes());
    prefix.push(ORIGIN_TERMINATOR);
    prefix
}

/// Decode a format-byte-prefixed Chromium string; `None` for an unknown format or bad UTF-16.
fn decode_text(bytes: &[u8]) -> Option<String> {
    let (&format, data) = bytes.split_first()?;
    match format {
        FORMAT_LATIN1 => Some(data.iter().map(|&byte| char::from(byte)).collect()),
        FORMAT_UTF16LE if data.len() % 2 == 0 => {
            let units: Vec<u16> = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => None,
    }
}
