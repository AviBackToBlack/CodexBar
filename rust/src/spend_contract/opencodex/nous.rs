//! Nous Portal rows in the OpenCodex ledger (upstream 0.68.0 #4008).
//!
//! The Hermes extractor records `provider: "nous"` with the exact inference
//! model id (`anthropic/claude-sonnet-4.6`), whatever vendor it names. Rows
//! are priced by the shared OpenCodex resolver: a custom override, or the
//! exact Nous models.dev entry. The vendor named in the model id never lends
//! its own rates.
//!
//! Ledger convention: `inputTokens` excludes cache reads and writes (a row may
//! carry more cache-read than input tokens). A custom override bills input,
//! cache reads and cache writes as separate lanes, as the upstream Nous tests
//! expect; the catalog path adds both back to form the inclusive prompt size
//! that upstream `providerCostUSD` prices.

pub(super) const SUBSCRIPTION_ID: &str = "nous";
