//! Upstream 0.53 Usage & Spend accounting bridge.

use codexbar::cost_reporting_period::CostReportingPeriod;
use codexbar::cost_scanner::CostScanner;
use codexbar::settings::Settings;
use codexbar::spend_contract::{SpendContract, build_contract_from_period_summary};

/// `period` is a raw reporting period (`rolling:N`, `month-to-date`, `all`).
/// `history_days` is the legacy rolling count (`0` meant All). With neither,
/// the saved `cost_reporting_period` applies.
#[tauri::command]
pub async fn get_spend_contract(
    provider_id: String,
    history_days: Option<u32>,
    period: Option<String>,
    include_open_codex: Option<bool>,
) -> Result<SpendContract, String> {
    let provider = provider_id.trim().to_ascii_lowercase();
    if !matches!(provider.as_str(), "codex" | "claude" | "pi" | "opencodego") {
        return Err(format!(
            "Spend contract is unavailable for provider: {provider}"
        ));
    }
    let include_import = include_open_codex.unwrap_or(false) && provider == "codex";
    tauri::async_runtime::spawn_blocking(move || {
        let settings = Settings::load();
        let period = CostReportingPeriod::resolve_request(
            period.as_deref(),
            history_days,
            settings.cost_reporting_period,
        );
        let scanner = CostScanner::for_period(period);
        let summary = match provider.as_str() {
            "codex" => scanner.scan_codex(),
            "claude" => scanner.scan_claude(),
            "pi" => scanner.scan_pi(),
            "opencodego" => scanner.scan_opencodego_with_cancel(None),
            _ => unreachable!(),
        };
        build_contract_from_period_summary(
            &provider,
            period,
            include_import,
            settings.hide_native_codex_cost_when_open_codex_present && provider == "codex",
            settings.hide_personal_info,
            summary,
        )
    })
    .await
    .map_err(|error| format!("spend contract worker failed: {error}"))
}
