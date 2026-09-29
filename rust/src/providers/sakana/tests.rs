use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::core::ProviderDisplayDetail;

// Wire shapes copied from upstream v0.67.0 `TestsPlugin/SakanaPluginTests.swift`.
const BILLING_HTML: &str = r#"
<main>
  <div data-slot="card-title"><span>Standard</span><span>$20/mo</span></div>
  <div data-slot="card-title">Usage limit</div>
  <p class="font-medium text-sm">5-hour</p>
  <p class="text-muted-foreground text-xs tabular-nums">Resets on June 23, 2026 at 2:53 PM</p>
  <button aria-label="The 5-hour window starts with your first request."></button>
  <p class="text-muted-foreground text-sm">92% used</p>
  <p class="font-medium text-sm">Weekly</p>
  <p class="text-muted-foreground text-xs tabular-nums">Resets on June 29, 2026 at 12:00 AM</p>
  <button aria-label="Weekly usage resets every Monday at 00:00 UTC."></button>
  <p class="text-muted-foreground text-sm">32% used</p>
</main>
"#;

const PAYG_HTML: &str = r#"
<main>
  <h2 class="font-semibold text-base">Credit balance</h2>
  <button aria-label="Credit updates may be delayed."></button>
  <p class="font-semibold text-3xl tabular-nums">$12.34</p>
  <button aria-label="Usage date range">Jun 02, 2026<!-- --> -<!-- --> <!-- -->Jul 01, 2026</button>
  <h2 class="font-semibold">Usage</h2>
  <span class="text-muted-foreground text-sm">Total<!-- -->: <!-- -->$5.67</span>
</main>
"#;

const PAYG_TARGET: &str = "/billing?tab=payAsYouGo";

#[derive(Clone)]
struct Route {
    status: u16,
    body: String,
    delay: Duration,
    location: Option<String>,
}

impl Route {
    fn ok(body: &str) -> Self {
        Self::status(200, body)
    }

    fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay: Duration::ZERO,
            location: None,
        }
    }

    fn redirect(status: u16, location: &str) -> Self {
        Self {
            location: Some(location.to_string()),
            ..Self::status(status, "")
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Recorded {
    target: String,
    cookie: Option<String>,
}

/// Minimal HTTP/1.1 server routing on the full request target so the
/// `/billing` and `/billing?tab=payAsYouGo` requests can be scripted apart.
struct TestServer {
    port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start(routes: Vec<(&str, Route)>) -> Self {
        let routes: Arc<HashMap<String, Route>> = Arc::new(
            routes
                .into_iter()
                .map(|(target, route)| (target.to_string(), route))
                .collect(),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let recorded = Arc::clone(&requests);
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(serve(stream, Arc::clone(&routes), Arc::clone(&recorded)));
            }
        });
        Self {
            port,
            requests,
            accept_task,
        }
    }

    fn billing_url(&self) -> Url {
        Url::parse(&format!("http://127.0.0.1:{}/billing", self.port)).unwrap()
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn serve(
    mut stream: tokio::net::TcpStream,
    routes: Arc<HashMap<String, Route>>,
    recorded: Arc<Mutex<Vec<Recorded>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let head = String::from_utf8_lossy(&buffer).to_string();
    let target = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string();
    let cookie = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("cookie")
            .then(|| value.trim().to_string())
    });
    recorded.lock().unwrap().push(Recorded {
        target: target.clone(),
        cookie,
    });
    let route = routes
        .get(&target)
        .cloned()
        .unwrap_or_else(|| Route::status(404, "not found"));
    tokio::time::sleep(route.delay).await;
    let location = route
        .location
        .map(|location| format!("Location: {location}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {} Test\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n{location}\r\n{}",
        route.status,
        route.body.len(),
        route.body
    );
    stream.write_all(response.as_bytes()).await.ok();
    stream.shutdown().await.ok();
}

fn provider_for(server: &TestServer) -> SakanaProvider {
    let client = crate::core::credentialed_http_client_builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    SakanaProvider::new().with_billing_url(client, server.billing_url())
}

fn context() -> FetchContext {
    FetchContext {
        source_mode: SourceMode::Web,
        manual_cookie_header: Some("session=fixture".into()),
        ..FetchContext::default()
    }
}

fn detail<'a>(result: &'a ProviderFetchResult, title: &str) -> Option<&'a ProviderDisplayDetail> {
    result
        .display_details()
        .iter()
        .find(|row| row.title() == title)
}

#[test]
fn parses_sakana_windows_and_utc_reset() {
    let html = r#"
        <section>5-hour quota <span>42%</span> resets July 3, 2026 at 4:30 PM</section>
        <section>Weekly usage <span>80% used</span> resets July 8, 2026 at 12:00 AM</section>
    "#;
    let snapshot = snapshot_from_html(html).unwrap();
    assert_eq!(snapshot.primary.used_percent, 42.0);
    assert_eq!(snapshot.secondary.unwrap().used_percent, 80.0);
    assert_eq!(
        snapshot.primary.resets_at.unwrap().to_rfc3339(),
        "2026-07-03T16:30:00+00:00"
    );
}

#[test]
fn normalizes_sakana_cookie_header() {
    assert_eq!(
        normalize_cookie_header("Cookie: session=a; empty=; other=b").as_deref(),
        Some("session=a; other=b")
    );
}

#[test]
fn payg_parser_reads_balance_usage_total_and_period() {
    let parsed = payg::parse(PAYG_HTML).unwrap();
    let rows = parsed.display_details();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title(), "Balance");
    assert_eq!(rows[0].value(), "$12.34");
    assert_eq!(rows[1].title(), "Usage");
    assert_eq!(rows[1].value(), "$5.67");
    assert_eq!(
        rows[1].secondary_value(),
        Some("Jun 02, 2026 - Jul 01, 2026")
    );
}

#[test]
fn payg_parser_strips_thousands_separators() {
    let html = PAYG_HTML
        .replace("$12.34", "$1,234.50")
        .replace("$5.67", "$2,000");
    let rows = payg::parse(&html).unwrap().display_details();
    assert_eq!(rows[0].value(), "$1234.50");
    assert_eq!(rows[1].value(), "$2000.00");
}

#[test]
fn payg_zero_balance_without_usage_total_keeps_only_balance() {
    let html = PAYG_HTML.replace("$12.34", "$0.00").replace(
        r#"<span class="text-muted-foreground text-sm">Total<!-- -->: <!-- -->$5.67</span>"#,
        "",
    );
    let rows = payg::parse(&html).unwrap().display_details();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title(), "Balance");
    assert_eq!(rows[0].value(), "$0.00");
}

#[test]
fn payg_period_is_truncated_to_120_characters() {
    let html = PAYG_HTML.replace(
        "Jun 02, 2026<!-- --> -<!-- --> <!-- -->Jul 01, 2026",
        &"x".repeat(200),
    );
    let rows = payg::parse(&html).unwrap().display_details();
    assert_eq!(rows[1].secondary_value(), Some("x".repeat(120).as_str()));
}

#[test]
fn payg_parser_rejects_markup_without_balance_card() {
    assert!(payg::parse("").is_none());
    assert!(payg::parse("<main>Billing</main>").is_none());
    assert!(payg::parse(&PAYG_HTML.replace("Credit balance", "Credits")).is_none());
}

#[tokio::test]
async fn balance_rows_ride_along_with_quota_windows() {
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (PAYG_TARGET, Route::ok(PAYG_HTML)),
    ])
    .await;
    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();

    assert_eq!(result.usage.primary.used_percent, 92.0);
    assert_eq!(result.usage.secondary.as_ref().unwrap().used_percent, 32.0);
    assert_eq!(detail(&result, "Balance").unwrap().value(), "$12.34");
    let usage = detail(&result, "Usage").unwrap();
    assert_eq!(usage.value(), "$5.67");
    assert_eq!(usage.secondary_value(), Some("Jun 02, 2026 - Jul 01, 2026"));

    let mut requests = server.requests();
    requests.sort_by(|a, b| a.target.cmp(&b.target));
    assert_eq!(
        requests,
        vec![
            Recorded {
                target: "/billing".into(),
                cookie: Some("session=fixture".into())
            },
            Recorded {
                target: PAYG_TARGET.into(),
                cookie: Some("session=fixture".into())
            },
        ]
    );
}

#[tokio::test]
async fn optional_request_is_skipped_when_credits_are_hidden() {
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (PAYG_TARGET, Route::ok(PAYG_HTML)),
    ])
    .await;
    let ctx = FetchContext {
        include_credits: false,
        ..context()
    };
    let result = provider_for(&server).fetch_usage(&ctx).await.unwrap();

    assert_eq!(result.usage.primary.used_percent, 92.0);
    assert!(result.display_details().is_empty());
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn zero_balance_is_a_measured_balance() {
    let payg = PAYG_HTML.replace("$12.34", "$0.00");
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (PAYG_TARGET, Route::ok(&payg)),
    ])
    .await;
    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();
    assert_eq!(detail(&result, "Balance").unwrap().value(), "$0.00");
}

#[tokio::test]
async fn quick_optional_response_can_arrive_after_the_primary() {
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (
            PAYG_TARGET,
            Route::ok(PAYG_HTML).delayed(Duration::from_millis(60)),
        ),
    ])
    .await;
    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();
    assert_eq!(detail(&result, "Balance").unwrap().value(), "$12.34");
}

#[tokio::test]
async fn slow_primary_takes_an_already_finished_optional_result() {
    let server = TestServer::start(vec![
        (
            "/billing",
            Route::ok(BILLING_HTML).delayed(Duration::from_millis(400)),
        ),
        (PAYG_TARGET, Route::ok(PAYG_HTML)),
    ])
    .await;
    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();
    assert_eq!(detail(&result, "Balance").unwrap().value(), "$12.34");
}

#[tokio::test]
async fn slow_optional_request_never_holds_the_primary_result() {
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (
            PAYG_TARGET,
            Route::ok(PAYG_HTML).delayed(Duration::from_secs(3)),
        ),
    ])
    .await;
    let started = Instant::now();
    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(result.usage.primary.used_percent, 92.0);
    assert!(result.display_details().is_empty());
}

#[tokio::test]
async fn foreground_reads_wait_for_a_slow_optional_response() {
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (
            PAYG_TARGET,
            Route::ok(PAYG_HTML).delayed(Duration::from_millis(600)),
        ),
    ])
    .await;
    let ctx = FetchContext {
        requires_optional_usage_completeness: true,
        ..context()
    };
    let result = provider_for(&server).fetch_usage(&ctx).await.unwrap();
    assert_eq!(detail(&result, "Balance").unwrap().value(), "$12.34");
}

#[tokio::test]
async fn required_failure_does_not_wait_for_the_optional_request() {
    let server = TestServer::start(vec![
        ("/billing", Route::status(401, "expired")),
        (
            PAYG_TARGET,
            Route::ok(PAYG_HTML).delayed(Duration::from_secs(3)),
        ),
    ])
    .await;
    let started = Instant::now();
    let error = provider_for(&server)
        .fetch_usage(&context())
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::AuthRequired));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn optional_failures_preserve_the_required_quotas() {
    for optional in [
        Route::status(500, "optional failure"),
        Route::status(401, "optional failure"),
        Route::ok("optional failure"),
        Route::ok(""),
    ] {
        let server = TestServer::start(vec![
            ("/billing", Route::ok(BILLING_HTML)),
            (PAYG_TARGET, optional),
        ])
        .await;
        let result = provider_for(&server).fetch_usage(&context()).await.unwrap();
        assert_eq!(result.usage.primary.used_percent, 92.0);
        assert!(result.display_details().is_empty());
    }
}

#[tokio::test]
async fn cross_origin_optional_redirect_is_not_parsed() {
    // A second listener stands in for the foreign origin; `localhost` differs
    // from the `127.0.0.1` billing host, so the redirect policy stops it.
    let foreign_server = TestServer::start(vec![("/foreign", Route::ok(PAYG_HTML))]).await;
    let foreign = format!("http://localhost:{}/foreign", foreign_server.port);
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (PAYG_TARGET, Route::redirect(302, &foreign)),
    ])
    .await;

    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();
    assert_eq!(result.usage.primary.used_percent, 92.0);
    assert!(result.display_details().is_empty());
    assert!(foreign_server.requests().is_empty());
}

#[tokio::test]
async fn same_origin_optional_redirect_is_followed() {
    let server = TestServer::start(vec![
        ("/billing", Route::ok(BILLING_HTML)),
        (PAYG_TARGET, Route::redirect(302, "/billing/payg")),
        ("/billing/payg", Route::ok(PAYG_HTML)),
    ])
    .await;
    let result = provider_for(&server).fetch_usage(&context()).await.unwrap();
    assert_eq!(detail(&result, "Balance").unwrap().value(), "$12.34");
}

#[tokio::test]
async fn unauthorized_and_redirected_primary_responses_require_login() {
    for route in [
        Route::status(401, "private body"),
        Route::status(403, "private body"),
        Route::redirect(302, "http://localhost:9/login"),
    ] {
        let server = TestServer::start(vec![("/billing", route)]).await;
        let error = provider_for(&server)
            .fetch_usage(&context())
            .await
            .unwrap_err();
        assert!(matches!(error, ProviderError::AuthRequired), "{error:?}");
    }
}

#[tokio::test]
async fn server_error_is_an_api_failure_without_the_response_body() {
    let server = TestServer::start(vec![("/billing", Route::status(500, "private body"))]).await;
    let error = provider_for(&server)
        .fetch_usage(&context())
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::Other(_)));
    assert!(!error.to_string().contains("private body"));
}
