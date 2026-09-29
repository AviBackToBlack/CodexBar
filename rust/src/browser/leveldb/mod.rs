//! Read-only LevelDB reader for Chromium browser storage (Local Storage, and other LevelDB
//! directories a browser keeps under a profile).
//!
//! This is a small hand-written reader rather than a dependency: it understands the write-ahead
//! log and sorted-table formats, including Snappy-compressed table blocks, and resolves each user
//! key to its newest value by sequence number. It does not open the database, take its `LOCK`, or
//! write anything, so it can be pointed at a profile of a running browser.
//!
//! Known limits, all acceptable for a best-effort credential import:
//! - The `MANIFEST` is not consulted, so a table that compaction already obsoleted but the
//!   browser has not yet removed can still be read. Newer sequence numbers win, so this only
//!   matters for a deleted key whose tombstone was compacted away.
//! - Checksums are not verified and only the bytewise comparator is assumed (which is what
//!   `Local Storage` uses; a full scan does not depend on ordering anyway).
//! - Blocks using a compression type other than none/Snappy are skipped.
//! - Files larger than [`MAX_FILE_BYTES`] and blocks that inflate beyond [`MAX_BLOCK_BYTES`] are
//!   skipped so a corrupt profile cannot exhaust memory.

pub mod local_storage;
mod log;
pub mod snappy;
mod table;
mod varint;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

/// Largest single log or table file that will be read into memory.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest decompressed table block accepted.
pub const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;

/// One live key/value pair of the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// Failure to read a LevelDB directory at all (per-file problems are skipped, not reported).
#[derive(Debug, thiserror::Error)]
pub enum LevelDbError {
    #[error("cannot read LevelDB directory: {0}")]
    Io(#[from] std::io::Error),
}

/// A put (`value: Some`) or delete (`value: None`) with the sequence number it was written at.
pub(crate) struct Record {
    pub(crate) key: Vec<u8>,
    pub(crate) sequence: u64,
    pub(crate) value: Option<Vec<u8>>,
}

/// Read every live entry of the LevelDB directory `dir`, sorted by key.
///
/// Keys that were deleted (or whose newest record is a delete) are omitted. Unreadable or corrupt
/// files are skipped with a debug log so the remaining data is still returned.
pub fn read_entries(dir: &Path) -> Result<Vec<Entry>, LevelDbError> {
    let mut newest: BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)> = BTreeMap::new();
    let mut absorb = |record: Record| match newest.get_mut(&record.key) {
        Some(existing) if existing.0 >= record.sequence => {}
        Some(existing) => *existing = (record.sequence, record.value),
        None => {
            newest.insert(record.key, (record.sequence, record.value));
        }
    };

    for dir_entry in std::fs::read_dir(dir)? {
        let Ok(dir_entry) = dir_entry else { continue };
        let path = dir_entry.path();
        let Some(kind) = FileKind::of(&path) else {
            continue;
        };
        let data = match read_bounded_file(&path) {
            Ok(data) => data,
            Err(error) => {
                tracing::debug!(file = ?path.file_name(), %error, "skipping unreadable LevelDB file");
                continue;
            }
        };
        match kind {
            FileKind::Log => log::read_log(&data, &mut absorb),
            FileKind::Table => match table::read_table(&data, &mut absorb) {
                Ok(0) => {}
                Ok(skipped) => tracing::debug!(
                    file = ?path.file_name(),
                    skipped,
                    "skipped undecodable LevelDB table blocks"
                ),
                Err(error) => {
                    tracing::debug!(file = ?path.file_name(), %error, "skipping malformed LevelDB table");
                }
            },
        }
    }

    Ok(newest
        .into_iter()
        .filter_map(|(key, (_, value))| value.map(|value| Entry { key, value }))
        .collect())
}

#[derive(Clone, Copy)]
enum FileKind {
    Log,
    Table,
}

impl FileKind {
    fn of(path: &Path) -> Option<Self> {
        let extension = path.extension().and_then(OsStr::to_str)?;
        if extension.eq_ignore_ascii_case("log") {
            Some(Self::Log)
        } else if extension.eq_ignore_ascii_case("ldb") || extension.eq_ignore_ascii_case("sst") {
            Some(Self::Table)
        } else {
            None
        }
    }
}

fn read_bounded_file(path: &Path) -> std::io::Result<Vec<u8>> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_FILE_BYTES {
        return Err(std::io::Error::other(format!(
            "file is {size} bytes, over the {MAX_FILE_BYTES} byte limit"
        )));
    }
    std::fs::read(path)
}
