//! Wire tests for the `cedar_ember` opt-in on the Claude Web usage request.

use super::ClaudeWebApiFetcher;
use crate::core::ProviderError;
use mockito::{Matcher, Mock, Server, ServerGuard};

const USAGE_PATH: &str = "/organizations/org-123/usage";
const CLOUDFLARE_MESSAGE: &str = crate::providers::claude::CLOUDFLARE_CHALLENGE_MESSAGE;

const USAGE_BODY: &str = r#"{"five_hour": {"utilization": 11}}"#;
const OPTED_IN_BODY: &str = r#"{"five_hour": {"utilization": 11},
    "cedar_ember": {"eligible": true, "grants": [
        {"resets_left": 1, "resets_total": 1, "paused": false, "ends_at": null}]}}"#;

async fn get_usage(server: &ServerGuard) -> Result<super::UsageResponse, ProviderError> {
    let fetcher = ClaudeWebApiFetcher::new().with_base_url(server.url());
    let headers = ClaudeWebApiFetcher::build_headers("sessionKey=sk-ant-fixture-token");
    fetcher.get_usage("org-123", &headers).await
}

async fn opted_in(server: &mut ServerGuard, status: usize, body: &str) -> Mock {
    server
        .mock("GET", USAGE_PATH)
        .match_query(Matcher::UrlEncoded("cedar_ember".into(), "1".into()))
        .match_header("cookie", "sessionKey=sk-ant-fixture-token")
        .with_status(status)
        .with_body(body)
        .expect(1)
        .create_async()
        .await
}

async fn plain(server: &mut ServerGuard, status: usize, expected_calls: usize) -> Mock {
    server
        .mock("GET", USAGE_PATH)
        .match_query(Matcher::Missing)
        .match_header("cookie", "sessionKey=sk-ant-fixture-token")
        .with_status(status)
        .with_body(USAGE_BODY)
        .expect(expected_calls)
        .create_async()
        .await
}

#[tokio::test]
async fn successful_opt_in_keeps_the_reset_block_and_does_not_retry() {
    let mut server = Server::new_async().await;
    let first = opted_in(&mut server, 200, OPTED_IN_BODY).await;
    let retry = plain(&mut server, 200, 0).await;

    let usage = get_usage(&server).await.unwrap();

    assert!(usage.five_hour.is_some());
    assert!(usage.cedar_ember.is_some());
    first.assert_async().await;
    retry.assert_async().await;
}

#[tokio::test]
async fn rejected_opt_in_retries_once_without_it_and_keeps_usage_windows() {
    for status in [400, 403, 404, 422, 500, 503] {
        let mut server = Server::new_async().await;
        let first = opted_in(
            &mut server,
            status,
            r#"{"error": "unknown query parameter"}"#,
        )
        .await;
        let retry = plain(&mut server, 200, 1).await;

        let usage = get_usage(&server).await.unwrap();

        assert!(usage.five_hour.is_some(), "status {status}");
        assert!(usage.cedar_ember.is_none(), "status {status}");
        first.assert_async().await;
        retry.assert_async().await;
    }
}

#[tokio::test]
async fn retry_failure_keeps_normal_error_handling() {
    let mut server = Server::new_async().await;
    let first = opted_in(&mut server, 422, "{}").await;
    let retry = plain(&mut server, 500, 1).await;

    let error = get_usage(&server).await.err().unwrap();

    assert!(matches!(error, ProviderError::Other(message) if message.contains("500")));
    first.assert_async().await;
    retry.assert_async().await;
}

#[tokio::test]
async fn unauthorized_and_rate_limited_responses_are_not_retried() {
    for status in [401, 429] {
        let mut server = Server::new_async().await;
        let first = opted_in(&mut server, status, "{}").await;
        let retry = plain(&mut server, 200, 0).await;

        let error = get_usage(&server).await.err().unwrap();

        match status {
            401 => assert!(matches!(error, ProviderError::AuthRequired)),
            _ => assert!(matches!(error, ProviderError::Other(message) if message.contains("429"))),
        }
        first.assert_async().await;
        retry.assert_async().await;
    }
}

#[tokio::test]
async fn cloudflare_challenge_is_not_retried() {
    let mut server = Server::new_async().await;
    let first = server
        .mock("GET", USAGE_PATH)
        .match_query(Matcher::UrlEncoded("cedar_ember".into(), "1".into()))
        .with_status(403)
        .with_header("cf-mitigated", "challenge")
        .with_body("challenge page")
        .expect(1)
        .create_async()
        .await;
    let retry = plain(&mut server, 200, 0).await;

    let error = get_usage(&server).await.err().unwrap();

    assert!(matches!(error, ProviderError::Other(message) if message == CLOUDFLARE_MESSAGE));
    first.assert_async().await;
    retry.assert_async().await;
}

#[tokio::test]
async fn ordinary_forbidden_retries_and_a_second_forbidden_is_an_auth_failure() {
    let mut server = Server::new_async().await;
    let first = opted_in(&mut server, 403, "permission denied").await;
    let retry = plain(&mut server, 403, 1).await;

    let error = get_usage(&server).await.err().unwrap();

    assert!(matches!(error, ProviderError::AuthRequired));
    first.assert_async().await;
    retry.assert_async().await;
}
