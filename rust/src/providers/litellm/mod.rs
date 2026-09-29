use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, Response, StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;

use crate::core::{
    CostSnapshot, FetchContext, Provider, ProviderError, ProviderFetchResult, ProviderId,
    ProviderMetadata, RateWindow, SourceMode, UsageSnapshot,
};

const CREDENTIAL_TARGET: &str = "codexbar-litellm";
/// Spend reports list one row per key or user; bound the body regardless.
const MAX_SPEND_REPORT_BYTES: usize = 4 * 1024 * 1024;

pub struct LiteLLMProvider {
    metadata: ProviderMetadata,
    client: Client,
}

impl LiteLLMProvider {
    pub fn new() -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::LiteLLM,
                display_name: "LiteLLM",
                session_label: "Budget",
                weekly_label: "Spend",
                supports_opus: false,
                supports_credits: true,
                default_enabled: false,
                is_primary: false,
                dashboard_url: None,
                status_page_url: None,
                tertiary_label_key: None,
            },
            client: crate::core::credentialed_http_client_builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }
}

impl Default for LiteLLMProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for LiteLLMProvider {
    fn id(&self) -> ProviderId {
        ProviderId::LiteLLM
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    async fn fetch_usage(&self, ctx: &FetchContext) -> Result<ProviderFetchResult, ProviderError> {
        match ctx.source_mode {
            SourceMode::Auto | SourceMode::OAuth => {
                let (base, key) = resolve_base_and_key(ctx)?;
                fetch_key_usage(
                    &self.client,
                    |path| management_url(&base, path),
                    &key,
                    Utc::now(),
                )
                .await
            }
            SourceMode::Web | SourceMode::Cli => {
                Err(ProviderError::UnsupportedSource(ctx.source_mode))
            }
        }
    }

    fn available_sources(&self) -> Vec<SourceMode> {
        vec![SourceMode::Auto, SourceMode::OAuth]
    }
}

fn resolve_base_and_key(ctx: &FetchContext) -> Result<(String, String), ProviderError> {
    if let Some(base) = ctx
        .workspace_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let key = ctx.api_key.as_deref().ok_or(ProviderError::AuthRequired)?;
        return Ok((base.to_string(), key.to_string()));
    }

    let key = crate::providers::resolve_api_key(
        ctx.api_key.as_deref(),
        CREDENTIAL_TARGET,
        &["LITELLM_API_KEY"],
    )?;
    let base = std::env::var("LITELLM_BASE_URL")
        .ok()
        .or_else(|| std::env::var("LITELLM_API_BASE").ok())
        .ok_or_else(|| {
            ProviderError::NotInstalled(
                "LiteLLM base URL not found. Set it in provider extras or LITELLM_BASE_URL.".into(),
            )
        })?;
    Ok((base, key))
}

fn management_url(base: &str, path: &str) -> Result<Url, ProviderError> {
    let mut url = crate::providers::validated_https_url(base, "LiteLLM base")?;
    let prefix = url.path().trim_end_matches('/');
    let prefix = prefix.strip_suffix("/v1").unwrap_or(prefix);
    let full_path = format!("{prefix}/{path}");
    url.set_path(&full_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// `key/info` (or report) statuses that mean the route is not available to
/// this key, so the next, more narrowly scoped source is tried.
fn route_unavailable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    )
}

async fn get_json(client: &Client, url: Url, key: &str) -> Result<Response, ProviderError> {
    Ok(client
        .get(url)
        .bearer_auth(key)
        .header("Accept", "application/json")
        .send()
        .await?)
}

async fn fetch_key_usage(
    client: &Client,
    url_for: impl Fn(&str) -> Result<Url, ProviderError>,
    key: &str,
    now: DateTime<Utc>,
) -> Result<ProviderFetchResult, ProviderError> {
    let response = get_json(client, url_for("key/info")?, key).await?;
    if route_unavailable(response.status()) {
        return fetch_spend_report(client, url_for, key, now).await;
    }
    if !response.status().is_success() {
        return Err(ProviderError::Other(format!(
            "LiteLLM key/info returned status {}",
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|e| ProviderError::Parse(format!("Failed to parse LiteLLM key/info: {e}")))?;
    Ok(result_from_key_info(&value))
}

/// Month-to-date (UTC) spend for deployments that disable management routes.
/// Tries the key-scoped report first and the user-scoped report only when the
/// key report is unavailable; every other failure keeps its class.
async fn fetch_spend_report(
    client: &Client,
    url_for: impl Fn(&str) -> Result<Url, ProviderError>,
    key: &str,
    now: DateTime<Utc>,
) -> Result<ProviderFetchResult, ProviderError> {
    let start = now.format("%Y-%m-01").to_string();
    let end = now.format("%Y-%m-%d").to_string();
    let query = format!("start_date={start}&end_date={end}");
    let report_url = |path: &str| -> Result<Url, ProviderError> {
        let mut url = url_for(path)?;
        url.set_query(Some(&query));
        Ok(url)
    };

    let mut scope = "Key";
    let mut response = get_json(client, report_url("key/spend/report")?, key).await?;
    if route_unavailable(response.status()) {
        scope = "User";
        response = get_json(client, report_url("user/spend/report")?, key).await?;
    }
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ProviderError::AuthRequired);
    }
    if !status.is_success() {
        // The report body is never echoed: it lists key identifiers.
        return Err(ProviderError::Other(format!(
            "LiteLLM {} spend report returned status {status}",
            scope.to_ascii_lowercase()
        )));
    }
    let body = crate::providers::read_bounded_response(response, MAX_SPEND_REPORT_BYTES)
        .await
        .map_err(|_| ProviderError::Parse("Failed to read LiteLLM spend report".into()))?;
    let used = parse_spend_report(&body)?;

    let period = format!("{scope} spend only ({start}\u{2013}{end} UTC)");
    // UsageSnapshot needs a primary lane; a report carries no budget, so the
    // labeled window is informational and the amount lives in the cost.
    let snapshot = UsageSnapshot::new(RateWindow::informational(period.clone()));
    Ok(ProviderFetchResult::new(snapshot, "api").with_cost(CostSnapshot::new(used, "USD", period)))
}

#[derive(Deserialize)]
struct SpendReportRow {
    total_cost: f64,
}

/// Sum each row's `total_cost` once (model subtotals are ignored). An empty,
/// malformed, negative, or overflowing report is an error, never zero spend.
fn parse_spend_report(body: &[u8]) -> Result<f64, ProviderError> {
    // Serde messages can quote report values, so they are not surfaced.
    let invalid = || ProviderError::Parse("LiteLLM spend report is empty or invalid".into());
    let rows: Vec<SpendReportRow> = serde_json::from_slice(body).map_err(|_| invalid())?;
    if rows.is_empty() {
        return Err(invalid());
    }
    let mut used = 0.0;
    for row in rows {
        if row.total_cost < 0.0 {
            return Err(invalid());
        }
        used += row.total_cost;
    }
    if used.is_finite() {
        Ok(used)
    } else {
        Err(invalid())
    }
}

fn result_from_key_info(value: &Value) -> ProviderFetchResult {
    let root = value
        .get("info")
        .or_else(|| value.get("key"))
        .unwrap_or(value);
    let spend = number(root, &["spend", "spend_usd", "spendUSD"]).unwrap_or(0.0);
    let limit = number(root, &["max_budget", "maxBudget", "budget", "limit"]);
    let percent = limit
        .filter(|v| *v > 0.0)
        .map_or(0.0, |limit| spend / limit * 100.0);
    let mut primary = RateWindow::new(percent);
    if let Some(limit) = limit.filter(|value| *value > 0.0) {
        primary.reset_description = Some(budget_detail(spend, limit));
    }
    let mut snapshot = UsageSnapshot::new(primary).with_login_method(format!("Spend ${spend:.2}"));
    if let Some(team) = root.get("team_info").or_else(|| root.get("teamInfo"))
        && let Some(team_spend) = number(team, &["spend", "team_spend", "teamSpend"])
    {
        let team_limit = number(team, &["max_budget", "budget", "limit"]);
        let team_percent = team_limit
            .filter(|v| *v > 0.0)
            .map_or(0.0, |limit| team_spend / limit * 100.0);
        let mut team_window = RateWindow::new(team_percent);
        if let Some(team_limit) = team_limit.filter(|value| *value > 0.0) {
            let alias = string(team, &["team_alias", "teamAlias", "alias"])
                .map(|value| format!("Team {value}: "))
                .unwrap_or_default();
            team_window.reset_description =
                Some(format!("{alias}{}", budget_detail(team_spend, team_limit)));
        }
        snapshot = snapshot.with_extra_rate_window("team", "Team budget", team_window);
    }
    let mut result = ProviderFetchResult::new(snapshot, "api");
    if spend > 0.0 {
        let mut cost = CostSnapshot::new(spend, "USD", "Spend");
        if let Some(limit) = limit {
            cost = cost.with_limit(limit);
        }
        result = result.with_cost(cost);
    }
    result
}

fn number(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_f64))
}

fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn budget_detail(spend: f64, budget: f64) -> String {
    format!("${spend:.2} / ${budget:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spend_budget() {
        let result =
            result_from_key_info(&serde_json::json!({"info":{"spend":25.0,"max_budget":100.0}}));
        assert_eq!(result.usage.primary.used_percent, 25.0);
        assert_eq!(
            result.usage.primary.reset_description.as_deref(),
            Some("$25.00 / $100.00")
        );
    }

    #[test]
    fn preserves_team_budget_detail_with_alias() {
        let result = result_from_key_info(&serde_json::json!({
            "info": {
                "team_info": {
                    "team_alias": "Platform",
                    "spend": 70.0,
                    "max_budget": 1000.0
                }
            }
        }));
        assert_eq!(result.usage.extra_rate_windows.len(), 1);
        assert_eq!(
            result.usage.extra_rate_windows[0]
                .window
                .reset_description
                .as_deref(),
            Some("Team Platform: $70.00 / $1000.00")
        );
    }

    /// Shape reported upstream in #3834; identifiers and amounts are synthetic.
    const REPORT: &str = r#"[{"api_key":"synthetic-key-identifier","total_cost":1.25,
        "total_input_tokens":1000,"total_output_tokens":100,
        "model_details":[{"model":"example-model","total_cost":1.25,
        "total_input_tokens":1000,"total_output_tokens":100}]},
        {"api_key":"another-synthetic-identifier","total_cost":2.5}]"#;
    const KEY_QUERY: &str = "?start_date=2026-09-01&end_date=2026-09-21";

    fn sept_21() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn test_client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    /// Serves `path` (under the `/proxy` prefix) with `status` and `body`,
    /// asserting the bearer key. `hits` is how many requests it must receive.
    async fn stub(
        server: &mut mockito::ServerGuard,
        path: &str,
        status: usize,
        body: &str,
        hits: usize,
    ) -> mockito::Mock {
        let mock = server.mock("GET", path);
        // A bare path would not match its own request once a query is added.
        let mock = if path.contains('?') {
            mock
        } else {
            mock.match_query(mockito::Matcher::Any)
        };
        mock.match_header("authorization", "Bearer fixture-key")
            .match_header("accept", "application/json")
            .with_status(status)
            .with_body(body)
            .expect(hits)
            .create_async()
            .await
    }

    async fn run(
        server: &mockito::ServerGuard,
        now: DateTime<Utc>,
    ) -> Result<ProviderFetchResult, ProviderError> {
        let base = server.url();
        fetch_key_usage(
            &test_client(),
            |path| {
                Url::parse(&format!("{base}/proxy/{path}"))
                    .map_err(|e| ProviderError::Other(e.to_string()))
            },
            "fixture-key",
            now,
        )
        .await
    }

    #[test]
    fn management_url_keeps_path_prefix_and_strips_v1() {
        for base in [
            "https://proxy.example.com/proxy/v1/",
            "https://proxy.example.com/proxy/v1",
            "https://proxy.example.com/proxy/",
            "https://proxy.example.com/proxy",
        ] {
            assert_eq!(
                management_url(base, "key/spend/report").unwrap().as_str(),
                "https://proxy.example.com/proxy/key/spend/report"
            );
        }
        assert_eq!(
            management_url("https://litellm.example.com", "key/info")
                .unwrap()
                .as_str(),
            "https://litellm.example.com/key/info"
        );
        assert_eq!(
            management_url("https://litellm.example.com/v1", "key/info")
                .unwrap()
                .as_str(),
            "https://litellm.example.com/key/info"
        );
    }

    #[test]
    fn sums_each_row_once_and_ignores_model_details() {
        assert_eq!(parse_spend_report(REPORT.as_bytes()).unwrap(), 3.75);
        assert_eq!(
            parse_spend_report(br#"[{"total_cost":0}]"#).unwrap(),
            0.0,
            "an explicit zero spend is valid"
        );
    }

    #[test]
    fn invalid_or_empty_reports_never_fabricate_zero_spend() {
        for body in [
            "[]",
            "{}",
            "null",
            "not json",
            "",
            "[null]",
            "[{}]",
            r#"[{"total_cost":null}]"#,
            r#"[{"total_cost":"1.25"}]"#,
            r#"[{"total_cost":-1}]"#,
            r#"[{"total_cost":1e309}]"#,
            r#"[{"total_cost":1e308},{"total_cost":1e308}]"#,
        ] {
            match parse_spend_report(body.as_bytes()) {
                Err(ProviderError::Parse(message)) => {
                    assert!(!message.contains("1.25"), "{body}: {message}");
                }
                other => panic!("{body}: expected a parse error, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn management_denial_falls_back_to_key_spend() {
        for status in [401, 403, 404] {
            let mut server = mockito::Server::new_async().await;
            let info = stub(&mut server, "/proxy/key/info", status, "{}", 1).await;
            let report = stub(
                &mut server,
                &format!("/proxy/key/spend/report{KEY_QUERY}"),
                200,
                REPORT,
                1,
            )
            .await;
            let user = stub(&mut server, "/proxy/user/spend/report", 200, REPORT, 0).await;

            let result = run(&server, sept_21()).await.unwrap();

            info.assert_async().await;
            report.assert_async().await;
            user.assert_async().await;
            let period = "Key spend only (2026-09-01\u{2013}2026-09-21 UTC)";
            let cost = result.cost.expect("spend is reported as a cost");
            assert_eq!(cost.used, 3.75);
            assert_eq!(cost.currency_code, "USD");
            assert_eq!(cost.period, period);
            assert_eq!(
                (cost.limit, cost.resets_at, cost.balance),
                (None, None, None)
            );
            let primary = &result.usage.primary;
            assert!(primary.is_informational && !primary.usage_known);
            assert_eq!(primary.reset_description.as_deref(), Some(period));
            assert!(result.usage.secondary.is_none());
            assert!(result.usage.extra_rate_windows.is_empty());
            assert!(
                result.usage.account_email.is_none() && result.usage.account_organization.is_none()
            );
            assert!(result.usage.login_method.is_none());
        }
    }

    #[tokio::test]
    async fn unavailable_key_report_falls_back_to_user_spend() {
        for status in [401, 403, 404] {
            let mut server = mockito::Server::new_async().await;
            stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
            let key = stub(
                &mut server,
                &format!("/proxy/key/spend/report{KEY_QUERY}"),
                status,
                "{}",
                1,
            )
            .await;
            let user = stub(
                &mut server,
                &format!("/proxy/user/spend/report{KEY_QUERY}"),
                200,
                REPORT,
                1,
            )
            .await;

            let result = run(&server, sept_21()).await.unwrap();

            key.assert_async().await;
            user.assert_async().await;
            let cost = result.cost.unwrap();
            assert_eq!(cost.used, 3.75);
            assert_eq!(
                cost.period,
                "User spend only (2026-09-01\u{2013}2026-09-21 UTC)"
            );
            assert!(result.usage.account_email.is_none());
        }
    }

    #[tokio::test]
    async fn zero_spend_is_valid_and_month_boundary_uses_utc() {
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
        let report = stub(
            &mut server,
            "/proxy/key/spend/report?start_date=2027-01-01&end_date=2027-01-01",
            200,
            r#"[{"total_cost":0}]"#,
            1,
        )
        .await;
        let now = DateTime::parse_from_rfc3339("2027-01-01T00:00:01Z")
            .unwrap()
            .with_timezone(&Utc);

        let cost = run(&server, now).await.unwrap().cost.unwrap();

        report.assert_async().await;
        assert_eq!(cost.used, 0.0);
        assert_eq!(
            cost.period,
            "Key spend only (2027-01-01\u{2013}2027-01-01 UTC)"
        );
    }

    #[tokio::test]
    async fn malformed_report_is_a_parse_error() {
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
        stub(
            &mut server,
            &format!("/proxy/key/spend/report{KEY_QUERY}"),
            200,
            "[]",
            1,
        )
        .await;
        assert!(matches!(
            run(&server, sept_21()).await,
            Err(ProviderError::Parse(_))
        ));
    }

    #[tokio::test]
    async fn transport_failure_does_not_switch_report_scopes() {
        let mut server = mockito::Server::new_async().await;
        stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
        let user = stub(&mut server, "/proxy/user/spend/report", 200, REPORT, 0).await;
        let base = server.url();
        // Key report requests go to a closed port; the user report must not run.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        drop(closed);
        let result = fetch_key_usage(
            &test_client(),
            |path| {
                let origin = if path == "key/spend/report" {
                    format!("http://127.0.0.1:{closed_port}")
                } else {
                    base.clone()
                };
                Url::parse(&format!("{origin}/proxy/{path}"))
                    .map_err(|e| ProviderError::Other(e.to_string()))
            },
            "fixture-key",
            sept_21(),
        )
        .await;
        assert!(matches!(result, Err(ProviderError::Network(_))));
        user.assert_async().await;
    }

    #[tokio::test]
    async fn other_failures_keep_their_class_and_never_echo_the_report() {
        for status in [429, 500, 400] {
            // A management failure other than 401/403/404 never tries a report.
            let mut server = mockito::Server::new_async().await;
            stub(&mut server, "/proxy/key/info", status, REPORT, 1).await;
            let key = stub(&mut server, "/proxy/key/spend/report", 200, REPORT, 0).await;
            let Err(ProviderError::Other(message)) = run(&server, sept_21()).await else {
                panic!("expected an error for {status}");
            };
            assert!(message.contains(&status.to_string()), "{message}");
            key.assert_async().await;

            // The same statuses on the key report do not reach the user report.
            let mut server = mockito::Server::new_async().await;
            stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
            stub(
                &mut server,
                &format!("/proxy/key/spend/report{KEY_QUERY}"),
                status,
                REPORT,
                1,
            )
            .await;
            let user = stub(&mut server, "/proxy/user/spend/report", 200, REPORT, 0).await;
            let Err(ProviderError::Other(message)) = run(&server, sept_21()).await else {
                panic!("expected an error for {status}");
            };
            assert!(message.contains(&status.to_string()), "{message}");
            assert!(!message.contains("synthetic-key-identifier"), "{message}");
            user.assert_async().await;
        }
    }

    #[tokio::test]
    async fn unavailable_user_report_surfaces_its_status_class() {
        for (status, expect_auth) in [(401, true), (403, true), (404, false)] {
            let mut server = mockito::Server::new_async().await;
            stub(&mut server, "/proxy/key/info", 403, "{}", 1).await;
            stub(
                &mut server,
                &format!("/proxy/key/spend/report{KEY_QUERY}"),
                404,
                "{}",
                1,
            )
            .await;
            stub(
                &mut server,
                &format!("/proxy/user/spend/report{KEY_QUERY}"),
                status,
                REPORT,
                1,
            )
            .await;
            match run(&server, sept_21()).await {
                Err(ProviderError::AuthRequired) if expect_auth => {}
                Err(ProviderError::Other(message)) if !expect_auth => {
                    assert!(message.contains("404"), "{message}");
                    assert!(!message.contains("synthetic-key-identifier"), "{message}");
                }
                other => panic!("status {status}: unexpected {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn key_info_success_is_unchanged() {
        let mut server = mockito::Server::new_async().await;
        let info = stub(
            &mut server,
            "/proxy/key/info",
            200,
            r#"{"info":{"spend":25.0,"max_budget":100.0}}"#,
            1,
        )
        .await;
        let report = stub(&mut server, "/proxy/key/spend/report", 200, REPORT, 0).await;
        let result = run(&server, sept_21()).await.unwrap();
        info.assert_async().await;
        report.assert_async().await;
        assert_eq!(result.usage.primary.used_percent, 25.0);
    }

    #[test]
    fn saved_base_url_uses_only_app_saved_key() {
        let mut ctx = FetchContext {
            workspace_id: Some("https://litellm.example.com".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            resolve_base_and_key(&ctx),
            Err(ProviderError::AuthRequired)
        ));

        ctx.api_key = Some("sk-app".to_string());
        let (base, key) = resolve_base_and_key(&ctx).unwrap();
        assert_eq!(base, "https://litellm.example.com");
        assert_eq!(key, "sk-app");
    }
}
