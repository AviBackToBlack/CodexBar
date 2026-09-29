//! Fixed-text explanation for why live Antigravity usage fell back to offline
//! conversation history.
//!
//! The reason is derived only from the typed [`ProviderError`] variant. The
//! error's message is never read: it can embed local paths, URLs, response
//! bodies or account details, so it must not reach a display surface.

use crate::core::{ProviderDisplayDetail, ProviderError};

use super::AGY_NOT_FOUND_MESSAGE;

const DETAIL_ID: &str = "antigravity-live-unavailable";
const DETAIL_TITLE: &str = "Live usage";
const DETAIL_PREFIX: &str = "Live Antigravity usage is unavailable; showing offline data.";
const GENERIC_HINT: &str = "check Diagnostics for per-source details";

/// Detail row explaining the failure that led to the offline snapshot.
///
/// `AuthRequired` is terminal and never reaches the offline snapshot, so it
/// yields no row.
pub(super) fn live_unavailable_detail(error: &ProviderError) -> Option<ProviderDisplayDetail> {
    let reason = match error {
        ProviderError::AuthRequired => return None,
        ProviderError::NotInstalled(_) => AGY_NOT_FOUND_MESSAGE,
        ProviderError::Timeout => "the quota request timed out",
        _ => GENERIC_HINT,
    };
    ProviderDisplayDetail::new(DETAIL_ID, DETAIL_TITLE, format!("{DETAIL_PREFIX} {reason}"))
}
