//! Product breakdown decoding on the grok.com gRPC-web billing answer. The
//! live frame and the case list follow upstream `GrokWebBillingProductUsageTests`
//! (v0.69.0).

use chrono::{DateTime, TimeZone, Utc};

use super::*;

/// Live `GetGrokCreditsConfig` frame captured 2026-09-26: 6% used, split into
/// GrokChat 4 and GrokBuild 2 (checked in verbatim upstream).
const LIVE_FRAME_HEX: &str = concat!(
    "000000005f0a5d0d0000c04012001a00220c08a5d2c0d5061088ccb580022a0c08a5c7e5d5061088ccb58002",
    "3a07080415000080403a0708021500000040421e0802120c08a5d2c0d5061088ccb580021a0c08a5c7e5d506",
    "1088ccb58002580162006801800000000f677270632d7374617475733a300d0a",
);

const PERIOD_START: u64 = 1_789_929_765;
const PERIOD_END: u64 = 1_790_534_565;

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_790_456_400, 0).single().unwrap()
}

fn product(name: &str, used_percent: f64) -> GrokProductUsage {
    GrokProductUsage {
        product: name.to_string(),
        used_percent,
    }
}

fn chat_and_build() -> Vec<GrokProductUsage> {
    vec![product("GrokChat", 4.0), product("GrokBuild", 2.0)]
}

fn parse(data: &[u8]) -> GrokBillingSnapshot {
    parse_grpc_web_response_at(data, now()).unwrap()
}

pub(in crate::providers::grok) fn live_frame() -> Vec<u8> {
    hex_bytes(LIVE_FRAME_HEX)
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    let digits = hex.as_bytes();
    assert!(
        digits.len().is_multiple_of(2),
        "hex fixture length must be even"
    );
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let low = u8::try_from(value & 0x7f).unwrap();
        value >>= 7;
        if value == 0 {
            bytes.push(low);
            return bytes;
        }
        bytes.push(low | 0x80);
    }
}

fn varint_field(number: u64, value: u64) -> Vec<u8> {
    let mut bytes = varint(number << 3);
    bytes.extend(varint(value));
    bytes
}

fn fixed32(number: u64, value: f32) -> Vec<u8> {
    let mut bytes = varint((number << 3) | 5);
    bytes.extend(value.to_le_bytes());
    bytes
}

fn message(number: u64, value: &[u8]) -> Vec<u8> {
    let mut bytes = varint((number << 3) | 2);
    bytes.extend(varint(u64::try_from(value.len()).unwrap()));
    bytes.extend(value);
    bytes
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend(payload);
    bytes
}

fn entry(id: Option<u64>, percent: Option<f32>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(id) = id {
        bytes.extend(varint_field(1, id));
    }
    if let Some(percent) = percent {
        bytes.extend(fixed32(2, percent));
    }
    bytes
}

fn share(id: u64, percent: f32) -> Vec<u8> {
    entry(Some(id), Some(percent))
}

fn payload(aggregate: Option<f32>, entries: &[Vec<u8>], extra: &[u8]) -> Vec<u8> {
    let mut config = Vec::new();
    if let Some(aggregate) = aggregate {
        config.extend(fixed32(1, aggregate));
    }
    config.extend(message(5, &varint_field(1, PERIOD_END)));
    let mut current_period = varint_field(1, 2);
    current_period.extend(message(2, &varint_field(1, PERIOD_START)));
    current_period.extend(message(3, &varint_field(1, PERIOD_END)));
    config.extend(message(8, &current_period));
    for entry in entries {
        config.extend(message(7, entry));
    }
    config.extend(extra);
    message(1, &config)
}

/// `data` must parse to the same billing as `baseline` (which carries no
/// products) except for the product list.
fn expect_same_billing(data: &[u8], baseline: &[u8], products: Vec<GrokProductUsage>) {
    let actual = parse(data);
    let without_products = parse(baseline);
    assert_eq!(actual.used_percent, without_products.used_percent);
    assert_eq!(actual.resets_at, without_products.resets_at);
    assert_eq!(
        actual.used_percent_is_wire_published,
        without_products.used_percent_is_wire_published
    );
    assert_eq!(
        actual.used_percent_is_implicit_zero,
        without_products.used_percent_is_implicit_zero
    );
    assert_eq!(actual.product_usage, products);
}

/// `data` must publish no breakdown. Win-CodexBar's percent parser is stricter
/// than upstream's: a payload whose scan is incomplete also withholds the
/// percent, so it may report 6 or nothing but never an implicit zero.
fn expect_no_breakdown(data: &[u8]) {
    let parsed = parse(data);
    assert!(parsed.product_usage.is_empty());
    assert!(matches!(parsed.used_percent, None | Some(6.0)));
    assert!(!parsed.used_percent_is_implicit_zero);
}

#[test]
fn live_billing_frame_decodes_the_product_breakdown() {
    let parsed = parse(&live_frame());

    assert_eq!(parsed.used_percent, Some(6.0));
    assert!(parsed.used_percent_is_wire_published);
    assert_eq!(parsed.product_usage, chat_and_build());
}

#[test]
fn unnamed_products_keep_only_complete_named_shares() {
    let named = [share(4, 4.0), share(2, 2.0)];
    let baseline = frame(&payload(Some(6.0), &[], &[]));

    let with_busy_unnamed = [named.to_vec(), vec![share(7, 1.0)]].concat();
    expect_same_billing(
        &frame(&payload(Some(6.0), &with_busy_unnamed, &[])),
        &baseline,
        Vec::new(),
    );
    let with_idle_unnamed = [vec![entry(Some(7), None)], named.to_vec()].concat();
    expect_same_billing(
        &frame(&payload(Some(6.0), &with_idle_unnamed, &[])),
        &baseline,
        chat_and_build(),
    );
}

#[test]
fn missing_duplicate_and_malformed_product_ids_drop_the_breakdown() {
    let baseline = frame(&payload(Some(6.0), &[], &[]));
    let invalid_entries: Vec<Vec<Vec<u8>>> = vec![
        vec![entry(None, Some(4.0))],
        vec![share(4, 4.0), share(4, 2.0)],
        // The percent sent as a varint instead of a fixed32.
        vec![
            [entry(Some(4), None), varint_field(2, 1)].concat(),
            share(2, 6.0),
        ],
    ];
    for entries in invalid_entries {
        expect_same_billing(
            &frame(&payload(Some(6.0), &entries, &[])),
            &baseline,
            Vec::new(),
        );
    }
    let malformed_entries: Vec<Vec<Vec<u8>>> = vec![
        // Truncated field 1 varint inside an entry.
        vec![vec![0x08]],
        // The id sent as a fixed32 instead of a varint.
        vec![fixed32(1, 4.0), share(2, 6.0)],
    ];
    for entries in malformed_entries {
        expect_no_breakdown(&frame(&payload(Some(6.0), &entries, &[])));
    }
    // A product entry sent as a varint instead of a message.
    expect_no_breakdown(&frame(&payload(Some(6.0), &[], &varint_field(7, 4))));
}

#[test]
fn omitted_percentages_default_to_zero_and_unknown_entry_fields_are_skipped() {
    let baseline = frame(&payload(Some(6.0), &[], &[]));
    let mut chat = share(4, 6.0);
    chat.extend(varint_field(3, 42));
    chat.extend(message(4, &[0xFF]));
    chat.extend(fixed32(5, 3.0));
    // Field 6, fixed64.
    chat.extend([0x31]);
    chat.extend([0; 8]);

    expect_same_billing(
        &frame(&payload(Some(6.0), &[chat, entry(Some(2), None)], &[])),
        &baseline,
        vec![product("GrokChat", 6.0), product("GrokBuild", 0.0)],
    );
}

#[test]
fn invalid_or_noncomposing_percentages_drop_the_breakdown() {
    let baseline = frame(&payload(Some(6.0), &[], &[]));
    let cases = [
        vec![share(4, -1.0), share(2, 7.0)],
        vec![share(4, f32::NAN), share(2, 2.0)],
        vec![share(2, 2.0)],
    ];
    for entries in cases {
        expect_same_billing(
            &frame(&payload(Some(6.0), &entries, &[])),
            &baseline,
            Vec::new(),
        );
    }
}

#[test]
fn products_require_one_complete_payload_with_a_published_config_aggregate() {
    let named = [share(4, 4.0), share(2, 2.0)];
    let baseline = frame(&payload(Some(6.0), &[], &[]));
    expect_same_billing(&baseline, &baseline, Vec::new());

    // An implicit zero publishes no percent, so shares have nothing to compose.
    let implicit_baseline = frame(&payload(None, &[], &[]));
    let implicit_zero = frame(&payload(None, &[entry(Some(4), None)], &[]));
    let implicit = parse(&implicit_baseline);
    assert_eq!(implicit.used_percent, Some(0.0));
    assert!(implicit.used_percent_is_implicit_zero);
    expect_same_billing(&implicit_zero, &implicit_baseline, Vec::new());

    let two_frames = [
        frame(&payload(Some(6.0), &named, &[])),
        frame(&payload(Some(6.0), &[], &[])),
    ]
    .concat();
    let two_frames_baseline = [baseline.clone(), baseline.clone()].concat();
    expect_same_billing(&two_frames, &two_frames_baseline, Vec::new());

    // A percent nested under another message is not the config aggregate.
    let nested_aggregate = message(2, &fixed32(1, 6.0));
    expect_same_billing(
        &frame(&payload(None, &named, &nested_aggregate)),
        &frame(&payload(None, &[], &nested_aggregate)),
        Vec::new(),
    );

    // Malformed bytes elsewhere in the payload make the whole scan incomplete.
    let malformed_other_field = [0x62, 0x02, 0x08];
    expect_same_billing(
        &frame(&payload(Some(6.0), &named, &malformed_other_field)),
        &frame(&payload(Some(6.0), &[], &malformed_other_field)),
        Vec::new(),
    );
}

#[test]
fn duplicate_scalar_fields_cannot_relabel_or_reweight_product_shares() {
    let baseline = frame(&payload(Some(6.0), &[], &[]));
    let duplicated_id = [share(2, 6.0), varint_field(1, 4)].concat();
    let duplicated_percent = [share(4, 1.0), fixed32(2, 6.0)].concat();
    for entry in [duplicated_id, duplicated_percent] {
        expect_same_billing(
            &frame(&payload(Some(6.0), &[entry], &[])),
            &baseline,
            Vec::new(),
        );
    }
    let repeated_aggregate = frame(&payload(Some(6.0), &[share(4, 6.0)], &fixed32(1, 6.0)));
    assert!(parse(&repeated_aggregate).product_usage.is_empty());
}

#[test]
fn compressed_or_reserved_frame_flags_fail_closed() {
    for flag in [1u8, 2, 3, 0x81] {
        let mut data = frame(&payload(Some(6.0), &[share(4, 6.0)], &[]));
        data[0] = flag;
        // 0x81 also reads as a (malformed) raw protobuf key, so it may parse
        // to an unavailable reading instead of an error; either way nothing
        // from the frame is published.
        if let Ok(parsed) = parse_grpc_web_response_at(&data, now()) {
            assert_eq!(parsed.used_percent, None, "flag {flag:#x}");
            assert!(parsed.product_usage.is_empty(), "flag {flag:#x}");
            assert!(!parsed.used_percent_is_implicit_zero, "flag {flag:#x}");
        }
    }
}

#[test]
fn truncated_framing_and_overflowing_product_values_cannot_supply_shares() {
    let valid = frame(&payload(Some(6.0), &[share(4, 6.0)], &[]));
    for suffix in [vec![0], vec![0, 0xFF, 0xFF, 0xFF, 0xFF]] {
        let data = [valid.clone(), suffix].concat();
        assert!(parse_grpc_web_response_at(&data, now()).is_err());
    }

    let mut overflow = vec![0x08];
    overflow.extend([0xFF; 9]);
    overflow.push(0x02);
    let mut oversized_length = vec![0x12];
    oversized_length.extend(varint(u64::MAX));
    for entry in [overflow, oversized_length] {
        expect_no_breakdown(&frame(&payload(Some(6.0), &[entry], &[])));
    }
}
