use super::*;

struct CodexParserState {
    current_model: Option<String>,
    previous_totals: Option<CodexTotals>,
    /// High watermark of observed cumulative totals (never lowered). Used for
    /// Ultra interleaved-lineage containment (issue #2037 Phase 1).
    totals_watermark: Option<CodexTotals>,
    /// Latched once any cumulative component drops below the watermark.
    saw_interleaved_totals: bool,
    records: Vec<CodexUsageRecord>,
    previous_token_timestamp: Option<String>,
    previous_token_timestamp_parsed: Option<DateTime<chrono::FixedOffset>>,
    token_timestamps_monotonic: Option<bool>,
    token_timestamp_comparisons: u64,
    fork_baseline: Option<CodexTotals>,
    fork_baseline_ambiguous: bool,
}

#[derive(Debug, Deserialize)]
struct CodexFastLine<'a> {
    #[serde(rename = "type", borrow)]
    event_type: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
    #[serde(default, borrow)]
    payload: Option<CodexFastPayload<'a>>,
    #[serde(default, borrow)]
    event_msg: Option<CodexFastPayload<'a>>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct CodexFastPayload<'a> {
    #[serde(rename = "type", borrow)]
    payload_type: Option<&'a str>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default, borrow)]
    model_name: Option<&'a str>,
    #[serde(default, borrow)]
    info: Option<CodexFastInfo<'a>>,
    #[serde(default)]
    input_tokens: Option<i32>,
    #[serde(default)]
    cached_input_tokens: Option<i32>,
    #[serde(default)]
    cache_read_input_tokens: Option<i32>,
    #[serde(default)]
    output_tokens: Option<i32>,
    #[serde(default)]
    reasoning_output_tokens: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct CodexFastInfo<'a> {
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default, borrow)]
    model_name: Option<&'a str>,
    #[serde(default)]
    total_token_usage: Option<CodexFastTotals>,
    #[serde(default)]
    last_token_usage: Option<CodexFastTotals>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct CodexFastTotals {
    #[serde(default)]
    input_tokens: i32,
    #[serde(default)]
    cached_input_tokens: Option<i32>,
    #[serde(default)]
    cache_read_input_tokens: Option<i32>,
    #[serde(default)]
    output_tokens: i32,
    #[serde(default)]
    reasoning_output_tokens: Option<i32>,
}

enum CodexFastEvent<'a> {
    TurnContext {
        model: Option<&'a str>,
    },
    TokenCount {
        timestamp: &'a str,
        payload: CodexFastPayload<'a>,
    },
}

impl CodexParserState {
    fn new(initial_model: Option<String>, initial_totals: Option<CodexTotals>) -> Self {
        Self::with_timestamp_state(initial_model, initial_totals, None, None)
    }

    fn with_timestamp_state(
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
    ) -> Self {
        Self::with_timestamp_state_and_fork_mode(
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            false,
        )
    }

    fn with_timestamp_state_and_fork_mode(
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        fork_baseline_mode: bool,
    ) -> Self {
        let previous_token_timestamp_parsed = previous_token_timestamp
            .as_deref()
            .and_then(parse_rfc3339_timestamp);
        let fork_baseline = fork_baseline_mode.then(|| initial_totals.clone()).flatten();
        Self {
            current_model: initial_model,
            previous_totals: initial_totals.clone(),
            totals_watermark: initial_totals,
            saw_interleaved_totals: false,
            records: Vec::new(),
            previous_token_timestamp,
            previous_token_timestamp_parsed,
            // A parser always validates a fresh prefix.  `None` is only an
            // input marker for the legacy-cache path, not an output state.
            token_timestamps_monotonic: Some(token_timestamps_monotonic.unwrap_or(true)),
            token_timestamp_comparisons: 0,
            fork_baseline,
            fork_baseline_ambiguous: false,
        }
    }

    fn process_line(&mut self, line: &str, range: &CostUsageDayRange) {
        let event_candidate = is_candidate_codex_line(line);
        let bare_candidate = !event_candidate && line.contains("\"usage\"");
        if !event_candidate && !bare_candidate {
            return;
        }

        if event_candidate && let Some(event) = parse_codex_fast_event(line) {
            self.process_fast_event(event, range);
            return;
        }

        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            return;
        };

        if bare_candidate {
            if obj.get("type").is_some() {
                return;
            }
            let parsed_timestamp = obj
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_codex_timestamp);
            if let Some(timestamp) = obj.get("timestamp").and_then(Value::as_str) {
                // Timestamp order is a property of the whole native file,
                // including usage records outside the requested day window.
                self.observe_token_timestamp(timestamp, parsed_timestamp.as_ref());
            }
            let day_key = parsed_timestamp
                .as_ref()
                .map(ParsedCodexTimestamp::day_key)
                .filter(|day_key| {
                    CostUsageDayRange::is_in_range(
                        day_key,
                        &range.scan_since_key,
                        &range.scan_until_key,
                    )
                })
                .or_else(|| self.records.last().map(|record| record.day_key.clone()));
            let Some(day_key) = day_key else {
                return;
            };
            if let Some((totals, model)) = bare_usage_totals(&obj) {
                let model = self
                    .current_model
                    .as_deref()
                    .and_then(model_evidence)
                    .or(model.as_deref().and_then(model_evidence))
                    .unwrap_or(CostUsagePricing::CODEX_UNATTRIBUTED_MODEL)
                    .to_string();
                self.record_usage(
                    day_key,
                    &model,
                    totals.input,
                    totals.cached,
                    totals.output,
                    totals.reasoning,
                );
            }
            return;
        }

        let is_token_count = token_count_payload(&obj).is_some();
        let parsed_timestamp = obj
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_codex_timestamp);
        if is_token_count && let Some(timestamp) = obj.get("timestamp").and_then(Value::as_str) {
            // Timestamp order is a property of the whole native file, not
            // only the requested display window. Validate it before the
            // range filter so a cached prefix remains safe to extend.
            self.observe_token_timestamp(timestamp, parsed_timestamp.as_ref());
        }
        let Some(day_key) = parsed_timestamp
            .as_ref()
            .map(ParsedCodexTimestamp::day_key)
            .filter(|day_key| {
                CostUsageDayRange::is_in_range(
                    day_key,
                    &range.scan_since_key,
                    &range.scan_until_key,
                )
            })
        else {
            return;
        };
        if obj.get("type").and_then(|v| v.as_str()) == Some("turn_context") {
            self.update_current_model(&obj);
        }

        if is_token_count {
            self.record_token_count(&obj, day_key);
        }
    }

    fn process_fast_event(&mut self, event: CodexFastEvent<'_>, range: &CostUsageDayRange) {
        match event {
            CodexFastEvent::TurnContext { model } => {
                // Explicit blank model evidence clears stale turn context.
                if let Some(raw) = model {
                    self.current_model = model_evidence(raw).map(str::to_string);
                }
            }
            CodexFastEvent::TokenCount { timestamp, payload } => {
                let parsed_timestamp = parse_codex_timestamp(timestamp);
                self.observe_token_timestamp(timestamp, parsed_timestamp.as_ref());
                let Some(parsed_timestamp) = parsed_timestamp else {
                    return;
                };
                let day_key = parsed_timestamp.day_key();
                if !CostUsageDayRange::is_in_range(
                    &day_key,
                    &range.scan_since_key,
                    &range.scan_until_key,
                ) {
                    return;
                }
                self.record_fast_token_count(payload, day_key);
            }
        }
    }

    fn update_current_model(&mut self, obj: &Value) {
        let candidates = [
            obj.get("model").and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("model"))
                .and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("model_name"))
                .and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("info"))
                .and_then(|info| info.get("model"))
                .and_then(|v| v.as_str()),
            obj.get("payload")
                .and_then(|payload| payload.get("info"))
                .and_then(|info| info.get("model_name"))
                .and_then(|v| v.as_str()),
        ];
        // Only rewrite current_model when the turn_context actually carries a
        // model field (including blank, which clears stale attribution).
        let has_key = candidates.iter().any(|c| c.is_some());
        if !has_key {
            return;
        }
        self.current_model = candidates
            .into_iter()
            .flatten()
            .find_map(model_evidence)
            .map(str::to_string);
    }

    fn record_token_count(&mut self, obj: &Value, day_key: String) {
        let Some(payload) = token_count_payload(obj) else {
            return;
        };
        let Some((delta_input, delta_cached, delta_output, reasoning)) = self.token_deltas(payload)
        else {
            return;
        };
        if delta_input == 0 && delta_cached == 0 && delta_output == 0 {
            return;
        }

        let info = payload.get("info");
        let model = self.resolve_token_model(info, payload, obj);
        self.record_usage(
            day_key,
            &model,
            delta_input,
            delta_cached,
            delta_output,
            reasoning,
        );
    }

    fn record_fast_token_count(&mut self, payload: CodexFastPayload<'_>, day_key: String) {
        let Some((delta_input, delta_cached, delta_output, reasoning)) =
            self.fast_token_deltas(&payload)
        else {
            return;
        };
        if delta_input == 0 && delta_cached == 0 && delta_output == 0 {
            return;
        }

        let event_model = payload
            .info
            .as_ref()
            .and_then(|info| info.model.or(info.model_name))
            .or(payload.model)
            .and_then(model_evidence);
        // Prefer current turn_context model over a conflicting event model,
        // matching upstream precedence. Fall back to unattributed (not gpt-5).
        let model = self
            .current_model
            .as_deref()
            .and_then(model_evidence)
            .or(event_model)
            .unwrap_or(CostUsagePricing::CODEX_UNATTRIBUTED_MODEL)
            .to_string();
        self.record_usage(
            day_key,
            &model,
            delta_input,
            delta_cached,
            delta_output,
            reasoning,
        );
    }

    fn record_usage(
        &mut self,
        day_key: String,
        model: &str,
        input: i32,
        cached: i32,
        output: i32,
        reasoning: Option<i32>,
    ) {
        self.records.push(CodexUsageRecord {
            day_key,
            model: CostUsagePricing::normalize_codex_model(model),
            input,
            cached: cached.min(input),
            output,
            reasoning: clamp_reasoning(reasoning, output),
        });
    }

    fn resolve_token_model(&self, info: Option<&Value>, payload: &Value, obj: &Value) -> String {
        let event_model = info
            .and_then(|i| i.get("model").or(i.get("model_name")))
            .or_else(|| payload.get("model"))
            .or_else(|| obj.get("model"))
            .and_then(|v| v.as_str())
            .and_then(model_evidence);
        self.current_model
            .as_deref()
            .and_then(model_evidence)
            .or(event_model)
            .unwrap_or(CostUsagePricing::CODEX_UNATTRIBUTED_MODEL)
            .to_string()
    }

    fn token_deltas(&mut self, payload: &Value) -> Option<(i32, i32, i32, Option<i32>)> {
        let info = payload.get("info");
        if let Some(total) = info.and_then(|i| i.get("total_token_usage")) {
            return Some(self.total_usage_delta(total));
        }

        if let Some(last) = info.and_then(|i| i.get("last_token_usage")) {
            return Some(last_usage_delta(last));
        }

        let direct = read_token_totals(payload);
        (direct.input != 0 || direct.cached != 0 || direct.output != 0).then_some((
            direct.input.max(0),
            direct.cached.max(0),
            direct.output.max(0),
            direct.reasoning,
        ))
    }

    fn fast_token_deltas(
        &mut self,
        payload: &CodexFastPayload<'_>,
    ) -> Option<(i32, i32, i32, Option<i32>)> {
        if let Some(total) = payload
            .info
            .as_ref()
            .and_then(|info| info.total_token_usage)
        {
            return Some(self.fast_total_usage_delta(total));
        }

        if let Some(last) = payload.info.as_ref().and_then(|info| info.last_token_usage) {
            return Some(fast_last_usage_delta(last));
        }

        let direct = fast_totals_from_payload(payload);
        (direct.input != 0 || direct.cached != 0 || direct.output != 0).then_some((
            direct.input.max(0),
            direct.cached.max(0),
            direct.output.max(0),
            direct.reasoning,
        ))
    }

    fn total_usage_delta(&mut self, total: &Value) -> (i32, i32, i32, Option<i32>) {
        let totals = read_token_totals(total);
        self.apply_totals_delta(totals)
    }

    fn fast_total_usage_delta(&mut self, total: CodexFastTotals) -> (i32, i32, i32, Option<i32>) {
        let totals = codex_totals_from_fast(total);
        self.apply_totals_delta(totals)
    }

    fn apply_totals_delta(&mut self, totals: CodexTotals) -> (i32, i32, i32, Option<i32>) {
        self.latch_if_below_watermark(&totals);

        let delta = if self.saw_interleaved_totals {
            contained_total_delta(
                self.totals_watermark.as_ref(),
                self.previous_totals.as_ref(),
                &totals,
            )
        } else {
            let previous = self.previous_totals.as_ref();
            let input = (totals.input - previous.map_or(0, |t| t.input)).max(0);
            let cached = (totals.cached - previous.map_or(0, |t| t.cached)).max(0);
            let output = (totals.output - previous.map_or(0, |t| t.output)).max(0);
            CodexTotals {
                input,
                cached,
                output,
                reasoning: cumulative_reasoning_delta(previous, totals.reasoning, output),
            }
        };

        self.previous_totals = Some(totals.clone());
        self.raise_watermark(&totals);
        (delta.input, delta.cached, delta.output, delta.reasoning)
    }

    fn observe_token_timestamp(
        &mut self,
        timestamp: &str,
        parsed_timestamp: Option<&ParsedCodexTimestamp>,
    ) {
        let current_parsed = parsed_timestamp
            .map(|parsed| parsed.parsed)
            .unwrap_or_else(|| parse_rfc3339_timestamp(timestamp));
        if let Some(previous) = self.previous_token_timestamp.as_deref()
            && self.token_timestamps_monotonic != Some(false)
        {
            self.token_timestamp_comparisons = self.token_timestamp_comparisons.saturating_add(1);
            let ordered = match (
                self.previous_token_timestamp_parsed.as_ref(),
                current_parsed.as_ref(),
            ) {
                (Some(previous), Some(current)) => previous <= current,
                // A malformed historical timestamp keeps the scanner's
                // existing lexical fallback semantics.  The current parsed
                // value is deliberately not reparsed here.
                _ => previous <= timestamp,
            };
            if !ordered {
                self.token_timestamps_monotonic = Some(false);
            }
        }
        self.previous_token_timestamp = Some(timestamp.to_string());
        self.previous_token_timestamp_parsed = current_parsed;
    }

    fn latch_if_below_watermark(&mut self, totals: &CodexTotals) {
        if let Some(baseline) = self.fork_baseline.as_ref()
            && (totals.input < baseline.input
                || totals.cached < baseline.cached
                || totals.output < baseline.output)
        {
            self.fork_baseline_ambiguous = true;
        }
        let Some(water) = self.totals_watermark.as_ref() else {
            return;
        };
        if totals.input < water.input
            || totals.cached < water.cached
            || totals.output < water.output
        {
            self.saw_interleaved_totals = true;
        }
    }

    fn raise_watermark(&mut self, totals: &CodexTotals) {
        self.totals_watermark = Some(match self.totals_watermark.as_ref() {
            Some(water) => CodexTotals {
                input: water.input.max(totals.input),
                cached: water.cached.max(totals.cached),
                output: water.output.max(totals.output),
                reasoning: match (water.reasoning, totals.reasoning) {
                    (Some(water), Some(current)) => Some(water.max(current)),
                    (Some(water), None) => Some(water),
                    (None, Some(current)) => Some(current),
                    (None, None) => None,
                },
            },
            None => totals.clone(),
        });
    }
}

fn model_evidence(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// When interleaved Ultra lineages reset cumulative counters, only count growth
/// above the historical high watermark so rewound branches do not re-add work.
fn contained_total_delta(
    watermark: Option<&CodexTotals>,
    counted: Option<&CodexTotals>,
    current: &CodexTotals,
) -> CodexTotals {
    let water = watermark.cloned().unwrap_or(CodexTotals {
        input: 0,
        cached: 0,
        output: 0,
        reasoning: None,
    });
    let counted = counted.cloned().unwrap_or(CodexTotals {
        input: 0,
        cached: 0,
        output: 0,
        reasoning: None,
    });

    let component = |water: i32, counted: i32, current: i32| -> i32 {
        if current >= water {
            // Only growth above the historical high watermark counts.
            (current - water.max(counted)).max(0)
        } else {
            // Below watermark: rewind / interleaved lineage — do not re-add
            // mid-range climbs that would inflate totals after a fork reset.
            0
        }
    };

    CodexTotals {
        input: component(water.input, counted.input, current.input),
        cached: component(water.cached, counted.cached, current.cached),
        output: component(water.output, counted.output, current.output),
        reasoning: cumulative_reasoning_delta(
            Some(&counted),
            current.reasoning,
            component(water.output, counted.output, current.output),
        ),
    }
}

fn cumulative_reasoning_delta(
    previous: Option<&CodexTotals>,
    current: Option<i32>,
    output_delta: i32,
) -> Option<i32> {
    let current = current?;
    let previous = match previous {
        Some(previous) => previous.reasoning?,
        None => 0,
    };
    Some(
        current
            .saturating_sub(previous)
            .max(0)
            .min(output_delta.max(0)),
    )
}

/// Read one JSONL line, discarding content when it exceeds `max_bytes`.
/// Returns `(line_without_newline, bytes_consumed_including_newline)`.
fn read_bounded_jsonl_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<(Vec<u8>, usize)>> {
    let mut line = Vec::new();
    let mut saw_bytes = false;
    let mut discarding = false;
    let mut consumed_total = 0;

    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(
                saw_bytes.then_some((if discarding { Vec::new() } else { line }, consumed_total))
            );
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let segment_end = newline.unwrap_or(chunk.len());
        let segment = &chunk[..segment_end];
        saw_bytes = true;

        if !discarding {
            let remaining = max_bytes.saturating_sub(line.len());
            if segment.len() <= remaining {
                line.extend_from_slice(segment);
            } else {
                line.clear();
                discarding = true;
            }
        }

        let consumed = segment_end + usize::from(newline.is_some());
        reader.consume(consumed);
        consumed_total += consumed;
        if newline.is_some() {
            return Ok(Some((
                if discarding { Vec::new() } else { line },
                consumed_total,
            )));
        }
    }
}

fn parse_codex_fast_event(line: &str) -> Option<CodexFastEvent<'_>> {
    let parsed: CodexFastLine<'_> = serde_json::from_str(line).ok()?;
    match parsed.event_type? {
        "turn_context" => {
            let model = parsed
                .payload
                .as_ref()
                .and_then(|payload| {
                    payload.model.or(payload.model_name).or_else(|| {
                        payload
                            .info
                            .as_ref()
                            .and_then(|info| info.model.or(info.model_name))
                    })
                })
                .or(parsed.model);
            Some(CodexFastEvent::TurnContext { model })
        }
        "event_msg" => {
            let payload = parsed.payload.or(parsed.event_msg)?;
            (payload.payload_type == Some("token_count")).then_some(CodexFastEvent::TokenCount {
                timestamp: parsed.timestamp?,
                payload,
            })
        }
        _ => None,
    }
}

fn is_candidate_codex_line(line: &str) -> bool {
    if !line.contains("\"type\":\"event_msg\"")
        && !line.contains("\"type\":\"turn_context\"")
        && !line.contains("\"event_msg\"")
    {
        return false;
    }

    !line.contains("\"type\":\"event_msg\"") || line.contains("\"token_count\"")
}

fn codex_timestamp_day_key(timestamp: &str) -> Option<String> {
    parse_codex_timestamp(timestamp).map(|parsed| parsed.day_key())
}

#[derive(Debug, Clone)]
struct ParsedCodexTimestamp {
    parsed: Option<DateTime<FixedOffset>>,
    fallback_day_key: String,
}

impl ParsedCodexTimestamp {
    fn day_key(&self) -> String {
        self.parsed
            .as_ref()
            .map(|timestamp| {
                timestamp
                    .with_timezone(&Local)
                    .date_naive()
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_else(|| self.fallback_day_key.clone())
    }
}

fn parse_codex_timestamp(timestamp: &str) -> Option<ParsedCodexTimestamp> {
    let bytes = timestamp.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let fallback_day_key = timestamp.get(..10)?;
    NaiveDate::parse_from_str(fallback_day_key, "%Y-%m-%d").ok()?;
    Some(ParsedCodexTimestamp {
        parsed: parse_rfc3339_timestamp(timestamp),
        fallback_day_key: fallback_day_key.to_string(),
    })
}

fn parse_rfc3339_timestamp(timestamp: &str) -> Option<DateTime<FixedOffset>> {
    parse_native_rfc3339(timestamp).or_else(|| DateTime::parse_from_rfc3339(timestamp).ok())
}

fn nonempty_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn session_meta_field(root: &Value, payload: Option<&Value>, keys: &[&str]) -> Option<String> {
    payload
        .and_then(|payload| {
            keys.iter()
                .find_map(|key| nonempty_json_string(payload.get(*key)))
        })
        .or_else(|| {
            keys.iter()
                .find_map(|key| nonempty_json_string(root.get(*key)))
        })
}

/// Fast path for the RFC3339 spelling emitted by native Codex logs. Historical
/// spellings still fall through to chrono's parser, preserving old behavior.
fn parse_native_rfc3339(timestamp: &str) -> Option<DateTime<FixedOffset>> {
    let bytes = timestamp.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let year = parse_ascii_number(bytes, 0, 4)?;
    if year < 1900 {
        return None;
    }
    let month = parse_ascii_number(bytes, 5, 2)?;
    let day = parse_ascii_number(bytes, 8, 2)?;
    let hour = parse_ascii_number(bytes, 11, 2)?;
    let minute = parse_ascii_number(bytes, 14, 2)?;
    let second = parse_ascii_number(bytes, 17, 2)?;
    if !(1..=12).contains(&month) || hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }

    let mut zone_index = 19;
    let mut nanoseconds = 0_u32;
    if bytes.get(zone_index) == Some(&b'.') {
        zone_index += 1;
        let fraction_start = zone_index;
        while bytes
            .get(zone_index)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            let digits = zone_index - fraction_start;
            if digits >= 9 {
                return None;
            }
            if digits < 3 {
                nanoseconds = nanoseconds * 10 + u32::from(bytes[zone_index] - b'0');
            }
            zone_index += 1;
        }
        let digits = zone_index - fraction_start;
        if digits == 0 {
            return None;
        }
        for _ in digits.min(3)..3 {
            nanoseconds *= 10;
        }
        for _ in 0..6 {
            nanoseconds *= 10;
        }
    }

    let offset_seconds = match bytes.get(zone_index) {
        Some(b'Z') if zone_index + 1 == bytes.len() => 0,
        Some(sign) if (*sign == b'+' || *sign == b'-') && zone_index + 6 == bytes.len() => {
            if bytes[zone_index + 3] != b':' {
                return None;
            }
            let hours = parse_ascii_number(bytes, zone_index + 1, 2)?;
            let minutes = parse_ascii_number(bytes, zone_index + 4, 2)?;
            if hours >= 24 || minutes >= 60 {
                return None;
            }
            let seconds = i32::try_from((hours * 60 + minutes) * 60).ok()?;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ => return None,
    };

    let year = i32::try_from(year).ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let local = date.and_hms_nano_opt(hour, minute, second, nanoseconds)?;
    FixedOffset::east_opt(offset_seconds)
        .and_then(|offset| offset.from_local_datetime(&local).single())
}

fn parse_ascii_number(bytes: &[u8], start: usize, count: usize) -> Option<u32> {
    let slice = bytes.get(start..start.checked_add(count)?)?;
    let mut value = 0_u32;
    for byte in slice {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(*byte - b'0');
    }
    Some(value)
}

fn bare_usage_totals(obj: &Value) -> Option<(CodexTotals, Option<String>)> {
    let usage = obj
        .get("usage")
        .or_else(|| obj.get("data").and_then(|v| v.get("usage")))
        .or_else(|| obj.get("result").and_then(|v| v.get("usage")))
        .or_else(|| obj.get("response").and_then(|v| v.get("usage")))?;
    // Token counts come from usage records and fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "usage token counts fit i32, the canonical totals storage type"
    )]
    let input = ["input_tokens", "prompt_tokens", "input"]
        .into_iter()
        .find_map(|key| usage.get(key).and_then(Value::as_i64))
        .unwrap_or(0)
        .max(0) as i32;
    // Token counts come from usage records and fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "usage token counts fit i32, the canonical totals storage type"
    )]
    let output = ["output_tokens", "completion_tokens", "output"]
        .into_iter()
        .find_map(|key| usage.get(key).and_then(Value::as_i64))
        .unwrap_or(0)
        .max(0) as i32;
    // Token counts come from usage records and fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "usage token counts fit i32, the canonical totals storage type"
    )]
    let cached = [
        "cached_input_tokens",
        "cache_read_input_tokens",
        "cached_tokens",
    ]
    .into_iter()
    .filter_map(|key| usage.get(key).and_then(Value::as_i64))
    .max()
    .unwrap_or(0)
    .max(0) as i32;
    let reasoning = clamp_reasoning(optional_token_i32(usage, "reasoning_output_tokens"), output);
    if input == 0 && output == 0 && cached == 0 {
        return None;
    }
    let model = obj
        .get("model")
        .or_else(|| obj.get("data").and_then(|v| v.get("model")))
        .or_else(|| obj.get("result").and_then(|v| v.get("model")))
        .or_else(|| obj.get("response").and_then(|v| v.get("model")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    Some((
        CodexTotals {
            input,
            cached,
            output,
            reasoning,
        },
        model,
    ))
}

fn token_count_payload(obj: &Value) -> Option<&Value> {
    if let Some(payload) = obj.get("payload")
        && payload.get("type").and_then(|v| v.as_str()) == Some("token_count")
    {
        return Some(payload);
    }

    let event_msg = obj.get("event_msg")?;
    (event_msg.get("type").and_then(|v| v.as_str()) == Some("token_count")).then_some(event_msg)
}

fn read_token_totals(value: &Value) -> CodexTotals {
    // Token counts come from Codex usage records and fit within i32, which is
    // the canonical storage type of the totals table.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "token counts from usage records fit i32"
    )]
    let cached = value
        .get("cached_input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(
            value
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        ) as i32;
    CodexTotals {
        input: token_i32(value, "input_tokens"),
        cached,
        output: token_i32(value, "output_tokens"),
        reasoning: clamp_reasoning(
            optional_token_i32(value, "reasoning_output_tokens"),
            token_i32(value, "output_tokens"),
        ),
    }
}

fn codex_totals_from_fast(value: CodexFastTotals) -> CodexTotals {
    CodexTotals {
        input: value.input_tokens,
        cached: value
            .cached_input_tokens
            .unwrap_or(0)
            .max(value.cache_read_input_tokens.unwrap_or(0)),
        output: value.output_tokens,
        reasoning: clamp_reasoning(value.reasoning_output_tokens, value.output_tokens),
    }
}

fn fast_totals_from_payload(value: &CodexFastPayload<'_>) -> CodexTotals {
    CodexTotals {
        input: value.input_tokens.unwrap_or(0),
        cached: value
            .cached_input_tokens
            .unwrap_or(0)
            .max(value.cache_read_input_tokens.unwrap_or(0)),
        output: value.output_tokens.unwrap_or(0),
        reasoning: clamp_reasoning(
            value.reasoning_output_tokens,
            value.output_tokens.unwrap_or(0),
        ),
    }
}

fn token_i32(value: &Value, key: &str) -> i32 {
    // Token counts from usage records fit i32, the canonical totals storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "token counts from usage records fit i32"
    )]
    let tokens = value.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    tokens
}

fn optional_token_i32(value: &Value, key: &str) -> Option<i32> {
    // Token counts from usage records fit i32, the canonical storage type.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "token counts from usage records fit i32"
    )]
    value
        .get(key)
        .and_then(Value::as_i64)
        .map(|tokens| tokens as i32)
}

fn clamp_reasoning(reasoning: Option<i32>, output: i32) -> Option<i32> {
    reasoning.map(|tokens| tokens.max(0).min(output.max(0)))
}

fn last_usage_delta(last: &Value) -> (i32, i32, i32, Option<i32>) {
    let totals = read_token_totals(last);
    (
        totals.input.max(0),
        totals.cached.max(0),
        totals.output.max(0),
        totals.reasoning,
    )
}

fn fast_last_usage_delta(last: CodexFastTotals) -> (i32, i32, i32, Option<i32>) {
    let totals = codex_totals_from_fast(last);
    (
        totals.input.max(0),
        totals.cached.max(0),
        totals.output.max(0),
        totals.reasoning,
    )
}

impl JsonlScanner {
    /// Get default Codex sessions root directory
    pub fn default_codex_sessions_root() -> Option<PathBuf> {
        // Check CODEX_HOME environment variable
        if let Ok(home) = std::env::var("CODEX_HOME") {
            let home = home.trim();
            if !home.is_empty() {
                return Some(PathBuf::from(home).join("sessions"));
            }
        }

        // Default to ~/.codex/sessions
        dirs::home_dir().map(|h| h.join(".codex").join("sessions"))
    }

    /// Get default Claude projects roots
    pub fn default_claude_projects_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();

        // Check CLAUDE_CONFIG_DIR
        if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            let path = PathBuf::from(config_dir.trim()).join("projects");
            if path.exists() {
                roots.push(path);
            }
        }

        // Default locations
        if let Some(home) = dirs::home_dir() {
            let default_path = home.join(".claude").join("projects");
            if default_path.exists() && !roots.contains(&default_path) {
                roots.push(default_path);
            }
        }

        roots
    }

    /// List Codex session files in the given date range
    pub fn list_codex_session_files(
        root: &Path,
        scan_since_key: &str,
        scan_until_key: &str,
    ) -> Vec<PathBuf> {
        let mut files = Vec::new();

        let Some(mut date) = CostUsageDayRange::parse_day_key(scan_since_key) else {
            return files;
        };
        let Some(until_date) = CostUsageDayRange::parse_day_key(scan_until_key) else {
            return files;
        };

        while date <= until_date {
            let year = format!("{:04}", date.year());
            let month = format!("{:02}", date.month());
            let day = format!("{:02}", date.day());

            let day_dir = root.join(&year).join(&month).join(&day);

            if let Ok(entries) = fs::read_dir(&day_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
                    {
                        files.push(path);
                    }
                }
            }

            date += chrono::Duration::days(1);
        }

        files
    }

    /// Read only a bounded prefix until the first authoritative `session_meta`
    /// row is found. Fork decisions must not require parsing the child usage
    /// stream before a safe parent baseline is selected.
    pub(crate) fn read_codex_session_metadata(
        file_path: &Path,
    ) -> std::io::Result<CodexSessionMetadata> {
        let file = File::open(file_path)?;
        let mut reader = BufReader::new(file);
        let mut bytes_examined = 0_usize;

        while bytes_examined < CODEX_JSONL_MAX_LINE_BYTES {
            let Some((line_bytes, consumed)) =
                read_bounded_jsonl_line(&mut reader, CODEX_JSONL_MAX_LINE_BYTES)?
            else {
                break;
            };
            bytes_examined = bytes_examined.saturating_add(consumed);
            if line_bytes.is_empty() {
                continue;
            }
            let Ok(line) = std::str::from_utf8(&line_bytes) else {
                continue;
            };
            let line = line.strip_suffix('\r').unwrap_or(line);
            let Ok(obj) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if obj.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }

            let payload = obj.get("payload").filter(|value| value.is_object());
            return Ok(CodexSessionMetadata {
                session_id: session_meta_field(&obj, payload, &["id", "session_id", "sessionId"]),
                forked_from_id: session_meta_field(
                    &obj,
                    payload,
                    &[
                        "forked_from_id",
                        "forkedFromId",
                        "parent_session_id",
                        "parentSessionId",
                    ],
                ),
                fork_timestamp: nonempty_json_string(obj.get("timestamp")).or_else(|| {
                    payload.and_then(|value| nonempty_json_string(value.get("timestamp")))
                }),
            });
        }

        Ok(CodexSessionMetadata::default())
    }

    /// Compare RFC3339 timestamps using parsed instants. Malformed timestamps
    /// are unsafe for fork-baseline reconciliation and therefore fail closed.
    pub(crate) fn codex_timestamp_at_or_before(earlier: &str, later: &str) -> bool {
        match (
            parse_rfc3339_timestamp(earlier),
            parse_rfc3339_timestamp(later),
        ) {
            (Some(earlier), Some(later)) => earlier <= later,
            _ => false,
        }
    }

    /// Parse a Codex JSONL file
    pub fn parse_codex_file(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state(
            file_path,
            range,
            start_offset,
            initial_model,
            initial_totals,
            None,
            None,
            None,
        )
    }

    /// Parse a Codex file while retaining the timestamp-order state of an
    /// already decoded prefix.  A known prefix only pays for the append
    /// boundary and newly read token events; an unknown legacy prefix is
    /// intentionally rejected by the caller and should be parsed from zero.
    pub fn parse_codex_file_with_state(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        cancel: Option<&AtomicBool>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state_bounded(
            file_path,
            range,
            start_offset,
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            cancel,
            None,
        )
    }

    /// Parse a Codex file with an optional cap on bytes newly consumed this pass.
    /// The reader may finish the current bounded JSONL line before yielding.
    #[allow(
        clippy::too_many_arguments,
        reason = "resume state mirrors the persisted parser cache"
    )]
    pub fn parse_codex_file_with_state_bounded(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        cancel: Option<&AtomicBool>,
        max_bytes_to_read: Option<i64>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state_bounded_internal(
            file_path,
            range,
            start_offset,
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            cancel,
            false,
            max_bytes_to_read,
        )
    }

    /// Parse a forked Codex child from byte zero with a parent cumulative
    /// baseline. This is intentionally separate from ordinary append-resume
    /// parsing so existing non-fork semantics remain unchanged.
    #[allow(
        clippy::too_many_arguments,
        reason = "fork parse state mirrors the persisted parser cache"
    )]
    pub(crate) fn parse_codex_file_with_state_bounded_fork(
        file_path: &Path,
        range: &CostUsageDayRange,
        initial_totals: CodexTotals,
        cancel: Option<&AtomicBool>,
        max_bytes_to_read: Option<i64>,
    ) -> std::io::Result<CodexParseResult> {
        Self::parse_codex_file_with_state_bounded_internal(
            file_path,
            range,
            0,
            None,
            Some(initial_totals),
            None,
            None,
            cancel,
            true,
            max_bytes_to_read,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "resume state mirrors the persisted parser cache"
    )]
    fn parse_codex_file_with_state_bounded_internal(
        file_path: &Path,
        range: &CostUsageDayRange,
        start_offset: i64,
        initial_model: Option<String>,
        initial_totals: Option<CodexTotals>,
        previous_token_timestamp: Option<String>,
        token_timestamps_monotonic: Option<bool>,
        cancel: Option<&AtomicBool>,
        fork_baseline_mode: bool,
        max_bytes_to_read: Option<i64>,
    ) -> std::io::Result<CodexParseResult> {
        let file = File::open(file_path)?;
        // Session JSONL files are bounded by the cache budget; sizes fit i64.
        #[allow(
            clippy::cast_possible_wrap,
            reason = "session JSONL file sizes fit i64"
        )]
        let file_size = file.metadata()?.len() as i64;

        let mut reader = BufReader::new(file);
        if start_offset > 0 {
            reader.seek(SeekFrom::Start(start_offset as u64))?;
        }

        let mut parser = CodexParserState::with_timestamp_state_and_fork_mode(
            initial_model,
            initial_totals,
            previous_token_timestamp,
            token_timestamps_monotonic,
            fork_baseline_mode,
        );
        let mut parsed_bytes = start_offset;
        let mut cancelled = false;
        let mut budget_exhausted = false;

        loop {
            if max_bytes_to_read.is_some_and(|limit| {
                parsed_bytes.saturating_sub(start_offset) >= limit.max(0)
                    && parsed_bytes < file_size
            }) {
                budget_exhausted = true;
                break;
            }
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                break;
            }
            let Some((line_bytes, consumed)) =
                read_bounded_jsonl_line(&mut reader, CODEX_JSONL_MAX_LINE_BYTES)?
            else {
                break;
            };
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                break;
            }
            // Per-line byte counts are capped at 256 KiB, far inside i64::MAX.
            #[allow(
                clippy::cast_possible_wrap,
                reason = "per-line consumed bytes are capped at CODEX_JSONL_MAX_LINE_BYTES"
            )]
            let consumed_i64 = consumed as i64;
            parsed_bytes += consumed_i64;
            if line_bytes.is_empty() {
                continue;
            }
            let Ok(line) = std::str::from_utf8(&line_bytes) else {
                continue;
            };
            let line = line.strip_suffix('\r').unwrap_or(line);
            parser.process_line(line, range);
        }

        let bytes_read = parsed_bytes.saturating_sub(start_offset).max(0);
        let is_complete = !cancelled && !budget_exhausted && parsed_bytes >= file_size;
        Ok(CodexParseResult {
            records: parser.records,
            parsed_bytes: if is_complete {
                file_size.max(parsed_bytes)
            } else {
                parsed_bytes
            },
            last_model: parser.current_model,
            last_totals: parser.previous_totals,
            token_timestamps_monotonic: parser.token_timestamps_monotonic,
            last_token_timestamp: parser.previous_token_timestamp,
            token_timestamp_comparisons: parser.token_timestamp_comparisons,
            bytes_read,
            is_complete,
            fork_baseline_ambiguous: parser.fork_baseline_ambiguous,
        })
    }

    /// F2 (upstream 0.48.0 #2648): whether a cached resume offset sits on a real
    /// line boundary. A partial trailing-line write leaves the cached offset
    /// mid-line; resuming there re-parses from mid-line and corrupts the first
    /// resumed record. Returns  when the byte just before  is
    /// not a newline (or the probe fails), signalling the caller to fall back
    /// to a full re-parse from zero.
    pub fn is_line_boundary_offset(file_path: &Path, offset: i64) -> bool {
        use std::io::{Read, Seek};
        if offset <= 0 {
            return true;
        }
        // Session JSONL file sizes fit i64; metadata feeds only boundary probes.
        #[allow(
            clippy::cast_possible_wrap,
            reason = "session JSONL file sizes fit i64"
        )]
        let file_size_i64 = fs::metadata(file_path).map(|m| m.len() as i64);
        let Ok(file_size) = file_size_i64 else {
            return false;
        };
        if offset >= file_size {
            return true;
        }
        let Ok(mut probe) = File::open(file_path) else {
            return false;
        };
        if probe.seek(SeekFrom::Start((offset - 1) as u64)).is_err() {
            return false;
        }
        let mut prev_byte = [0u8; 1];
        probe.read_exact(&mut prev_byte).is_ok() && prev_byte[0] == b'\n'
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
