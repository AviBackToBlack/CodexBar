//! Claude usage-limit resets ("Reset for free" in Claude Settings > Usage),
//! read from the `cedar_ember` block of the Claude Web usage response.
//!
//! The result is a display-only [`ProviderInventoryItem`]: a count plus the
//! soonest expiry. Grant identifiers are redemption handles and are never
//! deserialized, so they cannot reach the bridge, CLI output, or any
//! persisted snapshot. Only the Web source reads this block.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::core::ProviderInventoryItem;

const INVENTORY_ID: &str = "reset-credits";
const INVENTORY_TITLE: &str = "Limit Reset Credits";

/// Observed grants hold one reset each; a larger total is treated as
/// malformed instead of allocated.
const MAX_RESETS: usize = 50;
/// Upper bound on grant records, checked before any record is decoded.
const MAX_GRANT_RECORDS: usize = 200;

/// Build the display inventory from the raw `cedar_ember` value.
///
/// Anything unreadable yields `None` and leaves the usage windows intact.
pub(super) fn inventory_from_block(
    block: &serde_json::Value,
    now: DateTime<Utc>,
) -> Option<ProviderInventoryItem> {
    ResetStatus::deserialize(block).ok()?.inventory(now)
}

/// Raw `cedar_ember` block. Only `eligible: true` yields an inventory.
#[derive(Debug, Deserialize)]
struct ResetStatus {
    eligible: bool,
    #[serde(default, deserialize_with = "lossy_grants")]
    grants: Vec<Grant>,
}

/// `resets_left` and `resets_total` are `i64` so negative or oversized
/// values are read (and rejected by the bounds check) instead of coerced.
/// `paused` is required: a grant with unknown pause state is dropped, not
/// counted. `usable_now` is deliberately not read: a saved reset counts
/// even while Claude gates redemption.
#[derive(Debug, Deserialize)]
struct Grant {
    resets_left: i64,
    #[serde(default)]
    resets_total: Option<i64>,
    #[serde(default, deserialize_with = "optional_bound")]
    starts_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "optional_bound")]
    ends_at: Option<DateTime<Utc>>,
    paused: bool,
}

impl Grant {
    /// `resets_left` must lie within `0..=resets_total`.
    fn is_well_formed(&self) -> bool {
        self.resets_left >= 0
            && self
                .resets_total
                .is_none_or(|total| total >= self.resets_left)
    }

    /// Not paused, not used up, started, and not expired at `now`.
    fn is_available(&self, now: DateTime<Utc>) -> bool {
        !self.paused
            && self.resets_left > 0
            && self.starts_at.is_none_or(|start| start <= now)
            && self.ends_at.is_none_or(|end| end > now)
    }
}

impl ResetStatus {
    fn inventory(&self, now: DateTime<Utc>) -> Option<ProviderInventoryItem> {
        if !self.eligible {
            return None;
        }
        let mut count = 0usize;
        let mut next_expiry: Option<DateTime<Utc>> = None;
        for grant in self.grants.iter().filter(|grant| grant.is_available(now)) {
            let resets = usize::try_from(grant.resets_left).ok()?;
            if resets > MAX_RESETS - count {
                return None;
            }
            count += resets;
            if let Some(ends_at) = grant.ends_at {
                next_expiry = Some(next_expiry.map_or(ends_at, |current| current.min(ends_at)));
            }
        }
        Some(ProviderInventoryItem {
            id: INVENTORY_ID.to_string(),
            title: INVENTORY_TITLE.to_string(),
            available_count: u32::try_from(count).ok().filter(|count| *count > 0)?,
            next_expires_at: next_expiry,
        })
    }
}

/// A malformed grant is dropped without hiding the rest. A `grants` value
/// that is not an array yields no grants; more than [`MAX_GRANT_RECORDS`]
/// records fails the whole block.
fn lossy_grants<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Grant>, D::Error> {
    let serde_json::Value::Array(records) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    if records.len() > MAX_GRANT_RECORDS {
        return Err(serde::de::Error::custom("too many grant records"));
    }
    Ok(records
        .into_iter()
        .filter_map(|record| Grant::deserialize(record).ok())
        .filter(Grant::is_well_formed)
        .collect())
}

/// An absent or null bound is open; a supplied but unreadable bound makes the
/// grant malformed, never unbounded.
fn optional_bound<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|raw| {
            DateTime::parse_from_rfc3339(&raw)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|_| serde::de::Error::custom("unreadable ISO-8601 bound"))
        })
        .transpose()
}

#[cfg(test)]
mod tests;
