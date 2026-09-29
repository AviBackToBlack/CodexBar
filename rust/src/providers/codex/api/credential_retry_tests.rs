//! Upstream 0.69.0 #4088: bounded re-read of `auth.json` while its owner publishes it.

use super::*;
use std::path::Path;
use std::time::Duration;

const VALID_AUTH: &str = r#"{"tokens":{"access_token":"published-token","account_id":"acct_pub"}}"#;

fn api_for(home: &Path) -> CodexApi {
    CodexApi::new().with_codex_home(home)
}

/// Publish `contents` as `auth.json` after `delay`, like the CLI's atomic replace.
fn publish_after(
    home: &Path,
    delay: Duration,
    contents: &'static str,
) -> std::thread::JoinHandle<()> {
    let auth = home.join("auth.json");
    let staging = home.join("auth.json.tmp");
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        std::fs::write(&staging, contents).expect("stage auth.json");
        std::fs::rename(&staging, &auth).expect("publish auth.json");
    })
}

#[tokio::test]
async fn missing_auth_file_published_within_retry_window_is_loaded() {
    let dir = tempfile::tempdir().expect("codex home");
    let writer = publish_after(dir.path(), Duration::from_millis(60), VALID_AUTH);

    let credentials = api_for(dir.path())
        .load_credentials()
        .await
        .expect("credentials published during retry");
    writer.join().expect("writer");

    assert_eq!(credentials.access_token, "published-token");
    assert_eq!(credentials.account_id.as_deref(), Some("acct_pub"));
}

#[tokio::test]
async fn torn_auth_file_replaced_within_retry_window_is_loaded() {
    let dir = tempfile::tempdir().expect("codex home");
    std::fs::write(dir.path().join("auth.json"), r#"{"tokens":{"access_"#).expect("torn file");
    let writer = publish_after(dir.path(), Duration::from_millis(60), VALID_AUTH);

    let credentials = api_for(dir.path())
        .load_credentials()
        .await
        .expect("credentials replaced during retry");
    writer.join().expect("writer");

    assert_eq!(credentials.access_token, "published-token");
}

#[tokio::test]
async fn missing_auth_file_stays_not_installed_after_retries() {
    let dir = tempfile::tempdir().expect("codex home");

    let error = api_for(dir.path())
        .load_credentials()
        .await
        .err()
        .expect("no auth.json");

    assert!(matches!(error, ProviderError::NotInstalled(_)), "{error:?}");
}

#[tokio::test]
async fn unreadable_auth_file_stays_other_after_retries() {
    let dir = tempfile::tempdir().expect("codex home");
    // A directory named auth.json exists but cannot be read as a file.
    std::fs::create_dir(dir.path().join("auth.json")).expect("auth.json dir");

    let error = api_for(dir.path())
        .load_credentials()
        .await
        .err()
        .expect("unreadable auth.json");

    assert!(matches!(error, ProviderError::Other(_)), "{error:?}");
}

#[tokio::test]
async fn malformed_auth_file_stays_parse_after_retries() {
    let dir = tempfile::tempdir().expect("codex home");
    std::fs::write(dir.path().join("auth.json"), "{not json").expect("malformed file");

    let error = api_for(dir.path())
        .load_credentials()
        .await
        .err()
        .expect("malformed auth.json");

    assert!(matches!(error, ProviderError::Parse(_)), "{error:?}");
}

#[tokio::test]
async fn stale_external_credentials_are_not_retried() {
    let dir = tempfile::tempdir().expect("codex home");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(format!(r#"{{"exp":{}}}"#, Utc::now().timestamp() - 3600));
    let stale = format!(
        r#"{{"last_refresh":"2020-01-01T00:00:00Z","tokens":{{"access_token":"h.{payload}.s","refresh_token":"r"}}}}"#
    );
    std::fs::write(dir.path().join("auth.json"), stale).expect("stale auth.json");
    // A fresh publication lands inside the retry window; stale must not wait for it.
    let writer = publish_after(dir.path(), Duration::from_millis(60), VALID_AUTH);

    let error = api_for(dir.path())
        .load_credentials()
        .await
        .err()
        .expect("stale credentials");
    writer.join().expect("writer");

    assert!(matches!(error, ProviderError::AuthRequired), "{error:?}");
}

#[tokio::test]
async fn dropping_the_load_cancels_the_retry_delay() {
    let dir = tempfile::tempdir().expect("codex home");
    let api = api_for(dir.path());

    let outcome = tokio::time::timeout(Duration::from_millis(20), api.load_credentials()).await;

    assert!(outcome.is_err(), "the retry delay must be cancellable");
}
