//! LiteLLM management-route payloads and their projection into a usage result.
//!
//! Wire shapes follow upstream `litellm.ts`: `/key/info` names the key's
//! `user_id` / `team_id`, then `/user/info` or `/team/info` supplies the
//! budgets. Returned IDs must match the key's IDs before anything is shown.
//!
//! Upstream reports the personal budget as the primary window and the key's
//! team budget as the secondary window, and either may be absent. A Windows
//! snapshot always has a primary lane, so the windows that exist fill the
//! lanes in that order (personal, then team) with their own labels, and a key
//! with no budget at all shows an informational "No budget set" lane.
//! Budget amounts are detail lines, never reset wording (upstream #3631).

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
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
    /// Type-checked like upstream, but not otherwise used.
    #[serde(rename = "key_name")]
    _key_name: Option<String>,
    #[serde(rename = "spend")]
    _spend: Option<f64>,
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
    /// Type-checked like upstream, but not otherwise used.
    #[serde(rename = "budget_duration")]
    _budget_duration: Option<String>,
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

    /// The budget window, or `None` without a positive budget. The amount is
    /// a detail line, so a missing reset never reads as "Resets $x / $y" and
    /// a real reset never hides the amount.
    fn window(&self, label: Option<&str>) -> Option<RateWindow> {
        let limit = self.budget()?;
        let mut window = RateWindow::new(self.spend / limit * 100.0);
        window.resets_at = self.reset;
        let detail = format!("{} / {}", usd(self.spend), usd(limit));
        window.reset_description = Some(match label {
            Some(label) => format!("{label}: {detail}"),
            None => detail,
        });
        Some(window.with_description_as_detail())
    }
}

struct TeamBudget {
    alias: Option<String>,
    spend: Spend,
}

impl TeamBudget {
    fn from_wire(budget: &Budget) -> Self {
        Self {
            alias: nonempty(budget.team_alias.clone()),
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

/// Lane label for a team budget shown in the primary lane.
const TEAM_BUDGET: &str = "Team budget";
/// Informational lane text for a key without a personal or team budget.
const NO_BUDGET: &str = "No budget set";

/// Upstream fails with `LiteLLM parse error: <message>`. `ProviderError::Parse`
/// already displays as "Parse error: …", so only the provider name is added.
pub(super) fn parse_error(message: impl std::fmt::Display) -> ProviderError {
    ProviderError::Parse(format!("LiteLLM {message}"))
}

pub(super) const MISSING_KEY_IDS: &str = "key info did not include a user_id or team_id.";

pub(super) fn bind_key(response: KeyInfoResponse) -> Result<KeyBinding, ProviderError> {
    let info = response.info;
    let user_id = nonempty(info.user_id);
    let team_id = nonempty(info.team_id);
    if user_id.is_none() && team_id.is_none() {
        return Err(parse_error(MISSING_KEY_IDS));
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
    let team_window = team.as_ref().and_then(TeamBudget::window);
    let mut snapshot = match (personal.window(None), team_window) {
        (Some(personal), team) => {
            let mut snapshot = UsageSnapshot::new(personal);
            if let Some(team) = team {
                snapshot = snapshot.with_secondary(team);
            }
            snapshot
        }
        (None, Some(team)) => UsageSnapshot::new(team).with_primary_label(TEAM_BUDGET),
        (None, None) => UsageSnapshot::new(RateWindow::informational(NO_BUDGET)),
    };
    if let Some(email) = email {
        snapshot = snapshot.with_email(email);
    }
    if let Some(alias) = team.as_ref().and_then(|team| team.alias.as_deref()) {
        snapshot = snapshot.with_organization(alias);
    }
    Ok(finish(snapshot, key, &personal, "Personal"))
}

/// Project a team-only key: the team budget is the sole usage window.
pub(super) fn result_from_team(
    key: &KeyBinding,
    team_id: &str,
    response: TeamInfoResponse,
) -> Result<ProviderFetchResult, ProviderError> {
    let response_id = trimmed_id(response.team_info.team_id.as_deref())
        .or_else(|| trimmed_id(response.team_id.as_deref()));
    if response_id.is_some_and(|id| id != team_id) {
        return Err(parse_error("team_id did not match /key/info"));
    }
    let team = TeamBudget::from_wire(&response.team_info);
    let window = team
        .window()
        .unwrap_or_else(|| RateWindow::informational(NO_BUDGET));
    let mut snapshot = UsageSnapshot::new(window).with_primary_label(TEAM_BUDGET);
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
    // Budget resets carry no window length, so the shell's weekly pace
    // defaults would invent a pace upstream never shows.
    let mut result = ProviderFetchResult::new(snapshot, "api").with_non_authoritative_pace();
    let limit = spend.budget();
    if spend.spend > 0.0 || limit.is_some() {
        let kind = if limit.is_some() { "budget" } else { "spend" };
        let mut cost = CostSnapshot::new(spend.spend, "USD", format!("{scope} {kind}"));
        match limit {
            Some(limit) => cost = cost.with_limit(limit),
            // Upstream's "API spend" card: spend without a budget is the
            // only usage signal, so it stays visible like other API spend.
            None => cost = cost.always_visible(),
        }
        if let Some(reset) = spend.reset {
            cost = cost.with_resets_at(reset);
        }
        result = result.with_cost(cost);
    }
    result
}

/// Upstream `ctx.format.usd`: two decimals with thousands separators, and
/// `-$` for negative amounts.
fn usd(amount: f64) -> String {
    let fixed = format!("{:.2}", amount.abs());
    let (whole, cents) = fixed.split_once('.').unwrap_or((&fixed, "00"));
    let mut grouped = String::with_capacity(whole.len() + whole.len() / 3);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if amount < 0.0 { "-$" } else { "$" };
    format!("{sign}{grouped}.{cents}")
}

fn trimmed_id(id: Option<&str>) -> Option<&str> {
    id.map(str::trim).filter(|id| !id.is_empty())
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parse an ISO-8601 instant and drop anything unparseable, like upstream's
/// tolerant `date()` helper. A date-only value is UTC midnight, as in
/// JavaScript. A timestamp without an offset is also read as UTC, because
/// LiteLLM stores its budget times in UTC; upstream's JavaScript `Date` would
/// read it in the machine's local zone instead.
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
        .or_else(|| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date| date.and_utc())
        })
}
