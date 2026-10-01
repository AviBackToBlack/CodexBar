use super::*;

fn web_claims() -> serde_json::Map<String, Value> {
    serde_json::from_value(serde_json::json!({
        "exp": 1_900_000_000,
        "userType": "paid",
        "bundledCredits": 80,
        "veniceCredits": 120,
        "bundledCreditsUsage": {
            "usedThisCycle": 12,
            "monthlyRefillCredits": 100,
            "availableCredits": 88,
            "tierCap": 200,
            "nextRefillAt": 1_900_000_000_000i64
        }
    }))
    .unwrap()
}

#[test]
fn venice_snapshot_uses_diem_allocation() {
    let snapshot = snapshot_from_balance(&VeniceBalanceResponse {
        can_consume: true,
        consumption_currency: Some("DIEM".into()),
        balances: VeniceBalances {
            diem: Some(25.0),
            usd: None,
        },
        diem_epoch_allocation: Some(100.0),
    });
    assert_eq!(snapshot.primary.used_percent, 75.0);
}

fn legacy(value: &str) -> Option<VeniceSessionCredential> {
    Some(VeniceSessionCredential::Legacy(value.to_string()))
}

fn clerk(value: &str) -> Option<VeniceSessionCredential> {
    Some(VeniceSessionCredential::Clerk(value.to_string()))
}

fn browser_cookie(name: &str, domain: &str) -> crate::browser::cookies::Cookie {
    crate::browser::cookies::Cookie {
        name: name.to_string(),
        value: "synthetic-session".to_string(),
        domain: domain.to_string(),
        path: "/".to_string(),
        expires: None,
        is_secure: true,
        is_http_only: false,
    }
}

#[test]
fn session_cookie_prefers_exact_and_reassembles_contiguous_chunks() {
    assert_eq!(
        session_credential_from_header(
            "other=x; __venice-auth.session-token.0=ab; __venice-auth.session-token.1=cd"
        ),
        legacy("abcd")
    );
    assert_eq!(
        session_credential_from_header(
            "__venice-auth.session-token.0=ab; __venice-auth.session-token.2=cd"
        ),
        None
    );
    assert_eq!(
        session_credential_from_header(
            "__venice-auth.session-token=exact; __venice-auth.session-token.0=chunk"
        ),
        legacy("exact")
    );
    assert_eq!(
        session_credential_from_header("__venice-auth.session-token.0=a\nsecret"),
        None
    );
    assert_eq!(
        session_credential_from_header(
            "__venice-auth.session-token=one; __venice-auth.session-token=two"
        ),
        None
    );
    assert_eq!(
        session_credential_from_header("__venice-auth.session-token.not-a-chunk=value"),
        None
    );
    assert_eq!(
        session_credential_from_header("Cookie: __session=pasted; other=x"),
        clerk("pasted")
    );
    assert_eq!(
        session_credential_from_header("cookie: __venice-auth.session-token=legacy"),
        legacy("legacy")
    );
    let oversized = format!(
        "__venice-auth.session-token={}",
        "x".repeat(MAX_VENICE_COOKIE_VALUE_LEN + 1)
    );
    assert_eq!(session_credential_from_header(&oversized), None);
}

#[test]
fn clerk_session_family_is_accepted_and_sent_only_as_bearer() {
    for name in ["__session", "__session_synthetic"] {
        assert!(is_clerk_session_cookie_name(name));
        let raw = format!(
            "__client_uat=123; {name}=synthetic-session; __client=private; clerk_active_synthetic=1"
        );
        let credential = session_credential_from_header(&raw).unwrap();
        assert_eq!(
            credential,
            VeniceSessionCredential::Clerk("synthetic-session".into())
        );

        let client = Client::new();
        let request = credential
            .apply(client.get(VENICE_SESSION_URL))
            .header("Accept", "application/json")
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(request.url().as_str(), VENICE_SESSION_URL);
        assert_eq!(
            request.headers().get("Authorization").unwrap(),
            "Bearer synthetic-session"
        );
        assert!(request.headers().get("Cookie").is_none());
    }
}

#[test]
fn legacy_session_is_sent_as_cookie_without_authorization() {
    let client = Client::new();
    let request = legacy("legacy")
        .unwrap()
        .apply(client.get(VENICE_SESSION_URL))
        .build()
        .unwrap();
    assert_eq!(
        request.headers().get("Cookie").unwrap(),
        "__venice-auth.session-token=legacy"
    );
    assert!(request.headers().get("Authorization").is_none());
}

#[test]
fn legacy_session_retains_priority_over_clerk_including_numbered_chunks() {
    assert_eq!(
        session_credential_from_header("__session=clerk; __venice-auth.session-token=legacy"),
        legacy("legacy")
    );
    assert_eq!(
        session_credential_from_header(
            "__session=clerk; __venice-auth.session-token.1=b; __venice-auth.session-token.0=a"
        ),
        legacy("ab")
    );
    assert_eq!(
        session_credential_from_header("__session_synthetic=secondary; __session=primary"),
        clerk("primary")
    );
    assert_eq!(
        session_credential_from_header("__session=primary; __session_synthetic=secondary"),
        clerk("primary")
    );
    assert_eq!(
        session_credential_from_header("__session_a=first; __session_b=second"),
        clerk("first")
    );
    // A repeated `__session` keeps the last value, as upstream does.
    assert_eq!(
        session_credential_from_header("__session=stale; __session=fresh"),
        clerk("fresh")
    );
    // A partial legacy chunk set cannot be reassembled, so Clerk is used.
    assert_eq!(
        session_credential_from_header("__venice-auth.session-token.1=b; __session=clerk"),
        clerk("clerk")
    );
}

#[test]
fn browser_session_cookies_are_restricted_to_the_exact_venice_site() {
    for (domain, accepted) in [
        ("venice.ai", true),
        (".venice.ai", true),
        (".Venice.AI", true),
        ("clerk.venice.ai", false),
        (".clerk.venice.ai", false),
        ("outerface.venice.ai", false),
        ("notvenice.ai", false),
    ] {
        let cookies = [browser_cookie("__session", domain)];
        assert_eq!(
            session_credential_from_browser_cookies(&cookies),
            accepted.then(|| VeniceSessionCredential::Clerk("synthetic-session".into())),
            "{domain}"
        );
    }
    let cookies = [
        browser_cookie("__client", "clerk.venice.ai"),
        browser_cookie("__session", "clerk.venice.ai"),
        browser_cookie("__venice-auth.session-token", "venice.ai"),
    ];
    assert_eq!(
        session_credential_from_browser_cookies(&cookies),
        legacy("synthetic-session")
    );
}

#[test]
fn non_session_clerk_and_authjs_cookies_cannot_authenticate() {
    for name in [
        "__client",
        "__client_uat",
        "__client_uat_synthetic",
        "clerk_active_synthetic",
        "__session_",
        "__sessionevil",
        "__Host-authjs.csrf-token",
        "__Secure-authjs.callback-url",
    ] {
        assert!(!is_clerk_session_cookie_name(name), "{name}");
        assert_eq!(
            session_credential_from_header(&format!("{name}=synthetic")),
            None,
            "{name}"
        );
    }
}

#[test]
fn recovery_messages_explain_active_tab_and_missing_cookie_names() {
    let missing = VENICE_MISSING_CREDENTIALS_MESSAGE;
    assert!(missing.contains("__session"));
    assert!(missing.contains("__venice-auth.session-token"));
    assert!(VENICE_INVALID_SESSION_MESSAGE.contains("tab"));

    let provider = VeniceProvider::new();
    assert_eq!(
        provider.error_state_kind(&ProviderError::Other(missing.into())),
        crate::core::ProviderStateKind::NeedsAuthentication
    );
    assert_eq!(
        provider.error_state_kind(&invalid_session_error()),
        crate::core::ProviderStateKind::ExpiredSession
    );
    assert_eq!(
        provider.error_state_kind(&ProviderError::Other("other".into())),
        crate::core::ProviderStateKind::Unknown
    );
}

#[test]
fn web_claims_produce_display_details_without_quota_math() {
    let result = snapshot_from_web_claims(
        &web_claims(),
        DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap(),
    )
    .unwrap();

    assert!(result.usage.primary.is_informational);
    let details: Vec<_> = result.display_details().iter().collect();
    assert_eq!(details.len(), 6);
    assert_eq!(details[0].value(), "88");
    assert_eq!(
        details[2].progress().map(|progress| progress.total()),
        Some(100.0)
    );
}

#[test]
fn epoch_value_accepts_seconds_milliseconds_and_rejects_outliers() {
    let seconds = serde_json::json!(1_900_000_000u64);
    let millis = serde_json::json!(1_900_000_000_000i64);
    assert_eq!(
        epoch_value_to_datetime(Some(&seconds)),
        DateTime::<Utc>::from_timestamp(1_900_000_000, 0)
    );
    assert_eq!(
        epoch_value_to_datetime(Some(&millis)),
        DateTime::<Utc>::from_timestamp(1_900_000_000, 0)
    );
    assert_eq!(epoch_value_to_datetime(None), None);
    assert_eq!(epoch_value_to_datetime(Some(&serde_json::json!(42))), None);
    assert_eq!(
        epoch_value_to_datetime(Some(&serde_json::json!("1900000000"))),
        DateTime::<Utc>::from_timestamp(1_900_000_000, 0)
    );
}

#[test]
fn web_claims_reject_expired_anonymous_and_missing_usage() {
    let now = DateTime::<Utc>::from_timestamp(1_900_000_000, 0).unwrap();
    let mut expired = web_claims();
    expired.insert("exp".into(), Value::from(1_800_000_000));
    let expired_error = snapshot_from_web_claims(&expired, now)
        .unwrap_err()
        .into_provider_error();
    assert!(matches!(
        expired_error,
        ProviderError::Other(message) if message == VENICE_INVALID_SESSION_MESSAGE
    ));

    let mut anonymous = web_claims();
    anonymous.insert("userType".into(), Value::from("anonymous"));
    let anonymous_error = snapshot_from_web_claims(&anonymous, now)
        .unwrap_err()
        .into_provider_error();
    assert!(matches!(anonymous_error, ProviderError::AuthRequired));

    let mut missing = web_claims();
    missing.remove("bundledCreditsUsage");
    let missing_error = snapshot_from_web_claims(&missing, now)
        .unwrap_err()
        .into_provider_error();
    assert!(matches!(missing_error, ProviderError::Parse(_)));
}

fn test_candidates() -> Vec<(
    crate::browser::detection::BrowserType,
    VeniceSessionCredential,
)> {
    use crate::browser::detection::BrowserType;

    vec![
        (
            BrowserType::Chrome,
            VeniceSessionCredential::Clerk("first".into()),
        ),
        (
            BrowserType::Edge,
            VeniceSessionCredential::Legacy("second".into()),
        ),
    ]
}

fn credential_value(credential: &VeniceSessionCredential) -> &str {
    match credential {
        VeniceSessionCredential::Legacy(value) | VeniceSessionCredential::Clerk(value) => value,
    }
}

fn test_result(source: &str) -> ProviderFetchResult {
    ProviderFetchResult::new(
        UsageSnapshot::new(RateWindow::informational("synthetic")),
        source,
    )
}

#[tokio::test]
async fn unusable_browser_sessions_fall_through_to_the_next_candidate() {
    use std::cell::RefCell;

    let failures = [
        VeniceWebFailure::InvalidSession,
        VeniceWebFailure::Anonymous,
        VeniceWebFailure::MissingQuota(ProviderError::Parse("missing quota".into())),
    ];
    for failure in failures {
        let calls = RefCell::new(Vec::new());
        let first_failure = RefCell::new(Some(failure));
        let result = fetch_web_sessions(test_candidates(), |credential| {
            let value = credential_value(&credential).to_string();
            calls.borrow_mut().push(value.clone());
            let outcome = if value == "first" {
                Err(first_failure.borrow_mut().take().unwrap())
            } else {
                Ok(test_result("second"))
            };
            async move { outcome }
        })
        .await;

        assert_eq!(*calls.borrow(), ["first", "second"]);
        assert_eq!(result.unwrap().source_label, "second");
    }
}

#[tokio::test]
async fn other_browser_failures_stop_without_trying_the_next_candidate() {
    use std::cell::RefCell;

    let cases = [
        (false, "network failure"),
        (true, "JWT parse failure"),
        (
            false,
            "Venice web session returned status 500 Internal Server Error",
        ),
    ];
    for (parse, message) in cases {
        let calls = RefCell::new(Vec::new());
        let (failure, expected) = if parse {
            (
                VeniceWebFailure::Other(ProviderError::Parse(message.into())),
                ProviderError::Parse(message.into()),
            )
        } else {
            (
                VeniceWebFailure::Other(ProviderError::Other(message.into())),
                ProviderError::Other(message.into()),
            )
        };
        let actual = RefCell::new(Some(failure));
        let result = fetch_web_sessions(test_candidates(), |credential| {
            calls
                .borrow_mut()
                .push(credential_value(&credential).to_string());
            let failure = actual.borrow_mut().take().unwrap();
            async move { Err(failure) }
        })
        .await;

        assert_eq!(calls.borrow().as_slice(), ["first"]);
        match (result.unwrap_err(), expected) {
            (ProviderError::Other(actual), ProviderError::Other(expected))
            | (ProviderError::Parse(actual), ProviderError::Parse(expected)) => {
                assert_eq!(actual, expected);
            }
            errors => panic!("unexpected errors: {errors:?}"),
        }
    }
}

#[tokio::test]
async fn all_unusable_candidates_return_the_last_error() {
    let result = fetch_web_sessions(test_candidates(), |credential| async move {
        let failure = if credential_value(&credential) == "first" {
            VeniceWebFailure::InvalidSession
        } else {
            VeniceWebFailure::MissingQuota(ProviderError::Parse("last quota error".into()))
        };
        Err(failure)
    })
    .await;

    assert!(matches!(
        result,
        Err(ProviderError::Parse(message)) if message == "last quota error"
    ));
}

#[tokio::test]
async fn empty_browser_candidates_return_missing_credentials() {
    let result = fetch_web_sessions(Vec::new(), |_| async {
        panic!("the loader must not run for an empty candidate list")
    })
    .await;

    assert!(matches!(
        result,
        Err(ProviderError::Other(message)) if message == VENICE_MISSING_CREDENTIALS_MESSAGE
    ));
}

#[tokio::test]
async fn browser_candidates_without_session_credentials_are_skipped_in_order() {
    use crate::browser::detection::BrowserType;
    use std::cell::RefCell;

    let mut edge_cookie = browser_cookie("__session", "venice.ai");
    edge_cookie.value = "edge-session".into();
    let mut firefox_cookie = browser_cookie(VENICE_SESSION_COOKIE, "venice.ai");
    firefox_cookie.value = "firefox-session".into();
    let credentials = browser_session_candidates(vec![
        (
            BrowserType::Chrome,
            vec![browser_cookie("__client", "venice.ai")],
        ),
        (BrowserType::Edge, vec![edge_cookie]),
        (BrowserType::Firefox, vec![firefox_cookie]),
    ]);
    assert_eq!(
        credentials
            .iter()
            .map(|(browser, _)| *browser)
            .collect::<Vec<_>>(),
        [BrowserType::Edge, BrowserType::Firefox]
    );
    assert_eq!(
        credentials
            .iter()
            .map(|(_, credential)| credential_value(credential))
            .collect::<Vec<_>>(),
        ["edge-session", "firefox-session"]
    );

    let calls = RefCell::new(0);
    let result = fetch_web_sessions(credentials, |_| {
        *calls.borrow_mut() += 1;
        async { Err(VeniceWebFailure::InvalidSession) }
    })
    .await;
    assert_eq!(*calls.borrow(), 2);
    assert!(
        matches!(result, Err(ProviderError::Other(message)) if message == VENICE_INVALID_SESSION_MESSAGE)
    );

    let no_credentials = browser_session_candidates(vec![(
        BrowserType::Chrome,
        vec![browser_cookie("__client", "venice.ai")],
    )]);
    let called = RefCell::new(false);
    let result = fetch_web_sessions(no_credentials, |_| {
        *called.borrow_mut() = true;
        async { Err(VeniceWebFailure::InvalidSession) }
    })
    .await;
    assert!(!*called.borrow());
    assert!(
        matches!(result, Err(ProviderError::Other(message)) if message == VENICE_MISSING_CREDENTIALS_MESSAGE)
    );
}
