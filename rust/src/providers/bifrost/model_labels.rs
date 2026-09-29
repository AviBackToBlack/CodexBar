//! Display formatting for Bifrost per-model spend rows, matching the tagged
//! upstream `modelName()` and `tokenCount()` helpers.

const REGIONS: [&str; 5] = ["us-gov", "us", "eu", "apac", "global"];
const VENDORS: [&str; 13] = [
    "ai21",
    "amazon",
    "anthropic",
    "cohere",
    "deepseek",
    "luma",
    "meta",
    "mistral",
    "openai",
    "qwen",
    "stability",
    "twelvelabs",
    "writer",
];

/// Strip a leading region and vendor token, a Bedrock `-vN:N` revision, and a
/// trailing date so a versioned gateway model id reads as its short name.
pub(super) fn model_name(raw: &str) -> String {
    let name = strip_dotted_prefix(raw, &REGIONS);
    let name = strip_dotted_prefix(name, &VENDORS);
    let name = strip_revision(name);
    let name = strip_date(name);
    let name = name.trim_end_matches([' ', '\t', '-']);
    if name.is_empty() { raw } else { name }.to_owned()
}

fn strip_dotted_prefix<'a>(value: &'a str, tokens: &[&str]) -> &'a str {
    for token in tokens {
        if let Some(head) = value.get(..token.len())
            && head.eq_ignore_ascii_case(token)
            && let Some(rest) = value[token.len()..].strip_prefix('.')
        {
            return rest;
        }
    }
    value
}

/// Remove a trailing `-v<digits>:<digits>`.
fn strip_revision(value: &str) -> &str {
    let Some((head, tail)) = value.rsplit_once("-v") else {
        return value;
    };
    match tail.split_once(':') {
        Some((major, minor)) if all_digits(major) && all_digits(minor) => head,
        _ => value,
    }
}

/// Remove a trailing `-`/whitespace separator plus `YYYYMMDD` or `YYYY-MM-DD`.
fn strip_date(value: &str) -> &str {
    let bytes = value.as_bytes();
    for len in [10usize, 8] {
        let Some(start) = bytes.len().checked_sub(len + 1) else {
            continue;
        };
        let (separator, date) = (bytes[start], &bytes[start + 1..]);
        let dated = if len == 10 {
            date[4] == b'-'
                && date[7] == b'-'
                && [0, 1, 2, 3, 5, 6, 8, 9]
                    .iter()
                    .all(|&i| date[i].is_ascii_digit())
        } else {
            date.iter().all(u8::is_ascii_digit)
        };
        if dated && (separator == b'-' || separator.is_ascii_whitespace()) {
            return &value[..start];
        }
    }
    value
}

fn all_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Compact token count: `K`/`M`/`B` above 1,000 / 999,500 / 999,500,000 with
/// one decimal below 10 and no trailing `.0`.
pub(super) fn token_count(value: f64) -> String {
    let magnitude = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    for (threshold, divisor, unit) in [
        (999_500_000.0, 1e9, "B"),
        (999_500.0, 1e6, "M"),
        (1_000.0, 1e3, "K"),
    ] {
        if magnitude >= threshold {
            let scaled = magnitude / divisor;
            let fixed = to_fixed(scaled, usize::from(scaled < 10.0));
            return format!("{sign}{}{unit}", fixed.strip_suffix(".0").unwrap_or(&fixed));
        }
    }
    format!("{value:.0}")
}

/// `Number.prototype.toFixed`: exact ties round up, where Rust's formatter
/// rounds them to even.
fn to_fixed(value: f64, digits: usize) -> String {
    let exact = format!("{value:.40}");
    let fraction = exact.split_once('.').map_or("", |(_, fraction)| fraction);
    let tie = fraction
        .get(digits..)
        .is_some_and(|tail| tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0'));
    let value = if tie { value.next_up() } else { value };
    format!("{value:.digits$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_drop_region_vendor_revision_and_date() {
        for (raw, expected) in [
            (
                "us.anthropic.claude-sonnet-4-20250514-v1:0",
                "claude-sonnet-4",
            ),
            ("EU.Amazon.nova-pro-v1:0", "nova-pro"),
            ("us-gov.anthropic.claude-3-haiku", "claude-3-haiku"),
            ("gpt-4o-2024-08-06", "gpt-4o"),
            ("claude 20250514", "claude"),
            ("gpt-4o", "gpt-4o"),
            ("global.qwen.qwen3-32b", "qwen3-32b"),
            ("usa.model", "usa.model"),
            ("anthropic.", "anthropic."),
            ("-20250514", "-20250514"),
            ("model-v1:", "model-v1:"),
            ("20250514", "20250514"),
        ] {
            assert_eq!(model_name(raw), expected, "{raw}");
        }
    }

    #[test]
    fn token_counts_use_upstream_thresholds_and_half_up_ties() {
        for (value, expected) in [
            (0.0, "0"),
            (999.0, "999"),
            (1_000.0, "1K"),
            (1_250.0, "1.3K"),
            (1_500.0, "1.5K"),
            (12_500.0, "13K"),
            (999_499.0, "999K"),
            (999_500.0, "1M"),
            (2_340_000.0, "2.3M"),
            (999_499_999.0, "999M"),
            (999_500_000.0, "1B"),
            (-1_250.0, "-1.3K"),
        ] {
            assert_eq!(token_count(value), expected, "{value}");
        }
    }
}
