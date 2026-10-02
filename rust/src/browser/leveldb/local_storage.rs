//! Chromium `Local Storage` layer on top of the LevelDB reader.
//!
//! Chromium stores each origin's `localStorage` items under the key
//! `_<origin>\0<key>` where `<origin>` is the serialized origin (`https://www.kimi.ai`) and
//! `<key>` starts with a format byte: `0x00` for UTF-16LE text, `0x01` for Latin-1. Values use
//! the same format byte. Other keys in the database (`VERSION`, `META:<origin>`,
//! `METAACCESS:<origin>`) are bookkeeping and are ignored.

use super::{Entry, LevelDbError, read_entries};
use std::path::{Path, PathBuf};

const KEY_PREFIX: u8 = b'_';
const ORIGIN_TERMINATOR: u8 = 0;
const FORMAT_UTF16LE: u8 = 0;
const FORMAT_LATIN1: u8 = 1;

/// One decoded `localStorage` item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStorageEntry {
    pub key: String,
    pub value: String,
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
    read_local_storage_entries_for_origins(dir, &[origin])
}

/// Read `localStorage` items for any of `origins` with one LevelDB scan, sorted by key within
/// each origin. A trailing slash on an origin is ignored.
pub fn read_local_storage_entries_for_origins(
    dir: &Path,
    origins: &[&str],
) -> Result<Vec<LocalStorageEntry>, LevelDbError> {
    let entries = read_entries(dir)?;
    Ok(origins
        .iter()
        .flat_map(|origin| decode_origin_entries(&entries, origin))
        .collect())
}

pub(super) fn decode_origin_entries(entries: &[Entry], origin: &str) -> Vec<LocalStorageEntry> {
    let mut prefix = Vec::with_capacity(origin.len() + 2);
    prefix.push(KEY_PREFIX);
    prefix.extend_from_slice(origin.trim_end_matches('/').as_bytes());
    prefix.push(ORIGIN_TERMINATOR);

    entries
        .iter()
        .filter_map(|entry| {
            let key = decode_text(entry.key.strip_prefix(prefix.as_slice())?)?;
            let value = decode_text(&entry.value)?;
            Some(LocalStorageEntry { key, value })
        })
        .collect()
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
