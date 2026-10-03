use super::openai_usage::OpenAiApiUsageSnapshot;
use crate::commands::ProviderUsageSnapshot;
use codexbar::core::{
    OpenAiApiDailyUsage, OpenAiApiLineItemCost, OpenAiApiModelUsage, OpenAiApiUsageHistory,
    ProviderFetchResult, ProviderId, RateWindow, UsageSnapshot, instantiate_provider,
};

fn history() -> OpenAiApiUsageHistory {
    OpenAiApiUsageHistory {
        history_days: 30,
        project_id: Some("proj_abc".to_string()),
        daily: vec![OpenAiApiDailyUsage {
            start_time: 1_700_000_000,
            end_time: 1_700_086_400,
            cost_usd: 14.75,
            requests: 10,
            input_tokens: 1300,
            cached_input_tokens: 250,
            output_tokens: 700,
            total_tokens: 2000,
            line_items: vec![OpenAiApiLineItemCost {
                name: "Text tokens".to_string(),
                cost_usd: 12.5,
            }],
            models: vec![OpenAiApiModelUsage {
                name: "gpt-5.2".to_string(),
                requests: 7,
                input_tokens: 1000,
                cached_input_tokens: 250,
                output_tokens: 500,
                total_tokens: 1500,
            }],
        }],
    }
}

fn snapshot(history: Option<OpenAiApiUsageHistory>) -> ProviderUsageSnapshot {
    let metadata = instantiate_provider(ProviderId::OpenAIApi)
        .metadata()
        .clone();
    let mut result =
        ProviderFetchResult::new(UsageSnapshot::new(RateWindow::new(0.0)), "admin-api");
    result.open_ai_api_usage = history;
    ProviderUsageSnapshot::from_fetch_result(ProviderId::OpenAIApi, &metadata, &result, None)
}

#[test]
fn fetch_result_history_reaches_the_bridge_as_camel_case_epoch_seconds() {
    let json = serde_json::to_value(snapshot(Some(history()))).unwrap();
    assert_eq!(
        json["openAiApiUsage"],
        serde_json::json!({
            "historyDays": 30,
            "projectId": "proj_abc",
            "daily": [{
                "startTime": 1_700_000_000,
                "endTime": 1_700_086_400,
                "costUsd": 14.75,
                "requests": 10,
                "inputTokens": 1300,
                "cachedInputTokens": 250,
                "outputTokens": 700,
                "totalTokens": 2000,
                "lineItems": [{"name": "Text tokens", "costUsd": 12.5}],
                "models": [{
                    "name": "gpt-5.2",
                    "requests": 7,
                    "inputTokens": 1000,
                    "cachedInputTokens": 250,
                    "outputTokens": 500,
                    "totalTokens": 1500,
                }],
            }],
        })
    );
}

#[test]
fn missing_history_is_omitted_from_the_bridge_payload() {
    let json = serde_json::to_value(snapshot(None)).unwrap();
    assert!(json.get("openAiApiUsage").is_none());
}

#[test]
fn history_round_trips_through_the_bridge_payload() {
    let original = snapshot(Some(history()));
    let decoded: ProviderUsageSnapshot =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    assert_eq!(decoded.open_ai_api_usage, original.open_ai_api_usage);
    assert_eq!(
        decoded.open_ai_api_usage.unwrap().project_id.as_deref(),
        Some("proj_abc")
    );
}

#[test]
fn an_empty_history_serializes_a_null_project_and_no_days() {
    let empty = OpenAiApiUsageHistory {
        history_days: 30,
        project_id: None,
        daily: Vec::new(),
    };
    let json = serde_json::to_value(OpenAiApiUsageSnapshot::from(&empty)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"historyDays": 30, "projectId": null, "daily": []})
    );
}
