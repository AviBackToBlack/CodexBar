use super::*;

fn owner(scope: &str, secret: &str) -> LastGoodOwner {
    LastGoodOwner::derive("test", scope, secret).expect("owner")
}

#[test]
fn owner_requires_scope_and_secret() {
    assert!(LastGoodOwner::derive("test", "  ", "secret").is_none());
    assert!(LastGoodOwner::derive("test", "chrome:Default", " \n").is_none());
}

#[test]
fn owner_trims_and_separates_scope_from_secret() {
    assert_eq!(
        owner(" chrome:Default ", " token "),
        owner("chrome:Default", "token")
    );
    assert_ne!(
        owner("chrome:Default", "token"),
        owner("chrome:Profile 1", "token")
    );
    assert_ne!(
        owner("chrome:Default", "token"),
        owner("chrome:Default", "token2")
    );
    assert_ne!(owner("ab", "c"), owner("a", "bc"));
}

#[test]
fn owner_debug_output_never_contains_digest_or_secret() {
    let rendered = format!("{:?}", owner("chrome:Default", "super-secret-token"));
    assert_eq!(rendered, "LastGoodOwner(<redacted>)");
}

#[test]
fn only_transport_failures_are_wrapped() {
    let wrapped = ProviderError::Timeout.with_failure_owner(Some(owner("p", "t")));
    assert!(matches!(wrapped, ProviderError::OwnedTransport { .. }));
    assert!(wrapped.is_transport_failure());

    let parse = ProviderError::Parse("bad".into()).with_failure_owner(Some(owner("p", "t")));
    assert!(matches!(parse, ProviderError::Parse(_)));
}

#[test]
fn ownership_of_wrapped_and_plain_errors() {
    let a = owner("p", "t");
    assert_eq!(
        ProviderError::Timeout
            .with_failure_owner(Some(a.clone()))
            .failure_ownership(),
        FailureOwnership::Owned(a)
    );
    assert_eq!(
        ProviderError::Timeout
            .with_failure_owner(None)
            .failure_ownership(),
        FailureOwnership::Unowned
    );
    assert_eq!(
        ProviderError::Timeout.failure_ownership(),
        FailureOwnership::Unchecked
    );
}

#[test]
fn retention_requires_matching_owner_unless_unchecked() {
    let a = owner("p", "t");
    let b = owner("p", "other");
    assert!(FailureOwnership::Unchecked.allows_retention(None));
    assert!(FailureOwnership::Owned(a.clone()).allows_retention(Some(&a)));
    assert!(!FailureOwnership::Owned(a.clone()).allows_retention(Some(&b)));
    assert!(!FailureOwnership::Owned(a).allows_retention(None));
    assert!(!FailureOwnership::Unowned.allows_retention(Some(&b)));
}

#[test]
fn wrapper_displays_the_underlying_message() {
    let wrapped = ProviderError::Timeout.with_failure_owner(None);
    assert_eq!(wrapped.to_string(), "Timeout");
    assert!(matches!(
        wrapped.without_failure_owner(),
        ProviderError::Timeout
    ));
}
