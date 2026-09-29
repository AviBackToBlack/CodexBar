use super::*;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn no_messages() -> MessagesUsageResponse {
    serde_json::from_str(r#"{"data":[]}"#).unwrap()
}

fn costs(json: &str) -> CostReportResponse {
    serde_json::from_str(json).unwrap()
}

fn workspace_rows(result: &ProviderFetchResult) -> Vec<(&str, &str)> {
    result
        .display_details()
        .iter()
        .map(|row| (row.title(), row.value()))
        .collect()
}

/// Wire shape from upstream 0.67.0 `ClaudeAdminAPIUsageTests`.
const WORKSPACE_FIXTURE: &str = r#"{"data":[
  {"starting_at":"2026-09-23T00:00:00Z","ending_at":"2026-09-24T00:00:00Z","results":[
    {"currency":"USD","amount":"1250","description":"Input tokens","workspace_id":"wrk_fixture"},
    {"currency":"USD","amount":"250","description":"Output tokens","workspace_id":"wrk_fixture"},
    {"currency":"USD","amount":"50","description":"Input tokens","workspace_id":null}]},
  {"starting_at":"2026-09-24T00:00:00Z","ending_at":"2026-09-25T00:00:00Z","results":[
    {"currency":"USD","amount":"75","description":"Input tokens","workspace_id":null}]}]}"#;

#[test]
fn cleans_quoted_admin_key() {
    assert_eq!(
        clean_key(" 'sk-ant-admin-123' "),
        Some("sk-ant-admin-123".to_string())
    );
}

#[test]
fn workspace_details_preserve_organization_totals_and_default_workspace_cents() {
    let report = costs(WORKSPACE_FIXTURE);
    for enabled in [false, true] {
        let result = result_from_admin_usage(&report, &no_messages(), now(), enabled);
        let cost = result.cost.as_ref().expect("organization cost");
        assert_eq!(cost.used, 16.25);
        assert!(
            result
                .usage
                .extra_rate_windows
                .iter()
                .any(|window| window.id == "cost-0"
                    && window.window.reset_description.as_deref() == Some("$13.75"))
        );
        if enabled {
            assert_eq!(
                workspace_rows(&result),
                [("wrk_fixture", "$15.00"), ("Default", "$1.25")]
            );
            assert!(
                result
                    .display_details()
                    .iter()
                    .all(|row| row.section() == Some("Workspace spend \u{b7} 30d"))
            );
        } else {
            assert!(result.display_details().is_empty());
        }
    }
}

#[test]
fn blank_workspace_ids_group_with_the_default_workspace() {
    let report = costs(
        r#"{"data":[{"starting_at":"2026-09-24T00:00:00Z","ending_at":"2026-09-25T00:00:00Z","results":[
          {"amount":"100","workspace_id":"  "},
          {"amount":"200"},
          {"amount":"400","workspace_id":" wrk_a "}]}]}"#,
    );
    let result = result_from_admin_usage(&report, &no_messages(), now(), true);
    assert_eq!(
        workspace_rows(&result),
        [("wrk_a", "$4.00"), ("Default", "$3.00")]
    );
}

#[test]
fn equal_workspace_spend_is_ordered_by_name() {
    let report = costs(
        r#"{"data":[{"starting_at":"2026-09-24T00:00:00Z","ending_at":"2026-09-25T00:00:00Z","results":[
          {"amount":"100","workspace_id":"wrk_b"},
          {"amount":"100","workspace_id":"wrk_a"}]}]}"#,
    );
    let result = result_from_admin_usage(&report, &no_messages(), now(), true);
    assert_eq!(
        workspace_rows(&result),
        [("wrk_a", "$1.00"), ("wrk_b", "$1.00")]
    );
}

#[test]
fn workspace_rows_are_bounded_and_a_single_workspace_keeps_the_organization_view() {
    for count in [1_u32, 25] {
        let rows = (0..count)
            .map(|index| {
                format!(
                    r#"{{"amount":"{}","workspace_id":"wrk_fixture_{index}"}}"#,
                    index * 100
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let report = costs(&format!(
            r#"{{"data":[{{"starting_at":"2026-09-24T00:00:00Z","ending_at":"2026-09-25T00:00:00Z","results":[{rows}]}}]}}"#
        ));
        let result = result_from_admin_usage(&report, &no_messages(), now(), true);
        assert_eq!(
            result.cost.as_ref().map(|cost| cost.used),
            Some(f64::from(count * (count - 1) / 2))
        );
        let details = result.display_details();
        if count == 1 {
            assert!(details.is_empty());
        } else {
            assert_eq!(details.len(), 20);
            assert_eq!(
                details.first().map(|row| row.title()),
                Some("wrk_fixture_24")
            );
            assert_eq!(details.last().map(|row| row.title()), Some("wrk_fixture_5"));
        }
    }
}

#[test]
fn workspace_option_groups_the_existing_cost_request_only() {
    let start = now();
    let end = now() + Duration::days(1);
    let groups = |enabled| {
        cost_report_query(&start, &end, enabled)
            .into_iter()
            .filter(|(name, _)| *name == "group_by[]")
            .map(|(_, value)| value)
            .collect::<Vec<_>>()
    };
    assert_eq!(groups(false), ["description"]);
    assert_eq!(groups(true), ["description", "workspace_id"]);
    assert_eq!(
        cost_report_query(&start, &end, true).len(),
        cost_report_query(&start, &end, false).len() + 1
    );
}
