use super::*;

fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(action, shortcut)| (action.to_string(), shortcut.to_string()))
        .collect()
}

#[test]
fn defaults_resolve_unchanged() {
    let resolved = resolve(&BTreeMap::new()).unwrap();
    assert_eq!(resolved.len(), 11);
    assert_eq!(resolved["previous"], "left");
    assert_eq!(resolved["next"], "right");
    assert_eq!(resolved["select9"], "ctrl+9");
}

#[test]
fn normalize_orders_modifiers_and_folds_cmd() {
    assert_eq!(normalize("Shift+Ctrl+Right").unwrap(), "ctrl+shift+right");
    assert_eq!(normalize(" alt + ctrl + 2 ").unwrap(), "ctrl+alt+2");
    assert_eq!(normalize("alt+cmd+2").unwrap(), "ctrl+alt+2");
    assert_eq!(normalize("cmd+3").unwrap(), "ctrl+3");
    assert_eq!(normalize("Alt+,").unwrap(), "alt+,");
    assert_eq!(normalize("NONE").unwrap(), NONE);
    assert_eq!(normalize("left").unwrap(), "left");
    assert_eq!(normalize("shift+right").unwrap(), "shift+right");
}

#[test]
fn normalize_rejects_malformed_shortcuts() {
    for bad in [
        "",
        "ctrl+",
        "ctrl+ab",
        "ctrl+ctrl+1",
        "ctrl+cmd+1",
        "meta+1",
        "ctrl+f1",
        "ctrl+up",
        "alt+f4",
        "alt+tab",
        "ctrl+none",
    ] {
        assert!(
            matches!(normalize(bad), Err(SwitcherShortcutError::Invalid(_))),
            "{bad}"
        );
    }
}

#[test]
fn normalize_rejects_reserved_shortcuts() {
    for reserved in [
        "ctrl+r", "cmd+q", "ctrl+,", "ctrl+w", "a", "1", ",", "shift+a", "shift+1",
    ] {
        assert!(
            matches!(normalize(reserved), Err(SwitcherShortcutError::Reserved(_))),
            "{reserved}"
        );
    }
    assert_eq!(normalize("alt+a").unwrap(), "alt+a");
    assert_eq!(normalize("shift+ctrl+r").unwrap(), "ctrl+shift+r");
}

#[test]
fn resolve_overlays_and_normalizes_overrides() {
    let resolved = resolve(&overrides(&[
        ("select2", "alt+cmd+2"),
        ("next", "shift+right"),
    ]))
    .unwrap();
    assert_eq!(resolved["select2"], "ctrl+alt+2");
    assert_eq!(resolved["next"], "shift+right");
    assert_eq!(resolved["previous"], "left");
}

#[test]
fn resolve_rejects_unknown_actions() {
    assert_eq!(
        resolve(&overrides(&[("select10", "ctrl+0")])),
        Err(SwitcherShortcutError::UnknownAction("select10".to_string()))
    );
}

#[test]
fn resolve_rejects_duplicates_including_untouched_defaults() {
    assert_eq!(
        resolve(&overrides(&[("next", "left")])),
        Err(SwitcherShortcutError::Duplicate)
    );
    assert_eq!(
        resolve(&overrides(&[("select1", "cmd+2")])),
        Err(SwitcherShortcutError::Duplicate)
    );
}

#[test]
fn none_disables_an_action_and_frees_its_key() {
    let resolved = resolve(&overrides(&[("previous", "none"), ("next", "left")])).unwrap();
    assert_eq!(resolved["previous"], NONE);
    assert_eq!(resolved["next"], "left");
    assert!(resolve(&overrides(&[("select1", "none"), ("select2", "none")])).is_ok());
}

#[test]
fn normalize_overrides_keeps_only_non_default_entries() {
    let stored = normalize_overrides(&overrides(&[
        ("previous", "left"),
        ("select2", "alt+cmd+2"),
        ("next", "NONE"),
    ]))
    .unwrap();
    assert_eq!(
        stored,
        overrides(&[("select2", "ctrl+alt+2"), ("next", "none")])
    );
    assert!(normalize_overrides(&BTreeMap::new()).unwrap().is_empty());
}

mod persistence {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn default_settings_have_no_overrides_and_omit_the_key() {
        let settings = Settings::default();
        assert!(settings.switcher_shortcuts.is_empty());
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("switcher_shortcuts"));
    }

    #[test]
    fn overrides_roundtrip_through_settings_json() {
        let settings = Settings {
            switcher_shortcuts: overrides(&[("select2", "ctrl+alt+2"), ("next", "none")]),
            ..Settings::default()
        };
        let loaded: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(loaded.switcher_shortcuts, settings.switcher_shortcuts);
    }

    #[test]
    fn missing_key_loads_as_defaults() {
        let loaded: Settings = serde_json::from_str(r#"{ "enabled_providers": [] }"#).unwrap();
        assert!(loaded.switcher_shortcuts.is_empty());
    }

    #[test]
    fn hand_edited_map_is_normalized_on_load() {
        let loaded: Settings = serde_json::from_str(
            r#"{ "switcher_shortcuts": { "select2": "Alt+Cmd+2", "previous": "left" } }"#,
        )
        .unwrap();
        assert_eq!(
            loaded.switcher_shortcuts,
            overrides(&[("select2", "ctrl+alt+2")])
        );
    }

    #[test]
    fn invalid_stored_map_falls_back_to_defaults_without_breaking_load() {
        for stored in [
            r#"{ "bogus": "ctrl+1" }"#,
            r#"{ "next": "left" }"#,
            r#"{ "next": "ctrl+r" }"#,
            r#"{ "next": "f1" }"#,
        ] {
            let loaded: Settings = serde_json::from_str(&format!(
                r#"{{ "refresh_interval_secs": 120, "switcher_shortcuts": {stored} }}"#
            ))
            .unwrap();
            assert!(loaded.switcher_shortcuts.is_empty(), "{stored}");
            assert_eq!(loaded.refresh_interval_secs, 120);
        }
    }

    #[test]
    fn resolve_or_default_survives_invalid_overrides() {
        let resolved = resolve_or_default(&overrides(&[("next", "left")]));
        assert_eq!(resolved["next"], "right");
        assert_eq!(
            resolve_or_default(&overrides(&[("next", "alt+n")]))["next"],
            "alt+n"
        );
    }
}
