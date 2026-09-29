//! Perplexity session-cookie resolution.
//!
//! Mirrors upstream `perplexity.js` (`cookies()`) and `PerplexityCookieHeader`:
//! a raw value is turned into the exact `Cookie` header values worth sending,
//! in the order they should be tried.

use std::collections::BTreeMap;

use crate::providers::normalize_cookie_header;

/// Session cookie names Perplexity uses, in upstream's preference order.
pub(super) const SESSION_COOKIE_NAMES: [&str; 4] = [
    "__Secure-authjs.session-token",
    "authjs.session-token",
    "__Secure-next-auth.session-token",
    "next-auth.session-token",
];

/// Largest integer JavaScript represents exactly (`Number.isSafeInteger`).
const MAX_SAFE_INDEX: u64 = (1 << 53) - 1;

struct Pair<'a> {
    lowered_name: String,
    name: &'a str,
    value: &'a str,
}

/// Request cookies to try for `raw`, most specific first.
///
/// - A bare token (no `=` or `;`) is tried under every supported cookie name.
/// - A `Cookie` header yields at most one cookie: the first supported name
///   present, either whole or reassembled from `name.0`, `name.1`, ... chunks.
///   Every other cookie in the header is deliberately dropped.
pub(super) fn request_cookies(raw: &str) -> Vec<String> {
    let text = raw.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return Vec::new();
    }
    if !text.contains('=') && !text.contains(';') {
        return SESSION_COOKIE_NAMES
            .iter()
            .map(|name| format!("{name}={text}"))
            .collect();
    }

    let Some(header) = normalize_cookie_header(text) else {
        return Vec::new();
    };
    let pairs = parse_pairs(&header);
    for expected in SESSION_COOKIE_NAMES {
        let lowered = expected.to_lowercase();
        if let Some(direct) = pairs.iter().find(|pair| pair.lowered_name == lowered) {
            return vec![format!("{}={}", direct.name, direct.value)];
        }
        if let Some(cookie) = reassemble_chunks(&pairs, &lowered) {
            return vec![cookie];
        }
    }
    Vec::new()
}

/// `name=value` pairs with a nonempty name and value. A repeated name (case
/// insensitive) keeps its first position but takes the last value, matching
/// upstream's `Map.set`.
fn parse_pairs(header: &str) -> Vec<Pair<'_>> {
    let mut pairs: Vec<Pair<'_>> = Vec::new();
    for part in header.split(';') {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() || value.is_empty() {
            continue;
        }
        let lowered_name = name.to_lowercase();
        match pairs
            .iter_mut()
            .find(|pair| pair.lowered_name == lowered_name)
        {
            Some(existing) => {
                existing.name = name;
                existing.value = value;
            }
            None => pairs.push(Pair {
                lowered_name,
                name,
                value,
            }),
        }
    }
    pairs
}

/// Join `<expected>.0`, `<expected>.1`, ... into one cookie. Any gap, or a
/// missing chunk zero, rejects the whole cookie.
fn reassemble_chunks(pairs: &[Pair<'_>], lowered_expected: &str) -> Option<String> {
    let prefix = format!("{lowered_expected}.");
    let mut chunks: BTreeMap<u64, &Pair<'_>> = BTreeMap::new();
    for pair in pairs {
        let Some(suffix) = pair.lowered_name.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(index) = chunk_index(suffix) {
            chunks.insert(index, pair);
        }
    }
    let count = u64::try_from(chunks.len()).ok()?;
    if count == 0 || chunks.keys().copied().ne(0..count) {
        return None;
    }

    let first = chunks.values().next()?;
    let base_name = &first.name[..first.name.rfind('.')?];
    let token: String = chunks.values().map(|chunk| chunk.value).collect();
    Some(format!("{base_name}={token}"))
}

/// Chunk suffix as upstream accepts it: `^[+-]?\d+$` naming a nonnegative
/// safe integer (so `-0` and `+1` count, `-1` and `1e3` do not).
fn chunk_index(suffix: &str) -> Option<u64> {
    let digits = suffix.strip_prefix(['+', '-']).unwrap_or(suffix);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let index: u64 = digits.parse().ok()?;
    let negative = suffix.starts_with('-');
    ((!negative || index == 0) && index <= MAX_SAFE_INDEX).then_some(index)
}
