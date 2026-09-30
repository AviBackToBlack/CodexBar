//! Parsers for Codex trace-log rows. Bodies contain prompts, so every parser
//! works in memory and returns only ids, model names and timestamps.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::core::CodexPriorityTurnMetadata;

const REQUEST_MARKER: &str = "websocket request:";
const EVENT_MARKER: &str = "websocket event:";
const SUBMISSION_MARKER: &str = "Submission sub=Submission {";
const PRIORITY_SUBMISSION_TIER: &str = "service_tier: Some(Some(\"priority\"))";

// Upstream reads these bodies as `[String: Any]` and casts each field with
// `as? String`: only a JSON object is a body, and a field of another type is
// absent rather than an error. The lenient readers below keep that contract,
// so a numeric `model` no longer drops a Priority turn and an array is never
// read positionally as a struct.
#[derive(Deserialize)]
struct RequestBody<'a> {
    #[serde(default, rename = "type", borrow, deserialize_with = "lenient_str")]
    kind: Option<Cow<'a, str>>,
    #[serde(default, borrow, deserialize_with = "lenient_str")]
    service_tier: Option<Cow<'a, str>>,
    #[serde(default, borrow, deserialize_with = "lenient_str")]
    turn_id: Option<Cow<'a, str>>,
    #[serde(default, borrow, deserialize_with = "lenient_str")]
    model: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct EventBody<'a> {
    #[serde(default, rename = "type", borrow, deserialize_with = "lenient_str")]
    kind: Option<Cow<'a, str>>,
    #[serde(default, borrow, deserialize_with = "object_only")]
    response: Option<EventResponse<'a>>,
}

#[derive(Deserialize)]
struct EventResponse<'a> {
    #[serde(default, borrow, deserialize_with = "lenient_str")]
    model: Option<Cow<'a, str>>,
}

/// A JSON string, or `None` for any other value (upstream `as? String`).
fn lenient_str<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Cow<'de, str>>, D::Error> {
    deserializer.deserialize_any(LenientStr)
}

/// A JSON object read as `T`, or `None` for any other value (upstream
/// `as? [String: Any]`).
fn object_only<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    deserializer.deserialize_any(ObjectOnly(PhantomData))
}

struct LenientStr;

impl<'de> Visitor<'de> for LenientStr {
    type Value = Option<Cow<'de, str>>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Some(Cow::Borrowed(value)))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Some(Cow::Owned(value.to_owned())))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(Some(Cow::Owned(value)))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        skip_seq(seq)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }
}

struct ObjectOnly<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectOnly<T> {
    type Value = Option<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        T::deserialize(de::value::MapAccessDeserializer::new(map)).map(Some)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        skip_seq(seq)
    }
}

fn skip_seq<'de, A: SeqAccess<'de>, T>(mut seq: A) -> Result<Option<T>, A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(None)
}

/// The JSON object after a log marker; any other JSON value is not a body.
fn object_after<'a, T: Deserialize<'a>>(body: &'a str, marker_end: usize) -> Option<T> {
    let json = body[marker_end..].trim();
    if !json.starts_with('{') {
        return None;
    }
    serde_json::from_str(json).ok()
}

/// A `response.completed` event: the turn and the model that served it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompletedTrace {
    pub(super) turn_id: String,
    pub(super) model: String,
}

/// Priority evidence from one trace row: either a `response.create` request
/// with `service_tier == "priority"` or a Priority `Submission` row.
pub(super) fn parse_priority_trace_row(
    timestamp: Option<i64>,
    body: &str,
) -> Option<CodexPriorityTurnMetadata> {
    let Some(marker) = body.find(REQUEST_MARKER) else {
        return parse_priority_submission_row(timestamp, body);
    };
    let prefix = &body[..marker];
    let request: RequestBody<'_> = object_after(body, marker + REQUEST_MARKER.len())?;
    if request.kind.as_deref() != Some("response.create")
        || request.service_tier.as_deref() != Some("priority")
    {
        return None;
    }
    let turn_id = value_named("turn.id", prefix)
        .or_else(|| value_named("turn_id", prefix))
        .or(request.turn_id.as_deref())
        .filter(|id| !id.is_empty())?;
    Some(CodexPriorityTurnMetadata {
        thread_id: value_named("thread_id", prefix).map(str::to_string),
        turn_id: turn_id.to_string(),
        model: request.model.map(Cow::into_owned),
        timestamp,
    })
}

fn parse_priority_submission_row(
    timestamp: Option<i64>,
    body: &str,
) -> Option<CodexPriorityTurnMetadata> {
    if !body.contains(PRIORITY_SUBMISSION_TIER) {
        return None;
    }
    let submission = body.find(SUBMISSION_MARKER)?;
    let tail = &body[submission + SUBMISSION_MARKER.len()..];
    let turn_id = quoted_value_named("id", tail)?;
    Some(CodexPriorityTurnMetadata {
        thread_id: value_named("thread_id", &body[..submission]).map(str::to_string),
        turn_id: turn_id.to_string(),
        model: None,
        timestamp,
    })
}

/// A `response.completed` websocket event with a turn id and response model.
pub(super) fn parse_completed_trace_row(body: &str) -> Option<CompletedTrace> {
    let marker = body.find(EVENT_MARKER)?;
    let prefix = &body[..marker];
    let event: EventBody<'_> = object_after(body, marker + EVENT_MARKER.len())?;
    if event.kind.as_deref() != Some("response.completed") {
        return None;
    }
    let model = event.response?.model.filter(|model| !model.is_empty())?;
    let turn_id = value_named("turn.id", prefix)
        .or_else(|| value_named("turn_id", prefix))
        .filter(|id| !id.is_empty())?;
    Some(CompletedTrace {
        turn_id: turn_id.to_string(),
        model: model.into_owned(),
    })
}

/// The token after `name=` in a log prefix, up to whitespace or punctuation.
fn value_named<'a>(name: &str, text: &'a str) -> Option<&'a str> {
    let start = text.find(&format!("{name}="))? + name.len() + 1;
    let tail = &text[start..];
    let end = tail
        .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ']' | ')' | '}' | ':'))
        .unwrap_or(tail.len());
    let value = &tail[..end];
    (!value.is_empty()).then_some(value)
}

/// The string after `name: "` up to the closing quote.
fn quoted_value_named<'a>(name: &str, text: &'a str) -> Option<&'a str> {
    let start = text.find(&format!("{name}: \""))? + name.len() + 3;
    let tail = &text[start..];
    let value = &tail[..tail.find('"')?];
    (!value.is_empty()).then_some(value)
}
