use super::*;
use crate::settings::{
    Language, ProviderConfig, ThemePreference, TrayIconMode, UsageThresholdOverride,
};
use serde_json::json;

fn document(preferences: Value) -> String {
    json!({ "version": 1, "preferences": preferences }).to_string()
}

fn parse(preferences: Value) -> Result<PreferencesDocument, PreferencesError> {
    PreferencesDocument::from_json(&document(preferences))
}

fn max_document_bytes() -> usize {
    usize::try_from(MAX_PREFERENCES_DOCUMENT_BYTES).expect("limit fits in usize")
}

/// Settings as JSON with the set-valued `enabled_providers` in stable order.
fn snapshot(settings: &Settings) -> Value {
    let mut value = serde_json::to_value(settings).expect("serialize settings");
    if let Some(Value::Array(ids)) = value.get_mut("enabled_providers") {
        ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    value
}

fn customized_settings() -> Settings {
    let mut settings = Settings {
        refresh_interval_secs: 900,
        adaptive_refresh: true,
        refresh_all_providers_on_menu_open: true,
        low_power_mode_preference: crate::settings::LowPowerModePreference::Automatic,
        show_notifications: false,
        sound_enabled: false,
        high_usage_threshold: 55.5,
        critical_usage_threshold: 80.0,
        predictive_pace_warning_enabled: true,
        show_pace: false,
        show_as_used: false,
        reset_time_relative: false,
        show_reset_when_exhausted: true,
        hide_personal_info: true,
        menu_bar_shows_highest_usage: true,
        menu_bar_shows_percent: true,
        tray_icon_mode: TrayIconMode::PerProvider,
        switcher_shows_icons: false,
        overview_layout: "detailed".to_string(),
        menu_bar_display_mode: "minimal".to_string(),
        // Loading normalizes the order to the full canonical list.
        provider_order: crate::settings::normalize_provider_order(&[
            "codex".to_string(),
            "claude".to_string(),
        ]),
        theme: ThemePreference::Dark,
        ui_language: Language::Japanese,
        window_scale_percent: 150,
        tray_scale_percent: 120,
        float_bar_opacity: 60,
        float_bar_scale: 120,
        float_bar_orientation: "vertical".to_string(),
        float_bar_style: "taskbar".to_string(),
        float_bar_dark_text: true,
        float_bar_show_reset_inline: true,
        float_bar_show_cost: true,
        ..Settings::default()
    };
    settings.enabled_providers = ["claude", "codex", "zai"].map(String::from).into();
    settings.provider_usage_thresholds.insert(
        "codex:weekly".to_string(),
        UsageThresholdOverride {
            high: Some(60.0),
            critical: None,
        },
    );
    settings.provider_metrics.insert(
        "claude".to_string(),
        crate::settings::MetricPreference::Weekly,
    );
    settings
}

/// Settings whose secret and machine-specific fields are all populated.
fn secret_settings() -> Settings {
    let mut settings = customized_settings();
    settings.provider_configs.insert(
        ProviderId::Claude,
        ProviderConfig {
            api_token: Some("SECRET-API-TOKEN".to_string()),
            manual_cookie_header: Some("sessionKey=SECRET-COOKIE".to_string()),
            management_api_token: Some("SECRET-MGMT".to_string()),
            ..ProviderConfig::default()
        },
    );
    settings.http_proxy_enabled = true;
    settings.http_proxy_url = "http://proxy.invalid:8080".to_string();
    settings.http_proxy_username = "SECRET-PROXY-USER".to_string();
    settings.http_proxy_password = "SECRET-PROXY-PASSWORD".to_string();
    settings.codex_custom_sessions_dirs = vec!["C:\\secret\\sessions".to_string()];
    settings.agent_session_ssh_hosts = vec!["secret-host.invalid".to_string()];
    settings.notification_sound_paths.high_usage = Some("C:\\secret\\ding.wav".to_string());
    settings
}

#[test]
fn round_trips_every_allowlisted_preference() {
    let source = customized_settings();
    let exported = PreferencesDocument::from_settings(&source).expect("export");
    assert_eq!(exported.len(), ALLOWED_PREFERENCES.len());

    let reparsed = PreferencesDocument::from_json(&exported.to_json()).expect("reimport");
    assert_eq!(reparsed, exported);

    let mut target = Settings {
        start_at_login: true,
        global_shortcut: "Ctrl+Alt+K".to_string(),
        ..Settings::default()
    };
    let applied = reparsed.apply_to(&mut target).expect("apply");
    assert_eq!(applied, ALLOWED_PREFERENCES.len());

    let again = PreferencesDocument::from_settings(&target).expect("re-export");
    assert_eq!(again, exported);
    assert_eq!(target.window_scale_percent, 150);
    assert_eq!(target.ui_language, Language::Japanese);
    assert_eq!(
        target.enabled_providers,
        ["claude", "codex", "zai"].map(String::from).into()
    );
    // Non-portable state on the receiving machine is untouched.
    assert!(target.start_at_login);
    assert_eq!(target.global_shortcut, "Ctrl+Alt+K");
}

#[test]
fn export_is_deterministic_and_sorted() {
    let first = PreferencesDocument::from_settings(&customized_settings())
        .expect("export")
        .to_json();
    let second = PreferencesDocument::from_settings(&customized_settings())
        .expect("export")
        .to_json();
    assert_eq!(first, second);
    assert!(first.ends_with("}\n"));
    let claude = first.find("\"claude\"").expect("claude listed");
    let codex = first.find("\"codex\"").expect("codex listed");
    let zai = first.find("\"zai\"").expect("zai listed");
    assert!(claude < codex && codex < zai);
}

#[test]
fn rejection_matrix_names_the_offending_key() {
    let cases = [
        (
            "unknown key",
            json!({ "not_a_preference": true }),
            "not_a_preference",
        ),
        (
            "provider_configs smuggled in",
            json!({ "provider_configs": { "claude": { "api_token": "x" } } }),
            "provider_configs",
        ),
        (
            "http proxy password",
            json!({ "http_proxy_password": "x" }),
            "http_proxy_password",
        ),
        (
            "start at login",
            json!({ "start_at_login": true }),
            "start_at_login",
        ),
        ("bad enum", json!({ "theme": "sepia" }), "theme"),
        (
            "bad language",
            json!({ "ui_language": "klingon" }),
            "ui_language",
        ),
        (
            "bad layout",
            json!({ "overview_layout": "huge" }),
            "overview_layout",
        ),
        (
            "threshold over 100",
            json!({ "high_usage_threshold": 100.5 }),
            "high_usage_threshold",
        ),
        (
            "negative threshold",
            json!({ "critical_usage_threshold": -1 }),
            "critical_usage_threshold",
        ),
        (
            "threshold as string",
            json!({ "high_usage_threshold": "70" }),
            "high_usage_threshold",
        ),
        (
            "bool as string",
            json!({ "show_pace": "true" }),
            "show_pace",
        ),
        ("bool as number", json!({ "show_pace": 1 }), "show_pace"),
        (
            "scale too small",
            json!({ "window_scale_percent": 99 }),
            "window_scale_percent",
        ),
        (
            "scale too large",
            json!({ "tray_scale_percent": 201 }),
            "tray_scale_percent",
        ),
        (
            "scale fractional",
            json!({ "window_scale_percent": 100.5 }),
            "window_scale_percent",
        ),
        (
            "interval below a minute",
            json!({ "refresh_interval_secs": 5 }),
            "refresh_interval_secs",
        ),
        (
            "interval above a day",
            json!({ "refresh_interval_secs": 86_401 }),
            "refresh_interval_secs",
        ),
        (
            "unknown provider id",
            json!({ "enabled_providers": ["nope"] }),
            "enabled_providers",
        ),
        (
            "duplicate provider id",
            json!({ "provider_order": ["codex", "codex"] }),
            "provider_order",
        ),
        (
            "provider alias",
            json!({ "enabled_providers": ["openai"] }),
            "enabled_providers",
        ),
        (
            "metric value",
            json!({ "provider_metrics": { "codex": "bogus" } }),
            "provider_metrics",
        ),
        (
            "metric provider",
            json!({ "provider_metrics": { "nope": "weekly" } }),
            "provider_metrics",
        ),
        (
            "threshold window",
            json!({ "provider_usage_thresholds": { "codex:monthly": { "high": 50 } } }),
            "provider_usage_thresholds",
        ),
        (
            "threshold field",
            json!({ "provider_usage_thresholds": { "codex": { "low": 50 } } }),
            "provider_usage_thresholds",
        ),
        (
            "threshold range",
            json!({ "provider_usage_thresholds": { "codex": { "high": 150 } } }),
            "provider_usage_thresholds",
        ),
    ];
    for (name, preferences, key) in cases {
        assert_eq!(
            parse(preferences).expect_err(name),
            PreferencesError::InvalidPreference(key.to_string()),
            "{name}"
        );
    }
}

#[test]
fn rejects_bad_envelopes() {
    for text in [
        r#"{"version":2,"preferences":{}}"#,
        r#"{"version":"1","preferences":{}}"#,
        r#"{"preferences":{}}"#,
    ] {
        assert_eq!(
            PreferencesDocument::from_json(text),
            Err(PreferencesError::UnsupportedVersion),
            "{text}"
        );
    }
    for text in [
        "not json",
        "[]",
        r#"{"version":1}"#,
        r#"{"version":1,"preferences":[]}"#,
        r#"{"version":1,"preferences":{},"extra":true}"#,
    ] {
        assert_eq!(
            PreferencesDocument::from_json(text),
            Err(PreferencesError::Malformed),
            "{text}"
        );
    }
    let oversized = format!(
        r#"{{"version":1,"preferences":{{}},"pad":"{}"}}"#,
        "x".repeat(max_document_bytes())
    );
    assert_eq!(
        PreferencesDocument::from_json(&oversized),
        Err(PreferencesError::TooLarge)
    );
}

#[test]
fn errors_never_echo_values() {
    let error = parse(json!({ "theme": "TOP-SECRET-VALUE" })).expect_err("bad theme");
    assert!(!error.to_string().contains("TOP-SECRET-VALUE"));
    let error = PreferencesDocument::from_json("TOP-SECRET-VALUE").expect_err("not json");
    assert!(!error.to_string().contains("TOP-SECRET-VALUE"));
}

#[test]
fn missing_keys_leave_settings_unchanged() {
    let empty = parse(json!({})).expect("empty document");
    let mut settings = customized_settings();
    // The first apply normalizes state the way a settings load does.
    empty.apply_to(&mut settings).expect("apply");
    let before = snapshot(&settings);
    assert_eq!(empty.apply_to(&mut settings).expect("apply"), 0);
    assert_eq!(snapshot(&settings), before);

    parse(json!({ "theme": "light" }))
        .expect("theme")
        .apply_to(&mut settings)
        .expect("apply");
    assert_eq!(settings.theme, ThemePreference::Light);
    assert_eq!(settings.window_scale_percent, 150);
}

#[test]
fn null_restores_the_default() {
    let mut settings = customized_settings();
    let applied = parse(json!({
        "window_scale_percent": null,
        "theme": null,
        "enabled_providers": null,
        "provider_usage_thresholds": null,
        "provider_metrics": null,
        "provider_order": null,
        "show_pace": null,
    }))
    .expect("nulls are valid")
    .apply_to(&mut settings)
    .expect("apply");
    assert_eq!(applied, 7);

    let defaults = Settings::default();
    assert_eq!(settings.window_scale_percent, defaults.window_scale_percent);
    assert_eq!(settings.theme, defaults.theme);
    assert_eq!(settings.enabled_providers, defaults.enabled_providers);
    assert!(settings.provider_usage_thresholds.is_empty());
    assert!(settings.provider_metrics.is_empty());
    assert!(settings.provider_order.is_empty());
    assert!(settings.show_pace);
    // Untouched preferences keep their customized values.
    assert_eq!(settings.tray_scale_percent, 120);
}

#[test]
fn allowed_and_excluded_keys_classify_every_settings_field() {
    let Value::Object(serialized) = snapshot(&secret_settings()) else {
        panic!("settings must serialize to an object");
    };
    let allowed: HashSet<&str> = ALLOWED_PREFERENCES.iter().map(|(key, _)| *key).collect();
    let excluded: HashSet<&str> = EXCLUDED_KEYS.iter().copied().collect();

    assert_eq!(
        allowed.len(),
        ALLOWED_PREFERENCES.len(),
        "duplicate allowlist key"
    );
    assert_eq!(
        excluded.len(),
        EXCLUDED_KEYS.len(),
        "duplicate excluded key"
    );
    assert!(
        allowed.is_disjoint(&excluded),
        "a key is both allowed and excluded"
    );

    for key in serialized.keys() {
        assert!(
            allowed.contains(key.as_str()) || excluded.contains(key.as_str()),
            "Settings field `{key}` is not classified: add it to ALLOWED_PREFERENCES \
             (portable) or EXCLUDED_KEYS (never exported)"
        );
    }
    for key in allowed.iter().chain(excluded.iter()) {
        assert!(
            serialized.contains_key(*key),
            "`{key}` is classified but is not a Settings field"
        );
    }
}

#[test]
fn export_never_contains_secrets_or_machine_state() {
    let text = PreferencesDocument::from_settings(&secret_settings())
        .expect("export")
        .to_json();
    for secret in [
        "SECRET",
        "proxy.invalid",
        "secret-host",
        "secret\\\\",
        "sessionKey",
        "provider_configs",
        "http_proxy",
        "start_at_login",
        "global_shortcut",
    ] {
        assert!(!text.contains(secret), "export leaked `{secret}`");
    }
}

#[test]
fn applying_keeps_everything_else_on_the_receiving_machine() {
    let empty = parse(json!({})).expect("empty");
    let mut target = secret_settings();
    empty.apply_to(&mut target).expect("apply");
    let before = snapshot(&target);
    // The whole-struct JSON round trip used by apply is lossless once normalized.
    empty.apply_to(&mut target).expect("apply");
    assert_eq!(snapshot(&target), before);

    parse(json!({ "theme": "light" }))
        .expect("theme")
        .apply_to(&mut target)
        .expect("apply");
    let config = target
        .provider_configs
        .get(&ProviderId::Claude)
        .expect("config");
    assert_eq!(config.api_token.as_deref(), Some("SECRET-API-TOKEN"));
    assert_eq!(target.http_proxy_password, "SECRET-PROXY-PASSWORD");
}

#[test]
fn file_round_trip_and_size_bound() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("preferences.json");
    let exported = PreferencesDocument::from_settings(&customized_settings()).expect("export");
    exported.write_file(&path).expect("write");
    assert_eq!(
        PreferencesDocument::read_file(&path).expect("read"),
        exported
    );

    let big = dir.path().join("big.json");
    std::fs::write(&big, vec![b' '; max_document_bytes() + 10]).expect("write");
    assert_eq!(
        PreferencesDocument::read_file(&big),
        Err(PreferencesError::TooLarge)
    );

    assert!(matches!(
        PreferencesDocument::read_file(&dir.path().join("missing.json")),
        Err(PreferencesError::Io(_))
    ));
}
