//! Proof of which live session supplied a cached provider snapshot.
//!
//! A provider that reads a balance through a browser session can attach a
//! [`LastGoodOwner`] to its fetch result. The shell keeps that owner in memory
//! next to the cached snapshot and retains the snapshot through a transport
//! failure only when the failed request came from the same owner. The owner is
//! a salted digest of the session scope and secret, is never serialized, and
//! never appears in `Debug` output.

use sha2::{Digest, Sha256};

use super::ProviderError;

const OWNER_NAMESPACE: &str = "com.codexbar.last-good-owner.v1";

/// Opaque, in-memory identity of the session that produced a snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct LastGoodOwner(String);

impl LastGoodOwner {
    /// Derive an owner from a provider namespace, a scope such as a browser
    /// profile id, and the session secret. Returns `None` when the scope or the
    /// secret is empty after trimming, so an unidentified session cannot own a
    /// snapshot.
    pub fn derive(namespace: &str, scope: &str, secret: &str) -> Option<Self> {
        let scope = scope.trim();
        let secret = secret.trim();
        if scope.is_empty() || secret.is_empty() {
            return None;
        }
        let mut hasher = Sha256::new();
        for part in [OWNER_NAMESPACE, namespace, scope, secret] {
            hasher.update(part.as_bytes());
            hasher.update([0u8]);
        }
        let digest = hasher.finalize();
        Some(Self(
            digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }
}

impl std::fmt::Debug for LastGoodOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LastGoodOwner(<redacted>)")
    }
}

/// Which session a failed refresh came from, as far as the provider can prove.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FailureOwnership {
    /// The error carries no session identity; the provider's policy decides.
    #[default]
    Unchecked,
    /// The failed request used exactly this session.
    Owned(LastGoodOwner),
    /// The failure has no attributable session (for example a resolution
    /// deadline, or a session already rejected). Retention must fail closed.
    Unowned,
}

impl FailureOwnership {
    /// Whether a cached snapshot owned by `cached` may be shown in place of
    /// this failure.
    pub fn allows_retention(&self, cached: Option<&LastGoodOwner>) -> bool {
        match self {
            Self::Unchecked => true,
            Self::Owned(owner) => cached == Some(owner),
            Self::Unowned => false,
        }
    }
}

impl ProviderError {
    /// Wrap a transport failure with the session that produced it.
    ///
    /// A failure that is not a transport failure is returned unchanged, because
    /// only transport failures are eligible for owner-checked retention.
    pub fn with_failure_owner(self, owner: Option<LastGoodOwner>) -> Self {
        if self.is_transport_failure() {
            Self::OwnedTransport {
                owner,
                source: Box::new(self),
            }
        } else {
            self
        }
    }

    /// Session ownership of this failure for last-good retention.
    pub fn failure_ownership(&self) -> FailureOwnership {
        match self {
            Self::OwnedTransport {
                owner: Some(owner), ..
            } => FailureOwnership::Owned(owner.clone()),
            Self::OwnedTransport { owner: None, .. } => FailureOwnership::Unowned,
            _ => FailureOwnership::Unchecked,
        }
    }

    /// The underlying error with any session-ownership wrapper removed.
    pub fn without_failure_owner(&self) -> &Self {
        match self {
            Self::OwnedTransport { source, .. } => source.without_failure_owner(),
            other => other,
        }
    }
}

#[cfg(test)]
#[path = "last_good_owner_tests.rs"]
mod tests;
