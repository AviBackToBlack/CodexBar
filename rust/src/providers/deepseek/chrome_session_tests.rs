use super::*;

const TOKEN: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

#[test]
fn extracts_token_from_json_object_fields() {
    for field in ["value", "token", "access_token", "accessToken", "userToken"] {
        let raw = format!(r#"{{"{field}":"{TOKEN}","__version":"0"}}"#);
        assert_eq!(extract_user_token(&raw).as_deref(), Some(TOKEN), "{field}");
    }
}

#[test]
fn json_object_skips_implausible_fields_and_prefers_field_order() {
    let raw = format!(r#"{{"value":"short","token":"{TOKEN}"}}"#);
    assert_eq!(extract_user_token(&raw).as_deref(), Some(TOKEN));
    assert_eq!(extract_user_token(r#"{"value":"short"}"#), None);
    assert_eq!(extract_user_token(r#"{"value":42}"#), None);
}

#[test]
fn extracts_bare_and_quoted_tokens() {
    assert_eq!(extract_user_token(TOKEN).as_deref(), Some(TOKEN));
    assert_eq!(
        extract_user_token(&format!("  \"{TOKEN}\"\n")).as_deref(),
        Some(TOKEN)
    );
    assert_eq!(
        extract_user_token(&format!("'{TOKEN}'")).as_deref(),
        Some(TOKEN)
    );
}

#[test]
fn rejects_empty_short_and_spaced_values() {
    assert_eq!(extract_user_token(""), None);
    assert_eq!(extract_user_token("   "), None);
    assert_eq!(extract_user_token("short-token"), None);
    assert_eq!(extract_user_token("abcdefghij klmnopqrstuvwxyz"), None);
    assert_eq!(extract_user_token("[\"abcdefghijklmnopqrstuvwxyz\"]"), None);
}

#[test]
fn canonical_profile_id_normalizes_paths_only() {
    assert_eq!(canonical_profile_id(" chrome:Default "), "chrome:Default");
    assert_eq!(
        canonical_profile_id("C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Profile 1"),
        "chrome:Profile 1"
    );
    assert_eq!(
        canonical_profile_id("/home/me/.config/google-chrome/Default/"),
        "chrome:Default"
    );
}

#[test]
fn profile_selection_ignores_blank_values() {
    assert_eq!(ProfileSelection::from_value(None).profile_id, None);
    assert_eq!(ProfileSelection::from_value(Some("  ")).profile_id, None);
    assert_eq!(
        ProfileSelection::from_value(Some("chrome:Profile 2"))
            .profile_id
            .as_deref(),
        Some("chrome:Profile 2")
    );
}

#[test]
fn token_info_debug_redacts_the_token() {
    let info = TokenInfo {
        id: "chrome:Default".into(),
        token: TOKEN.into(),
        label: "Google Chrome Default".into(),
    };
    let rendered = format!("{info:?}");
    assert!(!rendered.contains(TOKEN));
    assert!(rendered.contains("chrome:Default"));
}
