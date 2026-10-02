//! Threshold-warning continuity across account-identity gaps.
//!
//! Some sources (Claude CLI / OAuth) alternate between samples that resolve an
//! account and samples that cannot. Keyed independently, each identity would
//! fire the same threshold toast once. This module keeps one warning episode
//! across those gaps (upstream CodexBar 0.66.0, `UsageStore+QuotaWarnings`):
//!
//! - A resolved sample moves any fallback ("unresolved") history for the same
//!   lane onto the account and marks the account as `shared_with_unresolved`.
//! - An unresolved sample with no fallback history yet follows the most recently
//!   known account when the window `resets_at` is unchanged and usage has not
//!   dropped; otherwise it keeps the already-fired thresholds if the account
//!   history has previously joined the fallback, and starts an independent
//!   episode if not.
//!
//! The manager stays provider-agnostic: the caller decides which identities are
//! resolved, unresolved or independent via [`WarningScope`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::{NotificationManager, ThresholdKey};
use crate::core::ProviderId;
use crate::settings::Settings;

/// How a sample's account key relates to identity-gap continuity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningScope {
    /// No gap semantics: the key is used as-is and never merged.
    Independent(String),
    /// A resolved account. `unresolved` is the fallback key whose history this
    /// account absorbs (the same source lane's unresolved scope).
    Resolved { account: String, unresolved: String },
    /// The source could not resolve an account; carries its fallback key.
    Unresolved(String),
}

impl WarningScope {
    pub fn key(&self) -> &str {
        match self {
            Self::Independent(key) | Self::Unresolved(key) => key,
            Self::Resolved { account, .. } => account,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LaneKey {
    provider: ProviderId,
    account: String,
    window: String,
}

impl LaneKey {
    fn new(provider: ProviderId, account: &str, window: &str) -> Self {
        Self {
            provider,
            account: account.to_string(),
            window: window.to_string(),
        }
    }
}

/// Last observation for one (provider, account, window) lane.
#[derive(Debug, Clone)]
struct LaneEpisode {
    resets_at: Option<DateTime<Utc>>,
    used_percent: f64,
    /// Monotonic observation order; the newer episode wins when merging.
    sequence: u64,
    shared_with_unresolved: bool,
}

#[derive(Debug, Default)]
pub(super) struct IdentityGapState {
    last_known: HashMap<ProviderId, String>,
    lanes: HashMap<LaneKey, LaneEpisode>,
    sequence: u64,
}

impl NotificationManager {
    /// Resolve which account key a lane sample should be deduped under.
    ///
    /// Call once per non-informational lane sample, before `check_and_notify` /
    /// `check_session_lane`, and pass the returned key to them.
    pub fn resolve_warning_account(
        &mut self,
        provider: ProviderId,
        scope: &WarningScope,
        window: &str,
        used_percent: f64,
        resets_at: Option<DateTime<Utc>>,
        settings: &Settings,
    ) -> String {
        if matches!(scope, WarningScope::Independent(_)) || !settings.show_notifications {
            return scope.key().to_string();
        }
        let key = match scope {
            WarningScope::Resolved {
                account,
                unresolved,
            } => {
                self.merge_unresolved_into_account(provider, unresolved, account);
                self.identity_gaps
                    .last_known
                    .insert(provider, account.clone());
                account.clone()
            }
            WarningScope::Unresolved(unresolved) => {
                self.unresolved_lane_account(provider, unresolved, window, used_percent, resets_at)
            }
            WarningScope::Independent(key) => key.clone(),
        };
        self.record_lane(provider, &key, window, used_percent, resets_at);
        key
    }

    fn unresolved_lane_account(
        &mut self,
        provider: ProviderId,
        unresolved: &str,
        window: &str,
        used_percent: f64,
        resets_at: Option<DateTime<Utc>>,
    ) -> String {
        let own_lane = LaneKey::new(provider, unresolved, window);
        if self.identity_gaps.lanes.contains_key(&own_lane) {
            return unresolved.to_string();
        }
        let Some(account) = self.identity_gaps.last_known.get(&provider).cloned() else {
            return unresolved.to_string();
        };
        let Some(prior) = self
            .identity_gaps
            .lanes
            .get(&LaneKey::new(provider, &account, window))
            .cloned()
        else {
            return unresolved.to_string();
        };
        let continues_episode = resets_at.is_some()
            && resets_at == prior.resets_at
            && used_percent >= prior.used_percent;
        if continues_episode {
            return account;
        }
        if prior.shared_with_unresolved {
            // A discontinuity may change the key but cannot discard reconciled thresholds.
            self.copy_lane_state(provider, &account, unresolved, window);
            self.identity_gaps.lanes.insert(own_lane, prior);
        }
        unresolved.to_string()
    }

    fn record_lane(
        &mut self,
        provider: ProviderId,
        account: &str,
        window: &str,
        used_percent: f64,
        resets_at: Option<DateTime<Utc>>,
    ) {
        let gaps = &mut self.identity_gaps;
        gaps.sequence += 1;
        let lane = gaps
            .lanes
            .entry(LaneKey::new(provider, account, window))
            .or_insert_with(|| LaneEpisode {
                resets_at,
                used_percent,
                sequence: 0,
                shared_with_unresolved: false,
            });
        lane.resets_at = resets_at;
        lane.used_percent = used_percent;
        lane.sequence = gaps.sequence;
    }

    /// Move every lane's fired-threshold state from the fallback key onto the
    /// account, keeping whichever episode was observed most recently.
    fn merge_unresolved_into_account(
        &mut self,
        provider: ProviderId,
        unresolved: &str,
        account: &str,
    ) {
        let windows: Vec<String> = self
            .identity_gaps
            .lanes
            .keys()
            .filter(|lane| lane.provider == provider && lane.account == unresolved)
            .map(|lane| lane.window.clone())
            .collect();
        for window in windows {
            let Some(prior) = self
                .identity_gaps
                .lanes
                .remove(&LaneKey::new(provider, unresolved, &window))
            else {
                continue;
            };
            let account_lane = LaneKey::new(provider, account, &window);
            let account_sequence = self
                .identity_gaps
                .lanes
                .get(&account_lane)
                .map(|lane| lane.sequence);
            if account_sequence.is_none_or(|sequence| prior.sequence >= sequence) {
                self.copy_lane_state(provider, unresolved, account, &window);
                self.identity_gaps.lanes.insert(account_lane.clone(), prior);
            }
            if let Some(lane) = self.identity_gaps.lanes.get_mut(&account_lane) {
                lane.shared_with_unresolved = true;
            }
            self.drop_lane_state(provider, unresolved, &window);
        }
    }

    /// Replace `to`'s fired thresholds (and session-transition baseline) with `from`'s.
    fn copy_lane_state(&mut self, provider: ProviderId, from: &str, to: &str, window: &str) {
        let fired: Vec<ThresholdKey> = self
            .sent_notifications
            .iter()
            .filter(|(p, a, w, _)| *p == provider && a == from && w == window)
            .cloned()
            .collect();
        self.sent_notifications
            .retain(|(p, a, w, _)| *p != provider || a != to || w != window);
        self.sent_notifications.extend(
            fired
                .into_iter()
                .map(|(p, _, w, t)| (p, to.to_string(), w, t)),
        );
        if window == "session" {
            match self
                .previous_session_percent
                .get(&(provider, from.to_string()))
                .copied()
            {
                Some(percent) => {
                    self.previous_session_percent
                        .insert((provider, to.to_string()), percent);
                }
                None => {
                    self.previous_session_percent
                        .remove(&(provider, to.to_string()));
                }
            }
        }
    }

    fn drop_lane_state(&mut self, provider: ProviderId, account: &str, window: &str) {
        self.sent_notifications
            .retain(|(p, a, w, _)| *p != provider || a != account || w != window);
        if window == "session" {
            self.previous_session_percent
                .remove(&(provider, account.to_string()));
        }
    }
}

#[cfg(test)]
mod tests;
