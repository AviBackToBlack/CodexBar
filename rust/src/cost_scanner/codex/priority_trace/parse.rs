//! Parsers for Codex trace-log rows. Bodies contain prompts, so every parser
//! works in memory and returns only ids, model names and timestamps.

use std::borrow::Cow;

use serde::Deserialize;

use crate::core::CodexPriorityTurnMetadata;

const REQUEST_MARKER: &str = "websocket request:";
const EVENT_MARKER: &str = "websocket event:";
const SUBMISSION_MARKER: &str = "Submission sub=Submission {";
const PRIORITY_SUBMISSION_TIER: &str = "service_tier: Some(Some(\"priority\"))";

#[derive(Deserialize)]
struct RequestBody<'a> {
    #[serde(default, rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    service_tier: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    turn_id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    model: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct EventBody<'a> {
    #[serde(default, rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    response: Option<EventResponse<'a>>,
}

#[derive(Deserialize)]
struct EventResponse<'a> {
    #[serde(default, borrow)]
    model: Option<Cow<'a, str>>,
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
    let json = body[marker + REQUEST_MARKER.len()..].trim();
    let request: RequestBody<'_> = serde_json::from_str(json).ok()?;
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
    let json = body[marker + EVENT_MARKER.len()..].trim();
    let event: EventBody<'_> = serde_json::from_str(json).ok()?;
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
