//! Unchanged-artifact save skipping for the cost cache.
//!
//! `JsonlScanner::save_cache` used to rewrite the whole artifact (up to
//! `CostUsageCacheBudget::MAX_FILE_BYTES`) after every non-debounced scan, even
//! when only `last_scan_unix_ms` moved. Upstream 0.66.0 (#3882) avoids
//! rewriting unchanged retained file state in its SQLite store; the Windows
//! store is one JSON artifact, so the equivalent is to skip the write when the
//! payload, excluding the scan timestamp, matches the decoded baseline.
//!
//! Three pieces make that safe:
//! - Deterministic key order. `HashMap` iteration order differs between
//!   instances, so a reloaded cache would never re-encode to the same bytes.
//!   The `sorted_*` serializers give equal content equal bytes.
//! - A timestamp-free content stamp. The save path already has the encoded
//!   JSON in memory, so the comparison hashes around the timestamp instead of
//!   serializing the full cache into a second buffer.
//! - An in-memory debounce mark. A skipped save records the new scan time for
//!   this process only, keyed by the on-disk stamp it applies to, so the
//!   scanner debounce still works. After a restart the older on-disk time
//!   costs one cheap metadata-only rescan.

use super::*;
use serde::Serializer;
use std::sync::{LazyLock, Mutex, PoisonError};

struct Sorted<'a, V>(&'a HashMap<String, V>);

impl<V: Serialize> Serialize for Sorted<'_, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut entries: Vec<_> = self.0.iter().collect();
        entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
        serializer.collect_map(entries)
    }
}

/// Serialize a string-keyed map in key order.
pub(super) fn sorted_map<V: Serialize, S: Serializer>(
    map: &HashMap<String, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    Sorted(map).serialize(serializer)
}

/// Serialize a day -> model -> counts map in key order at both levels.
pub(super) fn sorted_days<S: Serializer>(
    days: &HashMap<String, HashMap<String, Vec<i64>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut entries: Vec<_> = days
        .iter()
        .map(|(day, models)| (day, Sorted(models)))
        .collect();
    entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    serializer.collect_map(entries)
}

type ScanTimeMarks = HashMap<PathBuf, (CacheStamp, i64)>;

/// Latest scan time from a skipped save, keyed by artifact path.
static SKIPPED_SCAN_TIMES: LazyLock<Mutex<ScanTimeMarks>> = LazyLock::new(Mutex::default);

fn record_scan_time(cache_path: &Path, stamp: &CacheStamp, scan_unix_ms: i64) {
    SKIPPED_SCAN_TIMES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(cache_path.to_path_buf(), (stamp.clone(), scan_unix_ms));
}

/// Scan time recorded by a skipped save against exactly this on-disk artifact.
pub(super) fn recorded_scan_time(cache_path: &Path, stamp: &CacheStamp) -> Option<i64> {
    let marks = SKIPPED_SCAN_TIMES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    marks
        .get(cache_path)
        .filter(|(recorded, _)| recorded == stamp)
        .map(|(_, scan_unix_ms)| *scan_unix_ms)
}

/// Whether saving `cache` would only change `last_scan_unix_ms` compared with
/// the decoded on-disk baseline. `json` is the encoding of `cache` as-is.
/// When it is unchanged the scan time is kept in memory and the caller skips
/// the write.
pub(super) fn skip_unchanged_save(cache_path: &Path, cache: &CostUsageCache, json: &str) -> bool {
    let Some(baseline) = cache.loaded_payload_stamp.as_ref() else {
        return false;
    };
    let Some(Some(stamp)) = cache.loaded_stamp.as_ref() else {
        return false;
    };
    let unchanged =
        CacheStamp::from_cache_payload(json.as_bytes()).is_some_and(|current| current == *baseline);
    if unchanged {
        record_scan_time(cache_path, stamp, cache.last_scan_unix_ms);
    }
    unchanged
}
