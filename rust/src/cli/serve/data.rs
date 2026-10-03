//! `/usage` and `/cost` data route handlers.
//!
//! Moved verbatim from the pre-0.48.0 serve module; the only 0.48.0 change is
//! the additive `daily` field on `/cost` — the web dashboard's daily spend bar
//! charts ride this array (upstream #2722 fetches `/cost` for the same data).

use serde_json::json;

use crate::cli::cost_period::{cost_totals_json, rolling_window_days, stamp_period, window_days};
use crate::cli::fetch_context::populate_api_region_from_settings;
use crate::cli::usage::ProviderSelection;
use crate::core::{CostScanOptions, FetchContext, ProviderId, SourceMode, instantiate_provider};
use crate::cost_scanner::{self, CostScanner};
use crate::settings::Settings;

use super::json_response;

pub async fn usage_response(provider: Option<&str>) -> String {
    let selection = match ProviderSelection::from_arg(provider) {
        Ok(selection) => selection,
        Err(error) => {
            return json_response(400, json!({ "error": error.to_string() }));
        }
    };
    let ctx = FetchContext {
        source_mode: SourceMode::Auto,
        include_credits: true,
        web_timeout: 60,
        verbose: false,
        manual_cookie_header: None,
        manual_cookie_missing: false,
        api_key: None,
        token_account_kind: None,
        token_account_isolated: false,
        workspace_id: None,
        seat_credit_entitlement: None,
        api_region: None,
        gateway_url: None,
        auto_prefer_web: false,
        // Serve `/usage` is a background poll read: keep the short optional-
        // join grace (upstream #2583), unlike `codexbar usage` which blocks
        // for the full completeness window.
        requires_optional_usage_completeness: false,
    };
    let settings = Settings::load();

    let mut results = Vec::new();
    for provider_id in selection.as_list() {
        let provider = instantiate_provider(provider_id);
        let mut provider_ctx = ctx.clone();
        populate_api_region_from_settings(provider_id, &settings, &mut provider_ctx);
        match provider.fetch_usage(&provider_ctx).await {
            Ok(result) => results.push(json!({
                "provider": provider_id.cli_name(),
                "source": result.source_label,
                "usage": result.usage,
                "cost": result.cost,
            })),
            Err(error) => results.push(json!({
                "provider": provider_id.cli_name(),
                "error": error.to_string(),
            })),
        }
    }
    json_response(200, serde_json::Value::Array(results))
}

pub async fn cost_response(provider: Option<&str>) -> String {
    let selection = match ProviderSelection::from_arg(provider) {
        Ok(selection) => selection,
        Err(error) => {
            return json_response(400, json!({ "error": error.to_string() }));
        }
    };
    // The saved selection is read per request, so a change in Settings (or a
    // month rollover for month to date) applies without restarting `serve`.
    // There is no `/cost` response cache; the scanners' own caches are keyed by
    // the resolved day range, so one period never reuses another's entries.
    let period = Settings::load().cost_reporting_period;
    let days = window_days(period);
    let scanner = CostScanner::for_period(period).with_options(CostScanOptions::app_driven());
    let mut results = Vec::new();
    for provider_id in selection.as_list() {
        if provider_id == ProviderId::Antigravity {
            use crate::providers::antigravity::local_sessions;
            let history = local_sessions::summarize(days);
            // Upstream 0.64 `serve` refreshes unknown-model pricing in the
            // background; a later `/cost` read picks up the new prices.
            if let Some(refresh) = local_sessions::background_pricing_refresh(&history) {
                tokio::spawn(refresh);
            }
            let mut payload =
                crate::spend_contract::local_token_history_json("antigravity", &history, days);
            stamp_period(&mut payload, period);
            results.push(payload);
            continue;
        }
        if provider_id == ProviderId::Muse {
            let report = crate::providers::muse::local_usage::scan(days, None);
            let history: crate::spend_contract::LocalTokenHistorySummary = report.into();
            let mut payload =
                crate::spend_contract::local_token_history_json("muse", &history, days);
            stamp_period(&mut payload, period);
            results.push(payload);
            continue;
        }
        let (supported, summary, daily) = match provider_id {
            ProviderId::Codex => (true, scanner.scan_codex(), None),
            ProviderId::Claude => {
                let snapshot = scanner.scan_claude_chart_snapshot_with_cancel(None);
                (true, snapshot.summary, Some(snapshot.daily_cost))
            }
            ProviderId::Pi => (true, scanner.scan_pi(), None),
            _ => (false, Default::default(), None),
        };
        if supported {
            // Claude's snapshot derives the summary and chart rows in one
            // transcript walk. Other providers use the shared daily-history
            // path, capped at one year for All.
            let daily = daily.unwrap_or_else(|| {
                cost_scanner::get_daily_cost_history(
                    provider_id.cli_name(),
                    rolling_window_days(period),
                )
            });
            let daily = daily_json(daily);
            let mut payload = json!({
                "provider": provider_id.cli_name(),
                "supported": true,
                "days_scanned": days,
                "totals": cost_totals_json(provider_id.cli_name(), &summary),
                "cost": {
                    "total_usd": summary.total_cost_usd,
                    "currency": "USD"
                },
                "daily": daily,
                "tokens": {
                    "input": summary.input_tokens,
                    "output": summary.output_tokens,
                    "cached": summary.cached_tokens
                },
                "sessions_count": summary.sessions_count,
                "by_model": summary.by_model,
            });
            stamp_period(&mut payload, period);
            results.push(payload);
        } else {
            results.push(json!({
                "provider": provider_id.cli_name(),
                "supported": false,
                "error": "Local cost scanning not available for this provider"
            }));
        }
    }
    json_response(200, serde_json::Value::Array(results))
}

/// Dashboard-charts shape for one provider's daily spend: [{date, totalCost}].
fn daily_json(daily: Vec<(String, Option<f64>)>) -> serde_json::Value {
    serde_json::Value::Array(
        daily
            .into_iter()
            .map(|(date, cost_usd)| json!({ "date": date, "totalCost": cost_usd }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_array_shape_matches_dashboard_charts_contract() {
        let daily = daily_json(vec![
            ("2026-08-07".to_string(), Some(0.0)),
            ("2026-08-08".to_string(), Some(4.25)),
            ("2026-08-09".to_string(), None),
        ]);
        let rows = daily.as_array().unwrap();
        assert_eq!(rows[0]["date"], "2026-08-07");
        assert_eq!(rows[1]["totalCost"], 4.25);
        assert_eq!(rows[0]["totalCost"], 0.0);
        assert!(rows[2]["totalCost"].is_null());
    }

    #[test]
    fn antigravity_cost_payload_is_token_only_and_preserves_partial_unknown() {
        use crate::spend_contract::{LocalHistoryCoverage, LocalTokenHistorySummary};
        let complete = crate::spend_contract::local_token_history_json(
            "antigravity",
            &LocalTokenHistorySummary {
                total_tokens: 42,
                session_count: 1,
                coverage: LocalHistoryCoverage::Complete,
                cost_estimate: Default::default(),
                ..Default::default()
            },
            30,
        );
        assert!(complete["cost"]["total_usd"].is_null());
        assert_eq!(complete["tokens"]["total"], 42);
        assert_eq!(complete["historyCoverage"], "complete");

        let partial = crate::spend_contract::local_token_history_json(
            "antigravity",
            &LocalTokenHistorySummary {
                total_tokens: 42,
                session_count: 1,
                coverage: LocalHistoryCoverage::Partial,
                cost_estimate: Default::default(),
                ..Default::default()
            },
            30,
        );
        assert!(partial["tokens"]["total"].is_null());
        assert_eq!(partial["historyCoverage"], "partial");
    }
    #[test]
    fn daily_rows_use_upstream_total_cost_key_only() {
        let daily = daily_json(vec![
            ("2026-08-07".to_string(), Some(0.0)),
            ("2026-08-08".to_string(), Some(4.25)),
            ("2026-08-09".to_string(), None),
        ]);
        let serialized = daily.to_string();
        assert!(
            serialized.contains("\"totalCost\""),
            "wire key is totalCost"
        );
        assert!(
            !serialized.contains("cost_usd") && !serialized.contains("costUSD"),
            "no stale daily cost keys may leak to the wire"
        );
    }

    #[test]
    fn daily_empty_array_has_no_rows() {
        let daily = daily_json(vec![]);
        assert_eq!(daily.as_array().unwrap().len(), 0);
    }

    #[test]
    fn daily_zero_values_are_preserved_not_filtered() {
        let daily = daily_json(vec![("2026-08-07".to_string(), Some(0.0))]);
        let rows = daily.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["totalCost"], 0.0);
    }
}
