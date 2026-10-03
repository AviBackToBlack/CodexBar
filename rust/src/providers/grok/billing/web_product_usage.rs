//! Per-product credit shares carried by the grok.com `GetGrokCreditsConfig`
//! gRPC-web answer, as repeated `[1, 7]` entries `{1: product id, 2: float
//! percent}` (an omitted percent means 0). Follows upstream
//! `GrokWebBillingFetcher.decodeProductUsage` (v0.69.0).
//!
//! Decoding is all-or-nothing: any malformed or duplicate entry drops the
//! whole list so a partial list can never pass the composition check as if it
//! were complete.

use std::collections::HashSet;

use super::protobuf::ProtobufField;
use crate::providers::grok::product_usage::GrokProductUsage;

/// Field of the response message that holds the billing config.
const CONFIG_FIELD: u64 = 1;
/// Field of the config message that repeats one entry per product.
const PRODUCT_FIELD: u64 = 7;
/// Field of the config message that carries the aggregate credit percent.
const AGGREGATE_FIELD: u64 = 1;

/// Decode the product shares of one complete response message. Empty unless
/// the message holds exactly one config with exactly one aggregate field.
pub(super) fn decode_product_usage(payload: &[u8]) -> Vec<GrokProductUsage> {
    decode(payload).unwrap_or_default()
}

fn decode(payload: &[u8]) -> Option<Vec<GrokProductUsage>> {
    let root = ProtobufField::fields(payload)?;
    let mut configs = root.iter().filter(|field| field.number == CONFIG_FIELD);
    let config = configs.next()?.message()?;
    if configs.next().is_some() {
        return None;
    }
    let config = ProtobufField::fields(config)?;
    if config
        .iter()
        .filter(|field| field.number == AGGREGATE_FIELD)
        .count()
        != 1
    {
        return None;
    }

    let mut products = Vec::new();
    let mut seen_ids = HashSet::new();
    for field in config.iter().filter(|field| field.number == PRODUCT_FIELD) {
        let (id, percent) = decode_entry(field.message()?)?;
        if !seen_ids.insert(id) {
            return None;
        }
        match product_name(id) {
            Some(name) => products.push(GrokProductUsage {
                product: name.to_string(),
                used_percent: percent,
            }),
            // An unnamed product may only be idle; a nonzero share would make
            // the named shares an incomplete list.
            None if percent > 0.0 => return None,
            None => {}
        }
    }
    Some(products)
}

/// Only product ids verified against live CLI-proxy samples are named.
fn product_name(id: u64) -> Option<&'static str> {
    match id {
        2 => Some("GrokBuild"),
        4 => Some("GrokChat"),
        _ => None,
    }
}

fn decode_entry(entry: &[u8]) -> Option<(u64, f64)> {
    let fields = ProtobufField::fields(entry)?;
    let mut ids = fields.iter().filter(|field| field.number == 1);
    let id = ids.next()?.varint()?;
    if ids.next().is_some() {
        return None;
    }
    let mut percentages = fields.iter().filter(|field| field.number == 2);
    let percent = match percentages.next() {
        Some(field) => f64::from(field.fixed32()?),
        None => 0.0,
    };
    if percentages.next().is_some() || !percent.is_finite() || percent < 0.0 {
        return None;
    }
    Some((id, percent))
}
