//! LiteLLM management-route payloads and their projection into a usage result.
//!
//! Wire shapes follow upstream `litellm.ts`: `/key/info` names the key's
//! `user_id` / `team_id`, then `/user/info` or `/team/info` supplies the
//! budgets. Returned IDs must match the key's IDs before anything is shown.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::core::{
    CostSnapshot, ProviderError, ProviderFetchResult, RateWindow, SubscriptionMetadata,
    UsageSnapshot,
};

#[derive(Deserialize)]
pub(super) struct KeyInfoResponse {
    info: KeyInfo,
}

#[derive(Deserialize)]
struct KeyInfo {
    user_id: Option<String>,
    team_id: Option<String>,
    expires: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct UserInfoResponse {
    user_id: Option<String>,
    user_info: UserInfo,
    teams: Option<Vec<Budget>>,
}

#[derive(Deserialize)]
struct UserInfo {
    user_id: Option<String>,
    user_email: Option<String>,
    user_alias: Option<String>,
    spend: Option<f64>,
    max_budget: Option<f64>,
    budget_reset_at: Option<String>,
    metadata: Option<UserMetadata>,
}

#[derive(Deserialize)]
struct UserMetadata {
    preferred_username: Option<Value>,
}

#[derive(Deserialize)]
pub(super) struct TeamInfoResponse {
    team_id: Option<String>,
    team_info: Budget,
}

#[derive(Deserialize)]
struct Budget {
    team_id: Option<String>,
    team_alias: Option<String>,
    spend: Option<f64>,
    max_budget: Option<f64>,
    budget_reset_at: Option<String>,
}

/// Identity and routing data read from `/key/info`.
pub(super) struct KeyBinding {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    expires: Option<DateTime<Utc>>,
}

/// A spend/budget pair with its optional reset instant.
struct Spend {
    spend: f64,
    limit: Option<f64>,
    reset: Option<DateTime<Utc>>,
}

impl Spend {
    fn budget(&self) -> Option<f64> {
        self.limit.filter(|limit| *limit > 0.0)
    }

    fn window(&self, label: Option<&str>) -> Option<RateWindow> {
        let limit = self.budget()?;
        let mut window = RateWindow::new(self.spend / limit * 100.0);
        window.resets_at = self.reset;
        let detail = format!("${:.2} / ${limit:.2}", self.spend);
        window.reset_description = Some(match label {
            Some(label) => format!("{label}: {detail}"),
            None => detail,
        });
        Some(window)
    }
}

struct TeamBudget {
    alias: Option<String>,
    spend: Spend,
}

impl TeamBudget {
    fn from_wire(budget: &Budget) -> Self {
        Self {
            alias: budget.team_alias.clone(),
            spend: Spend {
                spend: budget.spend.unwrap_or(0.0),
                limit: budget.max_budget,
                reset: parse_date(budget.budget_reset_at.as_deref()),
            },
        }
    }

    fn window(&self) -> Option<RateWindow> {
        let label = match &self.alias {
            Some(alias) => format!("Team {alias}"),
            None => "Team".to_string(),
        };
        self.spend.window(Some(&label))
    }
}

pub(super) fn parse_error(message: impl std::fmt::Display) -> ProviderError {
    ProviderError::Parse(format!("LiteLLM parse error: {message}"))
}

pub(super) fn bind_key(response: KeyInfoResponse) -> Result<KeyBinding, ProviderError> {
    let info = response.info;
    let user_id = nonempty(info.user_id);
    let team_id = nonempty(info.team_id);
    if user_id.is_none() && team_id.is_none() {
        return Err(parse_error(
            "LiteLLM key info did not include a user_id or team_id.",
        ));
    }
    Ok(KeyBinding {
        user_id,
        team_id,
        expires: parse_date(info.expires.as_deref()),
    })
}

/// Project a user-bound key: personal budget plus the key's matching team.
pub(super) fn result_from_user(
    key: &KeyBinding,
    user_id: &str,
    response: UserInfoResponse,
) -> Result<ProviderFetchResult, ProviderError> {
    let user = response.user_info;
    let response_id = user.user_id.as_deref().or(response.user_id.as_deref());
    if response_id.is_some_and(|id| id != user_id) {
        return Err(parse_error("user_id did not match /key/info"));
    }
    let preferred = user
        .metadata
        .and_then(|metadata| metadata.preferred_username)
        .and_then(|value| value.as_str().map(str::to_owned));
    let email = nonempty(user.user_email)
        .or_else(|| nonempty(user.user_alias))
        .or_else(|| nonempty(preferred));
    let mut team = None;
    for wire in response.teams.unwrap_or_default() {
        let id = wire
            .team_id
            .as_deref()
            .ok_or_else(|| parse_error("missing team_id"))?;
        if team.is_none() && key.team_id.as_deref() == Some(id) {
            team = Some(TeamBudget::from_wire(&wire));
        }
    }
    let personal = Spend {
        spend: user.spend.unwrap_or(0.0),
        limit: user.max_budget,
        reset: parse_date(user.budget_reset_at.as_deref()),
    };
    let primary = personal
        .window(None)
        .unwrap_or_else(|| RateWindow::new(0.0));
    let mut snapshot = UsageSnapshot::new(primary);
    if let Some(email) = email {
        snapshot = snapshot.with_email(email);
    }
    if let Some(team) = &team {
        if let Some(alias) = &team.alias {
            snapshot = snapshot.with_organization(alias);
        }
        if let Some(window) = team.window() {
            snapshot = snapshot.with_extra_rate_window("team", "Team budget", window);
        }
    }
    Ok(finish(snapshot, key, &personal, "Personal"))
}

/// Project a team-only key: the team budget is the sole usage window.
pub(super) fn result_from_team(
    key: &KeyBinding,
    team_id: &str,
    response: TeamInfoResponse,
) -> Result<ProviderFetchResult, ProviderError> {
    let response_id = response
        .team_info
        .team_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .or(response
            .team_id
            .as_deref()
            .filter(|id| !id.trim().is_empty()));
    if response_id.is_some_and(|id| id != team_id) {
        return Err(parse_error("team_id did not match /key/info"));
    }
    let team = TeamBudget::from_wire(&response.team_info);
    let window = team.window().unwrap_or_else(|| RateWindow::new(0.0));
    let mut snapshot = UsageSnapshot::new(window).with_primary_label("Team budget");
    if let Some(alias) = &team.alias {
        snapshot = snapshot.with_organization(alias);
    }
    Ok(finish(snapshot, key, &team.spend, "Team"))
}

fn finish(
    snapshot: UsageSnapshot,
    key: &KeyBinding,
    spend: &Spend,
    scope: &str,
) -> ProviderFetchResult {
    let mut snapshot = snapshot.with_login_method("api");
    if key.expires.is_some() {
        snapshot =
            snapshot.with_subscription(Some(SubscriptionMetadata::new(None, key.expires, None)));
    }
    let mut result = ProviderFetchResult::new(snapshot, "api");
    let limit = spend.budget();
    if spend.spend > 0.0 || limit.is_some() {
        let kind = if limit.is_some() { "budget" } else { "spend" };
        let mut cost = CostSnapshot::new(spend.spend, "USD", format!("{scope} {kind}"));
        if let Some(limit) = limit {
            cost = cost.with_limit(limit);
        }
        if let Some(reset) = spend.reset {
            cost = cost.with_resets_at(reset);
        }
        result = result.with_cost(cost);
    }
    result
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parse an ISO-8601 instant; naive timestamps are read as UTC and anything
/// unparseable is dropped, matching upstream's tolerant `date()` helper.
fn parse_date(value: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(raw)
        .map(|date| date.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|date| date.and_utc())
        })
}
