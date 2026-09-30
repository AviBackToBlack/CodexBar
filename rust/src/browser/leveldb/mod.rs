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
//! - Files larger than [`MAX_FILE_BYTES`], scans larger than [`MAX_TOTAL_SCAN_BYTES`], and blocks
//!   that decode beyond [`MAX_BLOCK_BYTES`] are skipped so a corrupt profile cannot exhaust
//!   memory or monopolize a refresh.

pub mod local_storage;
mod log;
pub mod snappy;
mod table;
mod varint;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;

/// Largest single log or table file that will be read into memory.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest total number of bytes scanned across a LevelDB directory.
pub const MAX_TOTAL_SCAN_BYTES: u64 = 256 * 1024 * 1024;
/// Largest decompressed table block accepted.
pub const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 8192;
const MAX_RECORDS_PER_DIRECTORY: usize = 250_000;
const MAX_LIVE_ENTRIES: usize = 100_000;
const MAX_LIVE_ENTRY_BYTES: usize = 64 * 1024 * 1024;

/// One live key/value pair of the database.
#[derive(Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Entry")
            .field("key", &"[REDACTED]")
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Failure to read a LevelDB directory at all (per-file problems are skipped, not reported).
#[derive(Debug, thiserror::Error)]
pub enum LevelDbError {
    #[error("cannot read LevelDB directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("LevelDB directory exceeds safe scan limits")]
    ResourceLimit,
}

/// A put (`value: Some`) or delete (`value: None`) with the sequence number it was written at.
pub(crate) struct Record {
    pub(crate) key: Vec<u8>,
    pub(crate) sequence: u64,
    pub(crate) value: Option<Vec<u8>>,
}

#[derive(Default)]
struct EntryAccumulator<'a> {
    key_prefix: Option<&'a [u8]>,
    newest: BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)>,
    record_count: usize,
    retained_bytes: usize,
    resource_limit_exceeded: bool,
}

impl<'a> EntryAccumulator<'a> {
    fn absorb(&mut self, record: Record) -> bool {
        if self.record_count == MAX_RECORDS_PER_DIRECTORY {
            self.resource_limit_exceeded = true;
            return false;
        }
        self.record_count += 1;
        if self
            .key_prefix
            .is_some_and(|prefix| !record.key.starts_with(prefix))
        {
            return true;
        }

        let current_entry_count = self.newest.len();
        match self.newest.entry(record.key) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                if existing.0 >= record.sequence {
                    return true;
                }
                let old_value_bytes = existing.1.as_ref().map_or(0, Vec::len);
                let new_value_bytes = record.value.as_ref().map_or(0, Vec::len);
                let next_retained_bytes = self
                    .retained_bytes
                    .checked_sub(old_value_bytes)
                    .and_then(|bytes| bytes.checked_add(new_value_bytes));
                let Some(next_retained_bytes) = next_retained_bytes else {
                    self.resource_limit_exceeded = true;
                    return false;
                };
                if next_retained_bytes > MAX_LIVE_ENTRY_BYTES {
                    self.resource_limit_exceeded = true;
                    return false;
                }
                self.retained_bytes = next_retained_bytes;
                *existing = (record.sequence, record.value);
                true
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                let Some(entry_bytes) = entry
                    .key()
                    .len()
                    .checked_add(record.value.as_ref().map_or(0, Vec::len))
                else {
                    self.resource_limit_exceeded = true;
                    return false;
                };
                let Some(next_retained_bytes) = self.retained_bytes.checked_add(entry_bytes) else {
                    self.resource_limit_exceeded = true;
                    return false;
                };
                if current_entry_count == MAX_LIVE_ENTRIES
                    || next_retained_bytes > MAX_LIVE_ENTRY_BYTES
                {
                    self.resource_limit_exceeded = true;
                    return false;
                }
                self.retained_bytes = next_retained_bytes;
                entry.insert((record.sequence, record.value));
                true
            }
        }
    }

    fn into_entries(self) -> Vec<Entry> {
        self.newest
            .into_iter()
            .filter_map(|(key, (_, value))| value.map(|value| Entry { key, value }))
            .collect()
    }
}

/// Read every live entry of the LevelDB directory `dir`, sorted by key.
///
/// Keys that were deleted (or whose newest record is a delete) are omitted. Unreadable or corrupt
/// files are skipped with a debug log; scans over the configured resource limits fail as a whole.
pub fn read_entries(dir: &Path) -> Result<Vec<Entry>, LevelDbError> {
    read_entries_with_budget(dir, MAX_TOTAL_SCAN_BYTES, None).map(|(entries, _)| entries)
}

pub(crate) fn read_entries_with_budget(
    dir: &Path,
    max_total_bytes: u64,
    key_prefix: Option<&[u8]>,
) -> Result<(Vec<Entry>, u64), LevelDbError> {
    let mut entries = EntryAccumulator {
        key_prefix,
        ..EntryAccumulator::default()
    };
    let mut scanned_bytes = 0u64;

    for (directory_entry_count, dir_entry) in std::fs::read_dir(dir)?.enumerate() {
        if directory_entry_count == MAX_DIRECTORY_ENTRIES {
            return Err(LevelDbError::ResourceLimit);
        }
        let Ok(dir_entry) = dir_entry else { continue };
        let path = dir_entry.path();
        let Some(kind) = FileKind::of(&path) else {
            continue;
        };
        let remaining_bytes = max_total_bytes.saturating_sub(scanned_bytes);
        let data = match read_bounded_file(&path, remaining_bytes) {
            Ok(data) => data,
            Err(BoundedReadError::FileTooLarge) => continue,
            Err(BoundedReadError::BudgetExceeded) => return Err(LevelDbError::ResourceLimit),
            Err(BoundedReadError::Io(error)) => {
                tracing::debug!(file = ?path.file_name(), %error, "skipping unreadable LevelDB file");
                continue;
            }
        };
        let file_bytes = u64::try_from(data.len()).map_err(|_| LevelDbError::ResourceLimit)?;
        scanned_bytes = scanned_bytes
            .checked_add(file_bytes)
            .ok_or(LevelDbError::ResourceLimit)?;
        match kind {
            FileKind::Log => {
                let _ = log::read_log_until(&data, &mut |record| entries.absorb(record));
            }
            FileKind::Table => {
                match table::read_table_until(&data, &mut |record| entries.absorb(record)) {
                    Ok((0, _)) => {}
                    Ok((skipped, _)) => tracing::debug!(
                        file = ?path.file_name(),
                        skipped,
                        "skipped undecodable LevelDB table blocks"
                    ),
                    Err(error) => {
                        tracing::debug!(file = ?path.file_name(), %error, "skipping malformed LevelDB table");
                    }
                }
            }
        }
        if entries.resource_limit_exceeded {
            return Err(LevelDbError::ResourceLimit);
        }
    }

    Ok((entries.into_entries(), scanned_bytes))
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

enum BoundedReadError {
    Io(std::io::Error),
    FileTooLarge,
    BudgetExceeded,
}

fn read_bounded_file(path: &Path, remaining_bytes: u64) -> Result<Vec<u8>, BoundedReadError> {
    let file = std::fs::File::open(path).map_err(BoundedReadError::Io)?;
    let size = file.metadata().map_err(BoundedReadError::Io)?.len();
    if size > MAX_FILE_BYTES {
        return Err(BoundedReadError::FileTooLarge);
    }
    if size > remaining_bytes {
        return Err(BoundedReadError::BudgetExceeded);
    }

    let read_limit = MAX_FILE_BYTES.min(remaining_bytes);
    let reserve = usize::try_from(size).map_err(|_| BoundedReadError::FileTooLarge)?;
    let mut data = Vec::new();
    data.try_reserve_exact(reserve)
        .map_err(|error| BoundedReadError::Io(std::io::Error::other(error)))?;
    file.take(read_limit + 1)
        .read_to_end(&mut data)
        .map_err(BoundedReadError::Io)?;
    let data_len = u64::try_from(data.len()).map_err(|_| BoundedReadError::FileTooLarge)?;
    if data_len > MAX_FILE_BYTES {
        return Err(BoundedReadError::FileTooLarge);
    }
    if data_len > remaining_bytes {
        return Err(BoundedReadError::BudgetExceeded);
    }
    Ok(data)
}
