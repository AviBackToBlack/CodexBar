//! `get_safe_diagnostics` — a copy-friendly, secret-free diagnostics string
//! for bug reports. Contains only: app version/build, OS, update channel,
//! log directory, and the redacted log tail. Never includes provider names,
//! emails, plans, account info, cookies, or tokens.

use super::*;

#[tauri::command]
pub fn get_safe_diagnostics() -> String {
    let settings = Settings::load();
    let os = format!(
        "{} {}",
        std::env::consts::OS,
        std::env::var("OS").unwrap_or_default()
    );
    let log_dir = codexbar::logging::log_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unresolvable".to_string());
    let log_tail =
        codexbar::logging::read_log_tail().unwrap_or_else(|| "log file unavailable".to_string());

    format!(
        "CodexBar diagnostics\n\
         -------------------\n\
         version: {} (build {})\n\
         os: {}\n\
         channel: {}\n\
         log dir: {}\n\
         --- last log lines (redacted) ---\n\
         {}",
        env!("CARGO_PKG_VERSION"),
        option_env!("BUILD_NUMBER").unwrap_or("dev"),
        os,
        update_channel_label(settings.update_channel),
        log_dir,
        log_tail,
    )
}
