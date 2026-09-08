use std::collections::HashSet;

use crate::commands::ProviderCatalogEntry;
use codexbar::codex_accounts::CodexAccount;
use codexbar::locale::{self, LocaleKey};
use codexbar::settings::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayMenuEntry {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) children: Vec<Self>,
    pub(crate) is_separator: bool,
    pub(crate) disabled: bool,
    /// When `Some`, this entry renders as a check/checkbox item.
    /// `true` = checked (enabled), `false` = unchecked (disabled).
    pub(crate) checked: Option<bool>,
}

pub(crate) fn codex_accounts_menu(
    accounts: &[CodexAccount],
    active: Option<&CodexAccount>,
    lang: Language,
    hide_personal_info: bool,
) -> TrayMenuEntry {
    let text = |key| locale::get_text(lang, key);
    let mut children: Vec<_> = accounts
        .iter()
        .map(|account| {
            let is_active = active.is_some_and(|current| current.matches(account));
            let mut entry = TrayMenuEntry::check_item(
                format!("switch_codex_account:{}", account.id),
                if hide_personal_info
                    && account
                        .nickname
                        .as_deref()
                        .is_none_or(|n| n.trim().is_empty())
                {
                    codexbar::core::PersonalInfoRedactor::partial_redact_email(
                        account.email_hint.as_deref(),
                        true,
                    )
                } else {
                    account.display_name()
                },
                is_active,
            );
            entry.disabled = is_active;
            entry
        })
        .collect();
    if children.is_empty() {
        children.push(TrayMenuEntry::status_row(
            "codex_accounts_empty",
            text(LocaleKey::CodexAccountsEmpty),
        ));
    }
    children.push(TrayMenuEntry::separator());
    children.push(TrayMenuEntry::item(
        "add_codex_account",
        text(LocaleKey::CodexAccountsAddButton),
    ));
    TrayMenuEntry::submenu(
        "codex_accounts",
        text(LocaleKey::CodexAccountsTitle),
        children,
    )
}

pub(crate) fn claude_accounts_menu(
    accounts: &[codexbar::providers::claude::accounts::ClaudeAccount],
    lang: Language,
    hide_personal_info: bool,
) -> TrayMenuEntry {
    let text = |key| locale::get_text(lang, key);
    let mut children: Vec<_> = accounts
        .iter()
        .map(|account| {
            let label = if hide_personal_info {
                codexbar::core::PersonalInfoRedactor::partial_redact_email(
                    Some(&account.email),
                    true,
                )
            } else {
                account.organization.as_ref().map_or_else(
                    || account.email.clone(),
                    |org| format!("{} ({org})", account.email),
                )
            };
            let mut entry = TrayMenuEntry::check_item(
                format!("switch_claude_account:{}", account.id),
                label,
                account.is_active,
            );
            entry.disabled = account.is_active || !account.is_saved;
            entry
        })
        .collect();
    if children.is_empty() {
        children.push(TrayMenuEntry::status_row(
            "claude_accounts_empty",
            text(LocaleKey::ClaudeAccountsEmpty),
        ));
    }
    children.push(TrayMenuEntry::separator());
    children.push(TrayMenuEntry::item(
        "add_claude_account",
        text(LocaleKey::CodexAccountsAddButton),
    ));
    children.push(TrayMenuEntry::item(
        "save_claude_account",
        text(LocaleKey::ClaudeAccountsSaveCurrent),
    ));
    children.push(TrayMenuEntry::item(
        "cancel_claude_login",
        text(LocaleKey::ClaudeAccountsCancelLogin),
    ));
    TrayMenuEntry::submenu(
        "claude_accounts",
        text(LocaleKey::ClaudeAccountsTitle),
        children,
    )
}

impl TrayMenuEntry {
    fn item(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            children: Vec::new(),
            is_separator: false,
            disabled: false,
            checked: None,
        }
    }

    /// A checkbox menu item. `checked` mirrors the provider's enabled state.
    fn check_item(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            children: Vec::new(),
            is_separator: false,
            disabled: false,
            checked: Some(checked),
        }
    }

    fn submenu(id: impl Into<String>, label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            children,
            is_separator: false,
            disabled: false,
            checked: None,
        }
    }

    fn separator() -> Self {
        Self {
            id: None,
            label: String::new(),
            children: Vec::new(),
            is_separator: true,
            disabled: false,
            checked: None,
        }
    }

    pub(crate) fn status_row(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            children: Vec::new(),
            is_separator: false,
            disabled: true,
            checked: None,
        }
    }
}

#[cfg(test)]
pub(crate) fn build_tray_menu(
    providers: &[ProviderCatalogEntry],
    status_labels: &[(String, String)],
    enabled_providers: &HashSet<String>,
) -> Vec<TrayMenuEntry> {
    build_tray_menu_with(
        providers,
        status_labels,
        enabled_providers,
        false,
        Language::English,
    )
}

pub(crate) fn build_tray_menu_with(
    providers: &[ProviderCatalogEntry],
    status_labels: &[(String, String)],
    enabled_providers: &HashSet<String>,
    float_bar_enabled: bool,
    lang: Language,
) -> Vec<TrayMenuEntry> {
    let mut menu: Vec<TrayMenuEntry> = Vec::new();
    let text = |key| locale::get_text(lang, key);

    // Status rows (one per enabled provider with live usage).
    for (id, label) in status_labels {
        menu.push(TrayMenuEntry::status_row(format!("status_{id}"), label));
    }
    if !status_labels.is_empty() {
        menu.push(TrayMenuEntry::separator());
    }

    menu.push(TrayMenuEntry::item(
        "refresh",
        text(LocaleKey::TrayRefreshAll),
    ));
    menu.push(TrayMenuEntry::item(
        "pop_out",
        text(LocaleKey::TrayPopOutDashboard),
    ));
    menu.push(TrayMenuEntry::item(
        "show_panel",
        text(LocaleKey::TrayShowWindow),
    ));
    menu.push(TrayMenuEntry::check_item(
        "toggle_float_bar",
        text(LocaleKey::TrayShowFloatBar),
        float_bar_enabled,
    ));
    menu.push(TrayMenuEntry::separator());

    if !providers.is_empty() {
        menu.push(TrayMenuEntry::submenu(
            "providers",
            text(LocaleKey::TrayProviders),
            providers
                .iter()
                .map(|provider| {
                    let is_enabled = enabled_providers.contains(&provider.id);
                    TrayMenuEntry::check_item(
                        format!("toggle_provider:{}", provider.id),
                        &provider.display_name,
                        is_enabled,
                    )
                })
                .collect(),
        ));
        menu.push(TrayMenuEntry::separator());
    }

    menu.push(TrayMenuEntry::item(
        "settings",
        text(LocaleKey::TraySettings),
    ));
    menu.push(TrayMenuEntry::item(
        "check_for_updates",
        text(LocaleKey::TrayCheckForUpdates),
    ));
    menu.push(TrayMenuEntry::item("about", text(LocaleKey::MenuAbout)));
    menu.push(TrayMenuEntry::separator());
    menu.push(TrayMenuEntry::item("quit", text(LocaleKey::MenuQuit)));

    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_accounts_can_be_switched_or_added_from_tray() {
        use codexbar::codex_accounts::{CodexAccountSource, utc_now};
        let make = |name: &str| {
            CodexAccount::new(
                uuid::Uuid::new_v4(),
                Some(name.into()),
                None,
                None,
                Some(name.into()),
                std::path::PathBuf::from(name),
                CodexAccountSource::ManagedByApp,
                utc_now(),
                utc_now(),
                None,
            )
        };
        let first = make("Personal");
        let second = make("Work");
        let menu = codex_accounts_menu(
            &[first.clone(), second.clone()],
            Some(&first),
            Language::English,
            false,
        );
        assert_eq!(menu.children[0].checked, Some(true));
        assert!(menu.children[0].disabled);
        assert_eq!(
            menu.children[1].id.as_deref(),
            Some(format!("switch_codex_account:{}", second.id).as_str())
        );
        assert_eq!(menu.children[1].checked, Some(false));
        assert!(!menu.children[1].disabled);
        assert!(menu_contains(&menu.children, "add_codex_account"));
        let empty = codex_accounts_menu(&[], None, Language::English, false);
        assert!(menu_contains(&empty.children, "add_codex_account"));
        let mut email_account = second;
        email_account.nickname = None;
        email_account.email_hint = Some("private@example.com".into());
        let private = codex_accounts_menu(&[email_account.clone()], None, Language::English, true);
        assert!(!private.children[0].label.contains("private@example.com"));
        let visible = codex_accounts_menu(&[email_account], None, Language::English, false);
        assert_eq!(visible.children[0].label, "private@example.com");
    }

    #[test]
    fn claude_menu_checks_current_account_and_routes_saved_accounts() {
        use codexbar::providers::claude::accounts::ClaudeAccount;
        let current = ClaudeAccount {
            id: "a:org".into(),
            email: "a@example.com".into(),
            organization: None,
            plan: None,
            is_active: true,
            is_saved: false,
        };
        let saved = ClaudeAccount {
            id: "b:org".into(),
            is_active: false,
            is_saved: true,
            ..current.clone()
        };
        let menu = claude_accounts_menu(&[current, saved], Language::English, false);
        assert_eq!(menu.id.as_deref(), Some("claude_accounts"));
        assert_eq!(menu.children[0].checked, Some(true));
        assert!(menu.children[0].disabled);
        assert_eq!(
            menu.children[1].id.as_deref(),
            Some("switch_claude_account:b:org")
        );
        assert!(!menu.children[1].disabled);
        assert!(menu_contains(&menu.children, "add_claude_account"));
        assert!(menu_contains(
            &claude_accounts_menu(&[], Language::English, false).children,
            "add_claude_account"
        ));
    }

    fn menu_contains(menu: &[TrayMenuEntry], id: &str) -> bool {
        menu.iter().any(|entry| {
            entry.id.as_deref() == Some(id)
                || (!entry.children.is_empty() && menu_contains(&entry.children, id))
        })
    }

    fn sample_provider_catalog() -> Vec<ProviderCatalogEntry> {
        vec![
            ProviderCatalogEntry {
                id: "codex".into(),
                display_name: "Codex".into(),
                cookie_domain: None,
            },
            ProviderCatalogEntry {
                id: "claude".into(),
                display_name: "Claude".into(),
                cookie_domain: None,
            },
        ]
    }

    fn both_enabled() -> HashSet<String> {
        ["codex".to_string(), "claude".to_string()]
            .into_iter()
            .collect()
    }

    #[test]
    fn check_for_updates_item_is_present() {
        let menu = build_tray_menu(&sample_provider_catalog(), &[], &both_enabled());
        assert!(menu_contains(&menu, "check_for_updates"));
    }

    #[test]
    fn provider_check_items_reflect_enabled_state() {
        let menu = build_tray_menu(
            &sample_provider_catalog(),
            &[],
            &["claude".to_string()].into_iter().collect(),
        );
        let providers_submenu = menu
            .iter()
            .find(|e| e.id.as_deref() == Some("providers"))
            .expect("providers submenu");

        let claude_item = providers_submenu
            .children
            .iter()
            .find(|e| e.id.as_deref() == Some("toggle_provider:claude"))
            .expect("claude item");
        let codex_item = providers_submenu
            .children
            .iter()
            .find(|e| e.id.as_deref() == Some("toggle_provider:codex"))
            .expect("codex item");

        assert_eq!(claude_item.checked, Some(true), "Claude should be checked");
        assert_eq!(codex_item.checked, Some(false), "Codex should be unchecked");
    }

    #[test]
    fn float_bar_toggle_reflects_state() {
        let menu_on = build_tray_menu_with(
            &sample_provider_catalog(),
            &[],
            &both_enabled(),
            /* float_bar_enabled = */ true,
            Language::English,
        );
        let toggle = menu_on
            .iter()
            .find(|e| e.id.as_deref() == Some("toggle_float_bar"))
            .expect("float bar toggle present");
        assert_eq!(toggle.checked, Some(true));
        assert_eq!(toggle.label, "Show Float Bar");

        let menu_off = build_tray_menu_with(
            &sample_provider_catalog(),
            &[],
            &both_enabled(),
            /* float_bar_enabled = */ false,
            Language::English,
        );
        let toggle = menu_off
            .iter()
            .find(|e| e.id.as_deref() == Some("toggle_float_bar"))
            .expect("float bar toggle present");
        assert_eq!(toggle.checked, Some(false));
    }

    #[test]
    fn tray_menu_static_labels_follow_language_but_provider_names_stay_raw() {
        let menu = build_tray_menu_with(
            &sample_provider_catalog(),
            &[],
            &both_enabled(),
            false,
            Language::Japanese,
        );
        fn label_for<'a>(menu: &'a [TrayMenuEntry], id: &'a str) -> &'a str {
            menu.iter()
                .find(|e| e.id.as_deref() == Some(id))
                .map(|e| e.label.as_str())
                .expect(id)
        }

        assert_eq!(label_for(&menu, "refresh"), "すべて更新");
        assert_eq!(label_for(&menu, "show_panel"), "ウィンドウを表示");
        assert_eq!(label_for(&menu, "settings"), "設定...");
        assert_eq!(label_for(&menu, "quit"), "終了");

        let providers = menu
            .iter()
            .find(|e| e.id.as_deref() == Some("providers"))
            .expect("providers submenu");
        let provider_labels: Vec<&str> = providers
            .children
            .iter()
            .map(|e| e.label.as_str())
            .collect();
        assert_eq!(provider_labels, vec!["Codex", "Claude"]);
    }

    #[test]
    fn status_rows_appear_at_top_with_separator() {
        let labels = vec![
            ("claude".to_string(), "Claude 60%".to_string()),
            ("codex".to_string(), "Codex 30%".to_string()),
        ];
        let menu = build_tray_menu(&sample_provider_catalog(), &labels, &both_enabled());
        // First two items should be disabled status rows.
        assert_eq!(menu[0].id.as_deref(), Some("status_claude"));
        assert!(menu[0].disabled);
        assert_eq!(menu[1].id.as_deref(), Some("status_codex"));
        assert!(menu[1].disabled);
        // Third item should be a separator.
        assert!(menu[2].is_separator);
    }
}
