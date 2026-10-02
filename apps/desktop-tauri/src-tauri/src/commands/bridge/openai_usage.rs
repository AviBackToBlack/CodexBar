//! Bridge DTOs for the OpenAI Admin API per-day usage history (`openAiApiUsage`).
//!
//! Mirrors `OpenAiApiUsageSnapshot` in `src/types/bridge.ts`. Times are epoch seconds and
//! counts are non-negative integers, as in upstream 0.66.0's card payload.

use codexbar::core::{
    OpenAiApiDailyUsage, OpenAiApiLineItemCost, OpenAiApiModelUsage, OpenAiApiUsageHistory,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiApiUsageSnapshot {
    pub history_days: u32,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub daily: Vec<OpenAiApiDailyUsageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiApiDailyUsageSnapshot {
    pub start_time: i64,
    pub end_time: i64,
    pub cost_usd: f64,
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub line_items: Vec<OpenAiApiLineItemSnapshot>,
    #[serde(default)]
    pub models: Vec<OpenAiApiModelUsageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiApiLineItemSnapshot {
    pub name: String,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiApiModelUsageSnapshot {
    pub name: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl From<&OpenAiApiUsageHistory> for OpenAiApiUsageSnapshot {
    fn from(history: &OpenAiApiUsageHistory) -> Self {
        Self {
            history_days: history.history_days,
            project_id: history.project_id.clone(),
            daily: history.daily.iter().map(Into::into).collect(),
        }
    }
}

impl From<&OpenAiApiDailyUsage> for OpenAiApiDailyUsageSnapshot {
    fn from(day: &OpenAiApiDailyUsage) -> Self {
        Self {
            start_time: day.start_time,
            end_time: day.end_time,
            cost_usd: day.cost_usd,
            requests: day.requests,
            input_tokens: day.input_tokens,
            cached_input_tokens: day.cached_input_tokens,
            output_tokens: day.output_tokens,
            total_tokens: day.total_tokens,
            line_items: day.line_items.iter().map(Into::into).collect(),
            models: day.models.iter().map(Into::into).collect(),
        }
    }
}

impl From<&OpenAiApiLineItemCost> for OpenAiApiLineItemSnapshot {
    fn from(item: &OpenAiApiLineItemCost) -> Self {
        Self {
            name: item.name.clone(),
            cost_usd: item.cost_usd,
        }
    }
}

impl From<&OpenAiApiModelUsage> for OpenAiApiModelUsageSnapshot {
    fn from(model: &OpenAiApiModelUsage) -> Self {
        Self {
            name: model.name.clone(),
            requests: model.requests,
            input_tokens: model.input_tokens,
            cached_input_tokens: model.cached_input_tokens,
            output_tokens: model.output_tokens,
            total_tokens: model.total_tokens,
        }
    }
}
