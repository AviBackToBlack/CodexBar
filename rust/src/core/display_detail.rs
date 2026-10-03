//! Transient provider detail rows for display surfaces.
//!
//! These rows are intentionally separate from quota windows and inventory:
//! providers may report credit balances, subscription metadata, or other
//! values that must be shown without becoming quota math or persisted core
//! fetch state. Rows are validated once at construction and never re-checked
//! on the read path; the desktop bridge may export them as part of its
//! current display snapshot.
//!
//! Safety boundary: providers are trusted for display text the same way they
//! are trusted for `RateWindow` descriptions, `source_label`, and error
//! strings, which are all exported without a content scan. Construction
//! enforces only shape: non-empty, length-bounded, no control characters.

use crate::core::ProviderFetchResult;

/// One transient provider detail row for display surfaces.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderDisplayDetail {
    id: String,
    section_title: Option<String>,
    title: String,
    value: String,
    secondary_value: Option<String>,
    progress: Option<ProviderDisplayProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProviderDisplayProgress {
    used: f64,
    total: f64,
}

impl ProviderDisplayDetail {
    /// Build a validated row: every text field non-empty, within its length
    /// bound, and free of control characters. Returns `None` on violation.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        value: impl Into<String>,
    ) -> Option<Self> {
        let row = Self {
            id: id.into(),
            section_title: None,
            title: title.into(),
            value: value.into(),
            secondary_value: None,
            progress: None,
        };
        let valid = is_display_shape(&row.id, 64)
            && Self::is_valid_title(&row.title)
            && is_display_shape(&row.value, 512);
        valid.then_some(row)
    }

    /// Attach an optional display section title; rejects invalid text.
    pub fn with_section_title(mut self, title: impl Into<String>) -> Option<Self> {
        let title = title.into();
        if !is_display_shape(&title, 128) {
            return None;
        }
        self.section_title = Some(title);
        Some(self)
    }

    /// Attach a validated secondary value; rejects invalid text.
    pub fn with_secondary_value(mut self, value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if !is_display_shape(&value, 512) {
            return None;
        }
        self.secondary_value = Some(value);
        Some(self)
    }

    /// Whether `title` would be accepted as a row title, for providers that
    /// must reject a whole payload when any wire-supplied label is unusable.
    pub fn is_valid_title(title: &str) -> bool {
        is_display_shape(title, 128)
    }

    /// Attach a progress bar; rejects non-finite or non-positive ranges.
    pub fn with_progress(mut self, used: f64, total: f64) -> Option<Self> {
        if !(used.is_finite() && total.is_finite() && used >= 0.0 && total > 0.0) {
            return None;
        }
        self.progress = Some(ProviderDisplayProgress { used, total });
        Some(self)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn section_title(&self) -> Option<&str> {
        self.section_title.as_deref()
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn secondary_value(&self) -> Option<&str> {
        self.secondary_value.as_deref()
    }

    pub fn progress(&self) -> Option<ProviderDisplayProgress> {
        self.progress
    }
}

impl ProviderDisplayProgress {
    pub fn used(&self) -> f64 {
        self.used
    }

    pub fn total(&self) -> f64 {
        self.total
    }
}

/// Display shape: non-empty, length-bounded, no control characters.
fn is_display_shape(value: &str, max_len: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max_len && value.chars().all(|c| !c.is_control())
}

impl ProviderFetchResult {
    /// Attach one validated transient detail row. Invalid rows are rejected
    /// here, at the construction boundary, not filtered on read.
    pub fn with_display_detail(mut self, detail: Option<ProviderDisplayDetail>) -> Self {
        if let Some(detail) = detail
            && !self.display_details.iter().any(|row| row.id == detail.id)
        {
            self.display_details.push(detail);
        }
        self
    }

    /// Attach several validated rows in order; see [`Self::with_display_detail`].
    pub fn with_display_details(
        self,
        details: impl IntoIterator<Item = ProviderDisplayDetail>,
    ) -> Self {
        details.into_iter().fold(self, |result, detail| {
            result.with_display_detail(Some(detail))
        })
    }

    pub fn display_details(&self) -> &[ProviderDisplayDetail] {
        &self.display_details
    }
}
