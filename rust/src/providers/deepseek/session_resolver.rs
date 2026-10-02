//! Choose the Chrome session that supplies the DeepSeek Platform balance, and
//! decide which session a failed request belongs to.
//!
//! Every candidate is validated against the platform (a rejected session is
//! remembered for [`VALIDITY_TTL`]). The selected session's balance is always
//! fetched live. A transport failure keeps the identity of the session it came
//! from, so the shell can keep showing the last balance only when that same
//! session (same profile, same token) produced it. A session the platform has
//! rejected never keeps that authority.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::future::join_all;

use super::BalanceResponse;
use super::chrome_session::{ProfileSelection, TokenInfo};
use crate::core::{LastGoodOwner, ProviderError};

/// How long a validation result is trusted before the platform is asked again.
pub(super) const VALIDITY_TTL: Duration = Duration::from_secs(30 * 60);

const OWNER_NAMESPACE: &str = "deepseek-platform-balance";

/// Owner of a balance fetched with `token` from Chrome profile `profile_id`.
pub(super) fn balance_owner(profile_id: &str, token: &str) -> Option<LastGoodOwner> {
    LastGoodOwner::derive(OWNER_NAMESPACE, profile_id, token)
}

/// Outcome of asking the platform about one candidate.
enum Validation {
    Valid(Box<BalanceResponse>),
    Rejected,
    Unavailable(Option<ProviderError>),
}

struct Outcome {
    index: usize,
    validation: Validation,
}

/// Result of resolving the selected Chrome session.
#[derive(Debug)]
pub(super) enum Resolution {
    Balance {
        balance: Box<BalanceResponse>,
        owner: Option<LastGoodOwner>,
    },
    /// No profile holds a usable DeepSeek session.
    SessionRequired,
    /// Several profiles hold a session and none was selected.
    SelectionRequired(Vec<String>),
    Failed(ProviderError),
}

#[derive(Clone, Copy)]
struct Lookup {
    fresh: Option<bool>,
    last_known: Option<bool>,
}

struct CacheEntry {
    proof: Option<LastGoodOwner>,
    status: bool,
    checked_at: Instant,
}

/// Remembers which sessions the platform accepted or rejected. Holds a digest
/// of each token, never the token.
pub(super) struct ValidationCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl ValidationCache {
    pub(super) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lookup(&self, candidate: &TokenInfo, now: Instant) -> Lookup {
        let proof = balance_owner(&candidate.id, &candidate.token);
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get(&candidate.id) {
            Some(entry) if entry.proof == proof => Lookup {
                fresh: (now.saturating_duration_since(entry.checked_at) < self.ttl)
                    .then_some(entry.status),
                last_known: Some(entry.status),
            },
            _ => Lookup {
                fresh: None,
                last_known: None,
            },
        }
    }

    fn record(&self, candidate: &TokenInfo, status: bool, now: Instant) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(
            candidate.id.clone(),
            CacheEntry {
                proof: balance_owner(&candidate.id, &candidate.token),
                status,
                checked_at: now,
            },
        );
    }
}

/// Resolve the session to use and fetch its balance through `validate`.
pub(super) async fn resolve<F, Fut>(
    candidates: &[TokenInfo],
    selection: &ProfileSelection,
    cache: &ValidationCache,
    validate: F,
) -> Resolution
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<BalanceResponse, ProviderError>>,
{
    if candidates.is_empty() {
        return Resolution::SessionRequired;
    }

    let now = Instant::now();
    let lookups: Vec<Lookup> = candidates.iter().map(|c| cache.lookup(c, now)).collect();
    let selected_id = selection.profile_id.as_deref();
    let to_validate: Vec<usize> = (0..candidates.len())
        .filter(|&i| lookups[i].fresh.is_none() || Some(candidates[i].id.as_str()) == selected_id)
        .collect();

    let mut outcomes = run_validation(candidates, &to_validate, &validate).await;
    record(cache, candidates, &outcomes, now);
    let mut statuses = resolved_statuses(&lookups, &outcomes);
    let mut valid = valid_indices(&statuses);

    // A status that came only from the cache carries no balance.
    if let Some(index) = selected_candidate(candidates, &valid, selection)
        && !has_balance(&outcomes, index)
    {
        outcomes.extend(run_validation(candidates, &[index], &validate).await);
        record(cache, candidates, &outcomes, now);
        statuses = resolved_statuses(&lookups, &outcomes);
        valid = valid_indices(&statuses);
    }

    let all: Vec<usize> = (0..candidates.len()).collect();
    let unresolved = selected_candidate(candidates, &all, selection);

    if valid.is_empty() {
        let failure =
            unresolved.and_then(|index| take_failure(candidates, &mut outcomes, &statuses, index));
        if let Some(error) = failure {
            return Resolution::Failed(error);
        }
        if outcomes
            .iter()
            .any(|o| matches!(o.validation, Validation::Unavailable(_)))
        {
            return Resolution::Failed(validation_unavailable());
        }
        return Resolution::SessionRequired;
    }

    let Some(selected) = selected_candidate(candidates, &valid, selection) else {
        let rejected = unresolved.is_some_and(|index| statuses[index] == Some(false));
        let failure = if rejected {
            None
        } else {
            unresolved.and_then(|index| take_failure(candidates, &mut outcomes, &statuses, index))
        };
        return match failure {
            Some(error) => Resolution::Failed(error),
            None => Resolution::SelectionRequired(
                valid.iter().map(|&i| candidates[i].id.clone()).collect(),
            ),
        };
    };

    if let Some(balance) = take_balance(&mut outcomes, selected) {
        let candidate = &candidates[selected];
        return Resolution::Balance {
            balance,
            owner: balance_owner(&candidate.id, &candidate.token),
        };
    }
    Resolution::Failed(
        take_failure(candidates, &mut outcomes, &statuses, selected)
            .unwrap_or_else(validation_unavailable),
    )
}

fn validation_unavailable() -> ProviderError {
    ProviderError::Other("Chrome DeepSeek session could not be validated right now".to_string())
}

async fn run_validation<F, Fut>(
    candidates: &[TokenInfo],
    indices: &[usize],
    validate: &F,
) -> Vec<Outcome>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<BalanceResponse, ProviderError>>,
{
    join_all(indices.iter().map(|&index| async move {
        let validation = match validate(candidates[index].token.clone()).await {
            Ok(balance) => Validation::Valid(Box::new(balance)),
            Err(ProviderError::AuthRequired) => Validation::Rejected,
            Err(error) => Validation::Unavailable(Some(error)),
        };
        Outcome { index, validation }
    }))
    .await
}

fn record(cache: &ValidationCache, candidates: &[TokenInfo], outcomes: &[Outcome], now: Instant) {
    for outcome in outcomes {
        match outcome.validation {
            Validation::Valid(_) => cache.record(&candidates[outcome.index], true, now),
            Validation::Rejected => cache.record(&candidates[outcome.index], false, now),
            Validation::Unavailable(_) => {}
        }
    }
}

/// Known status per candidate: a fresh cache entry, overridden by this round's
/// outcomes. An unavailable check falls back to the last known status.
fn resolved_statuses(lookups: &[Lookup], outcomes: &[Outcome]) -> Vec<Option<bool>> {
    let mut statuses: Vec<Option<bool>> = lookups.iter().map(|l| l.fresh).collect();
    for outcome in outcomes {
        match outcome.validation {
            Validation::Valid(_) => statuses[outcome.index] = Some(true),
            Validation::Rejected => statuses[outcome.index] = Some(false),
            Validation::Unavailable(_) => {
                if let Some(known) = lookups[outcome.index].last_known {
                    statuses[outcome.index] = Some(known);
                }
            }
        }
    }
    statuses
}

fn valid_indices(statuses: &[Option<bool>]) -> Vec<usize> {
    statuses
        .iter()
        .enumerate()
        .filter_map(|(index, status)| (*status == Some(true)).then_some(index))
        .collect()
}

/// The candidate the user's selection points at among `pool`. Without an
/// explicit selection only a single candidate is unambiguous.
fn selected_candidate(
    candidates: &[TokenInfo],
    pool: &[usize],
    selection: &ProfileSelection,
) -> Option<usize> {
    match selection.profile_id.as_deref() {
        Some(id) => pool.iter().copied().find(|&i| candidates[i].id == id),
        None => (pool.len() == 1).then(|| pool[0]),
    }
}

fn has_balance(outcomes: &[Outcome], index: usize) -> bool {
    outcomes
        .iter()
        .any(|o| o.index == index && matches!(o.validation, Validation::Valid(_)))
}

fn take_balance(outcomes: &mut [Outcome], index: usize) -> Option<Box<BalanceResponse>> {
    let outcome = outcomes
        .iter_mut()
        .rev()
        .find(|o| o.index == index && matches!(o.validation, Validation::Valid(_)))?;
    match std::mem::replace(&mut outcome.validation, Validation::Unavailable(None)) {
        Validation::Valid(balance) => Some(balance),
        _ => None,
    }
}

/// The latest failed check of `index` as an error. A transport failure carries
/// the session that produced it, except when the platform already rejected
/// that session: an outage after a rejection must not revive the old balance.
fn take_failure(
    candidates: &[TokenInfo],
    outcomes: &mut [Outcome],
    statuses: &[Option<bool>],
    index: usize,
) -> Option<ProviderError> {
    let latest = outcomes.iter_mut().rev().find(|o| o.index == index)?;
    let Validation::Unavailable(error) = &mut latest.validation else {
        return None;
    };
    let error = error.take()?;
    let owner = if statuses[index] == Some(false) {
        None
    } else {
        balance_owner(&candidates[index].id, &candidates[index].token)
    };
    Some(error.with_failure_owner(owner))
}

#[cfg(test)]
#[path = "session_resolver_tests.rs"]
mod tests;
