//! SuperGrok reset-coupon parser for the GetRemainingResets gRPC-web endpoint.
//!
//! Independent of the billing response parser: own framing, own protobuf
//! shape, and a fail-closed failure policy so a partial inventory is never
//! published. The token ID stays private to this module and is never attached
//! to a public or persisted snapshot.

use chrono::{DateTime, Utc};

use super::protobuf::{ProtobufField, looks_like_protobuf_payload};
use crate::core::ProviderError;

/// One unused SuperGrok usage-limit reset coupon. The token ID is retained
/// only while parsing and is never attached to a public or persisted snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::providers::grok) struct GrokResetCoupon {
    pub(in crate::providers::grok) token_id: String,
    pub(in crate::providers::grok) granted_at: Option<DateTime<Utc>>,
    pub(in crate::providers::grok) expires_at: DateTime<Utc>,
}

pub(in crate::providers::grok) fn parse_grpc_web_reset_coupons(
    data: &[u8],
    now: DateTime<Utc>,
) -> Result<Vec<GrokResetCoupon>, ProviderError> {
    let payloads = grpc_web_reset_payloads(data)?;

    let mut coupons = Vec::new();
    for payload in payloads {
        parse_reset_coupon_container(&payload, now, &mut coupons)?;
    }
    coupons.sort_by_key(|coupon| coupon.expires_at);
    Ok(coupons)
}

/// One empty unary gRPC-web request/response frame.
const EMPTY_GRPC_WEB_FRAME: [u8; 5] = [0, 0, 0, 0, 0];

fn grpc_web_reset_payloads(data: &[u8]) -> Result<Vec<Vec<u8>>, ProviderError> {
    if data.is_empty() || data == EMPTY_GRPC_WEB_FRAME {
        return Ok(Vec::new());
    }

    // A raw protobuf payload is retained as a compatibility fallback for the
    // captured endpoint fixtures. Valid protobuf keys cannot begin with a
    // gRPC-web data/trailer flag, so a leading 0/0x80 unambiguously selects
    // framed parsing and makes truncated frames fail closed.
    let is_framed = data.first().is_some_and(|flag| *flag == 0 || *flag == 0x80);
    if !is_framed {
        return looks_like_protobuf_payload(data)
            .then(|| vec![data.to_vec()])
            .ok_or_else(|| {
                ProviderError::Parse("Grok reset-credit response had no payload".to_string())
            });
    }

    super::grpc_web_frames(data, on_malformed_frame_fail).and_then(|frames| {
        let mut payloads = Vec::new();
        for (flags, payload) in frames {
            if flags & 0x80 != 0 {
                validate_grpc_web_reset_trailer(payload)?;
            } else {
                payloads.push(payload.to_vec());
            }
        }
        Ok(payloads)
    })
}

fn on_malformed_frame_fail(context: &str) -> Option<ProviderError> {
    Some(ProviderError::Parse(format!(
        "Grok reset-credit gRPC-web frame is {context}"
    )))
}

fn validate_grpc_web_reset_trailer(payload: &[u8]) -> Result<(), ProviderError> {
    let text = std::str::from_utf8(payload).map_err(|_| {
        ProviderError::Parse("Grok reset-credit gRPC-web trailer is not UTF-8".to_string())
    })?;
    let mut grpc_status = None;
    for line in text
        .split(['\r', '\n'])
        .filter(|line| !line.trim().is_empty())
    {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("grpc-status") {
            let status = value.trim().parse::<u16>().map_err(|_| {
                ProviderError::Parse("Grok reset-credit gRPC status is malformed".to_string())
            })?;
            if grpc_status.is_some_and(|previous| previous != status) {
                return Err(ProviderError::Parse(
                    "Grok reset-credit gRPC status is conflicting".to_string(),
                ));
            }
            grpc_status = Some(status);
        }
    }
    let status = grpc_status.ok_or_else(|| {
        ProviderError::Parse("Grok reset-credit gRPC status is missing".to_string())
    })?;
    super::map_grpc_status(status, "Grok reset-credit RPC")
}

fn parse_reset_coupon_container(
    data: &[u8],
    now: DateTime<Utc>,
    coupons: &mut Vec<GrokResetCoupon>,
) -> Result<(), ProviderError> {
    let fields = ProtobufField::fields(data).ok_or_else(|| {
        ProviderError::Parse("Grok reset-credit protobuf is malformed".to_string())
    })?;
    for field in fields.iter().filter(|field| field.number == 10) {
        let record = field.message().ok_or_else(|| {
            ProviderError::Parse("Grok reset-credit record has an invalid wire type".to_string())
        })?;
        if let Some(coupon) = parse_reset_coupon(record, now)? {
            coupons.push(coupon);
        }
    }
    Ok(())
}

fn parse_reset_coupon(
    data: &[u8],
    now: DateTime<Utc>,
) -> Result<Option<GrokResetCoupon>, ProviderError> {
    let fields = ProtobufField::fields(data)
        .ok_or_else(|| ProviderError::Parse("Grok reset-credit record is malformed".to_string()))?;
    let mut token_id = None;
    let mut granted_at = None;
    let mut expires_at = None;
    for field in &fields {
        match field.number {
            10 => {
                let bytes = field.message().ok_or_else(|| {
                    ProviderError::Parse(
                        "Grok reset-credit token id has an invalid wire type".to_string(),
                    )
                })?;
                token_id = Some(
                    std::str::from_utf8(bytes)
                        .map_err(|_| {
                            ProviderError::Parse(
                                "Grok reset-credit token id is not UTF-8".to_string(),
                            )
                        })?
                        .to_string(),
                );
            }
            20 | 30 => {
                let message = field.message().ok_or_else(|| {
                    ProviderError::Parse(
                        "Grok reset-credit timestamp has an invalid wire type".to_string(),
                    )
                })?;
                let timestamp = parse_timestamp_message(message)?;
                if field.number == 20 {
                    granted_at = timestamp;
                } else {
                    expires_at = timestamp;
                }
            }
            _ => {}
        }
    }

    let Some(token_id) = token_id.filter(|id| !id.trim().is_empty()) else {
        return Ok(None);
    };
    let Some(expires_at) = expires_at.filter(|expires_at| *expires_at > now) else {
        return Ok(None);
    };
    Ok(Some(GrokResetCoupon {
        token_id,
        granted_at,
        expires_at,
    }))
}

/// Decode one embedded timestamp message (field 1 varint, Unix seconds).
fn parse_timestamp_message(data: &[u8]) -> Result<Option<DateTime<Utc>>, ProviderError> {
    let fields = ProtobufField::fields(data).ok_or_else(|| {
        ProviderError::Parse("Grok reset-credit timestamp is malformed".to_string())
    })?;
    let mut seconds = None;
    for field in fields.iter().filter(|field| field.number == 1) {
        seconds = Some(field.varint().ok_or_else(|| {
            ProviderError::Parse(
                "Grok reset-credit timestamp seconds has an invalid wire type".to_string(),
            )
        })?);
    }
    Ok(seconds.and_then(super::unix_seconds_timestamp))
}
