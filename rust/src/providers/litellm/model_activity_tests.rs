//! Scenarios ported from upstream 0.67.0 `LiteLLMModelUsageTests.swift`, run
//! against a mockito server. The window is fixed at 2026-08-26..=2026-09-24.

use chrono::NaiveDate;
use mockito::{Matcher, Server};
use reqwest::{Client, Url};

use super::*;

const COUNTERS: &str =
    r#"{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30,"api_requests":1}"#;
const NESTED: &str =
    r#"{"metrics":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30,"api_requests":2}}"#;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 24).unwrap()
}

fn page(date: &str, models: &str, total_pages: u32, page: u32) -> String {
    format!(
        r#"{{"results":[{{"date":"{date}","breakdown":{{"models":{models}}}}}],"metadata":{{"total_pages":{total_pages},"page":{page}}}}}"#
    )
}

fn normal_pages() -> Vec<(u64, u16, String)> {
    let first = format!(r#"{{"fixture-alpha":{COUNTERS},"fixture-beta":{COUNTERS}}}"#);
    let second = format!(r#"{{"fixture-alpha":{NESTED}}}"#);
    vec![
        (1, 200, page("2026-09-24", &first, 2, 1)),
        (2, 200, page("2026-09-23", &second, 2, 2)),
    ]
}

async fn run(pages: Vec<(u64, u16, String)>) -> Vec<ProviderDisplayDetail> {
    let mut server = Server::new_async().await;
    let mut mocks = Vec::new();
    for (number, status, body) in pages {
        let mock = server
            .mock("GET", "/user/daily/activity")
            .match_header("authorization", "Bearer fixture-key")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("user_id".into(), "fixture+user".into()),
                Matcher::UrlEncoded("start_date".into(), "2026-08-26".into()),
                Matcher::UrlEncoded("end_date".into(), "2026-09-24".into()),
                Matcher::UrlEncoded("page_size".into(), "1000".into()),
                Matcher::UrlEncoded("page".into(), number.to_string()),
            ]))
            .with_status(status as usize)
            .with_body(body)
            .expect(1)
            .create_async()
            .await;
        mocks.push(mock);
    }
    let endpoint = Url::parse(&server.url())
        .unwrap()
        .join("user/daily/activity")
        .unwrap();
    let rows = fetch(
        &Client::new(),
        &endpoint,
        "fixture-key",
        "fixture+user",
        today(),
    )
    .await;
    for mock in mocks {
        mock.assert_async().await;
    }
    rows
}

#[tokio::test]
async fn combines_flat_and_nested_metrics_across_pages() {
    let rows = run(normal_pages()).await;
    let titles: Vec<_> = rows.iter().map(|row| row.title()).collect();
    assert_eq!(titles, ["fixture-alpha", "fixture-beta"]);
    assert_eq!(rows[0].value(), "60 tokens \u{b7} 3 requests");
    assert_eq!(rows[0].secondary_value(), Some("Input 40 \u{b7} Output 20"));
    assert_eq!(rows[1].value(), "30 tokens \u{b7} 1 requests");
    assert!(
        rows.iter()
            .all(|row| row.section() == Some("Model activity \u{b7} 30d UTC"))
    );
}

#[tokio::test]
async fn one_day_can_be_split_across_pages() {
    let first = format!(r#"{{"fixture-alpha":{COUNTERS}}}"#);
    let second = format!(r#"{{"fixture-alpha":{NESTED}}}"#);
    let rows = run(vec![
        (1, 200, page("2026-09-24", &first, 2, 1)),
        (2, 200, page("2026-09-24", &second, 2, 2)),
    ])
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value(), "60 tokens \u{b7} 3 requests");
}

#[tokio::test]
async fn forbidden_history_is_omitted() {
    assert!(run(vec![(1, 403, "{}".into())]).await.is_empty());
}

#[tokio::test]
async fn non_200_success_status_is_omitted() {
    let body = page("2026-09-24", &format!(r#"{{"a":{COUNTERS}}}"#), 1, 1);
    assert!(run(vec![(1, 201, body)]).await.is_empty());
}

#[tokio::test]
async fn unreachable_endpoint_is_omitted() {
    let endpoint = Url::parse("http://127.0.0.1:1/user/daily/activity").unwrap();
    let rows = fetch(
        &Client::new(),
        &endpoint,
        "fixture-key",
        "fixture+user",
        today(),
    )
    .await;
    assert!(rows.is_empty());
}

#[tokio::test]
async fn malformed_pages_are_omitted() {
    for body in [
        r#"{"results":[{"date":"2026-09-24"}]}"#,
        r#"{"results":{}}"#,
        r#"[]"#,
        r#"not json"#,
        r#"{"results":[["2026-09-24",{}]]}"#,
        r#"{"results":[{"date":"2026-09-24","breakdown":{"models":[]}}]}"#,
        r#"{"results":[{"date":"2026-09-24","breakdown":{"models":{"m":[20,10,30,1]}}}]}"#,
        r#"{"results":[{"date":"2026-09-24","breakdown":{"models":{"m":{"prompt_tokens":1}}}}]}"#,
        r#"{"results":[],"metadata":[]}"#,
        r#"{"results":[],"metadata":{"has_more":"yes"}}"#,
    ] {
        assert!(run(vec![(1, 200, body.into())]).await.is_empty(), "{body}");
    }
}

#[tokio::test]
async fn too_many_days_on_one_page_is_omitted() {
    let day = format!(r#"{{"date":"2026-09-24","breakdown":{{"models":{{"m":{COUNTERS}}}}}}}"#);
    let days = vec![day; 32].join(",");
    let body = format!(r#"{{"results":[{days}]}}"#);
    assert!(run(vec![(1, 200, body)]).await.is_empty());
}

#[tokio::test]
async fn unsafe_and_invalid_counters_are_omitted() {
    for counters in [
        r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":1e100,"api_requests":0}"#,
        r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":9007199254740992,"api_requests":0}"#,
        r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":9007199254740992.0,"api_requests":0}"#,
        r#"{"prompt_tokens":-1,"completion_tokens":0,"total_tokens":0,"api_requests":0}"#,
        r#"{"prompt_tokens":1.5,"completion_tokens":0,"total_tokens":0,"api_requests":0}"#,
        r#"{"prompt_tokens":"1","completion_tokens":0,"total_tokens":0,"api_requests":0}"#,
    ] {
        let body = page(
            "2026-09-24",
            &format!(r#"{{"fixture-alpha":{counters}}}"#),
            1,
            1,
        );
        assert!(run(vec![(1, 200, body)]).await.is_empty(), "{counters}");
    }
}

#[tokio::test]
async fn running_sum_past_the_safe_integer_range_is_omitted() {
    let near_max = r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":9007199254740991,"api_requests":0}"#;
    let one = r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":1,"api_requests":0}"#;
    let first = page("2026-09-24", &format!(r#"{{"m":{near_max}}}"#), 2, 1);
    let second = page("2026-09-23", &format!(r#"{{"m":{one}}}"#), 2, 2);
    assert!(
        run(vec![(1, 200, first), (2, 200, second)])
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn integral_float_counters_are_accepted() {
    let counters =
        r#"{"prompt_tokens":20.0,"completion_tokens":10.0,"total_tokens":30.0,"api_requests":1.0}"#;
    let body = page(
        "2026-09-24",
        &format!(r#"{{"fixture-alpha":{counters}}}"#),
        1,
        1,
    );
    let rows = run(vec![(1, 200, body)]).await;
    assert_eq!(rows[0].value(), "30 tokens \u{b7} 1 requests");
}

#[tokio::test]
async fn history_still_incomplete_after_three_pages_is_omitted() {
    let models = format!(r#"{{"fixture-alpha":{COUNTERS}}}"#);
    let pages = (1..=3)
        .map(|number| {
            let date = format!("2026-09-{}", 25 - number);
            (number as u64, 200, page(&date, &models, 4, number))
        })
        .collect();
    assert!(run(pages).await.is_empty());
}

#[tokio::test]
async fn has_more_keeps_paging_until_done() {
    let models = format!(r#"{{"fixture-alpha":{COUNTERS}}}"#);
    let first = format!(
        r#"{{"results":[{{"date":"2026-09-24","breakdown":{{"models":{models}}}}}],"metadata":{{"has_more":true,"page":1,"total_pages":1}}}}"#
    );
    let second = format!(
        r#"{{"results":[{{"date":"2026-09-23","breakdown":{{"models":{models}}}}}],"metadata":{{"has_more":false,"page":2,"total_pages":2}}}}"#
    );
    let rows = run(vec![(1, 200, first), (2, 200, second)]).await;
    assert_eq!(rows[0].value(), "60 tokens \u{b7} 2 requests");
}

#[tokio::test]
async fn repeated_page_metadata_is_omitted() {
    let models = format!(r#"{{"fixture-alpha":{COUNTERS}}}"#);
    let repeat = page("2026-09-24", &models, 2, 1);
    let rows = run(vec![(1, 200, repeat.clone()), (2, 200, repeat)]).await;
    assert!(rows.is_empty());
}

#[tokio::test]
async fn days_outside_the_window_are_omitted() {
    for date in ["2026-08-25", "2026-09-25", "2026-9-24", "20260924"] {
        let body = page(date, "{}", 1, 1);
        assert!(run(vec![(1, 200, body)]).await.is_empty(), "{date}");
    }
}

#[tokio::test]
async fn impossible_calendar_days_are_omitted() {
    let body = page("2026-09-00", &format!(r#"{{"m":{COUNTERS}}}"#), 1, 1);
    assert!(run(vec![(1, 200, body)]).await.is_empty());
}

#[tokio::test]
async fn window_boundary_days_are_accepted() {
    for date in ["2026-08-26", "2026-09-24"] {
        let body = page(date, &format!(r#"{{"m":{COUNTERS}}}"#), 1, 1);
        assert_eq!(run(vec![(1, 200, body)]).await.len(), 1, "{date}");
    }
}

#[tokio::test]
async fn invalid_model_names_are_omitted() {
    for name in ["", "bad\\nname"] {
        let body = page("2026-09-24", &format!(r#"{{"{name}":{COUNTERS}}}"#), 1, 1);
        assert!(run(vec![(1, 200, body)]).await.is_empty(), "{name:?}");
    }
}

#[tokio::test]
async fn empty_history_has_no_section() {
    let body = r#"{"results":[],"metadata":{"has_more":false}}"#;
    assert!(run(vec![(1, 200, body.into())]).await.is_empty());
}

#[tokio::test]
async fn displayed_models_are_limited_to_twenty_by_total_tokens() {
    let models = (0..25)
        .map(|index| {
            format!(
                r#""fixture-{index}":{{"prompt_tokens":{index},"completion_tokens":0,"total_tokens":{index},"api_requests":1}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let body = page("2026-09-24", &format!("{{{models}}}"), 1, 1);
    let rows = run(vec![(1, 200, body)]).await;
    assert_eq!(rows.len(), 20);
    assert_eq!(rows.first().map(|row| row.title()), Some("fixture-24"));
    assert_eq!(rows.last().map(|row| row.title()), Some("fixture-5"));
}

#[tokio::test]
async fn equal_totals_are_ordered_by_name() {
    let models = format!(r#"{{"zeta":{COUNTERS},"alpha":{COUNTERS}}}"#);
    let rows = run(vec![(1, 200, page("2026-09-24", &models, 1, 1))]).await;
    let titles: Vec<_> = rows.iter().map(|row| row.title()).collect();
    assert_eq!(titles, ["alpha", "zeta"]);
}

#[tokio::test]
async fn more_than_a_thousand_models_is_omitted() {
    let models = (0..1001)
        .map(|index| format!(r#""m{index}":{COUNTERS}"#))
        .collect::<Vec<_>>()
        .join(",");
    let body = page("2026-09-24", &format!("{{{models}}}"), 1, 1);
    assert!(run(vec![(1, 200, body)]).await.is_empty());
}
