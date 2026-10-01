//! Redaction and value behaviour of [`ProcessEnvironment`] and of the structs that store one
//! (upstream 0.70.0 `ProcessEnvironmentTests`).

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::panic::{self, AssertUnwindSafe};

use super::ProcessEnvironment;
use crate::cli::tty_runner::TtyCommandOptions;
use crate::core::{ProviderId, TokenAccount, TokenAccountOverride};

const SENTINEL: &str = "sentinel-environment-value-must-not-be-rendered";
const SENTINEL_KEY: &str = "CODEXBAR_TEST_SENTINEL_SECRET";
const ORDINARY_KEY: &str = "ORDINARY_NAME";
const TWO_ENTRIES: &str = "ProcessEnvironment(2 entries; redacted)";

/// A synthetic environment: a secret-looking variable and an ordinary one, both holding the
/// sentinel value.
fn synthetic_environment() -> HashMap<String, String> {
    HashMap::from([
        (SENTINEL_KEY.to_string(), SENTINEL.to_string()),
        (ORDINARY_KEY.to_string(), SENTINEL.to_string()),
    ])
}

fn single_entry() -> HashMap<String, String> {
    HashMap::from([(SENTINEL_KEY.to_string(), SENTINEL.to_string())])
}

/// Neither values nor variable names may be rendered.
fn assert_redacted(output: &str) {
    for hidden in [SENTINEL, SENTINEL_KEY, ORDINARY_KEY] {
        assert!(!output.contains(hidden), "rendered {hidden}: {output}");
    }
}

/// `{:?}` and `{:#?}` of a storing struct show the count and nothing else of the environment.
fn assert_debug_shows_only_two_entries(value: &impl std::fmt::Debug) {
    for output in [format!("{value:?}"), format!("{value:#?}")] {
        assert_redacted(&output);
        assert!(output.contains(TWO_ENTRIES), "{output}");
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OptionalConfiguration {
    environment: ProcessEnvironment<Option<HashMap<String, String>>>,
}

impl OptionalConfiguration {
    fn with(environment: HashMap<String, String>) -> Self {
        Self {
            environment: Some(environment).into(),
        }
    }
}

#[test]
fn structs_storing_an_environment_render_only_its_entry_count() {
    let tty = TtyCommandOptions {
        env: synthetic_environment().into(),
        ..TtyCommandOptions::default()
    };
    assert_debug_shows_only_two_entries(&tty);
    let token_override = TokenAccountOverride {
        provider: ProviderId::OpenRouter,
        account: TokenAccount::new("synthetic", "synthetic-token"),
        env_override: Some(synthetic_environment()).into(),
        cookie_header: None,
    };
    assert_debug_shows_only_two_entries(&token_override);
}

#[cfg(windows)]
#[test]
fn managed_process_config_renders_only_its_entry_count() {
    let managed = crate::managed_process::ManagedProcessConfig {
        program: std::path::PathBuf::from("synthetic-helper.exe"),
        args: Vec::new(),
        env: synthetic_environment()
            .into_iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect::<Vec<_>>()
            .into(),
        cwd: None,
        pty_rows: 24,
        pty_cols: 80,
        label: "synthetic".to_string(),
    };
    assert_debug_shows_only_two_entries(&managed);
}

#[test]
fn failed_assertions_hide_captured_environment_contents() {
    let captured = OptionalConfiguration::with(synthetic_environment());
    let other = OptionalConfiguration::default();
    let payload = panic::catch_unwind(AssertUnwindSafe(|| assert_eq!(captured, other)))
        .expect_err("different environments must not compare equal");
    let message = payload
        .downcast_ref::<String>()
        .expect("assert_eq! panics with a formatted message");
    assert!(message.contains(TWO_ENTRIES), "{message}");
    assert!(
        message.contains("ProcessEnvironment(0 entries; redacted)"),
        "{message}"
    );
    assert_redacted(message);
}

#[test]
fn wrapper_preserves_map_access_and_reports_the_current_count() {
    let mut environment = ProcessEnvironment::new(single_entry());
    assert_eq!(
        environment.get(SENTINEL_KEY).map(String::as_str),
        Some(SENTINEL)
    );
    environment.insert(ORDINARY_KEY.to_string(), SENTINEL.to_string());
    assert_eq!(environment.len(), 2);
    assert_eq!(format!("{environment:?}"), TWO_ENTRIES);
    assert_eq!(format!("{environment:#?}"), TWO_ENTRIES);

    let mut names = Vec::new();
    for (name, value) in &environment {
        assert_eq!(value, SENTINEL);
        names.push(name.as_str());
    }
    names.sort_unstable();
    assert_eq!(names, [SENTINEL_KEY, ORDINARY_KEY]);
    assert_eq!(environment.into_inner(), synthetic_environment());
}

#[test]
fn pair_lists_and_ordered_maps_count_their_entries() {
    let pairs = ProcessEnvironment::new(vec![(
        OsString::from(SENTINEL_KEY),
        OsString::from(SENTINEL),
    )]);
    assert_eq!(
        format!("{pairs:?}"),
        "ProcessEnvironment(1 entries; redacted)"
    );
    let ordered = ProcessEnvironment::new(BTreeMap::from([
        (SENTINEL_KEY, SENTINEL),
        (ORDINARY_KEY, SENTINEL),
    ]));
    assert_eq!(format!("{ordered:?}"), TWO_ENTRIES);
    let handed_to_child: Vec<(OsString, OsString)> = pairs.into_iter().collect();
    assert_eq!(
        handed_to_child,
        [(OsString::from(SENTINEL_KEY), OsString::from(SENTINEL))]
    );
}

#[test]
fn optional_environments_preserve_absence_mutation_and_value_equality() {
    let mut absent = OptionalConfiguration::default();
    let empty = OptionalConfiguration::with(HashMap::new());
    assert!(absent.environment.is_none());
    assert_ne!(absent, empty);
    assert_eq!(absent, OptionalConfiguration::default());

    *absent.environment = Some(single_entry());
    assert_eq!(absent, OptionalConfiguration::with(single_entry()));
    let mut copy = absent.clone();
    copy.environment
        .as_mut()
        .expect("environment present")
        .insert(ORDINARY_KEY.to_string(), SENTINEL.to_string());
    assert_ne!(copy, absent);
    assert_eq!(absent.environment.as_ref().map(HashMap::len), Some(1));
    assert_eq!(copy.environment.as_ref().map(HashMap::len), Some(2));

    // Equal counts render identically whatever the contents.
    let unrelated = OptionalConfiguration::with(HashMap::from([
        ("unrelated".to_string(), "value".to_string()),
        ("different".to_string(), "contents".to_string()),
    ]));
    assert_eq!(format!("{copy:?}"), format!("{unrelated:?}"));
    assert_redacted(&format!("{copy:#?}"));

    *absent.environment = None;
    assert_eq!(absent, OptionalConfiguration::default());
    assert_eq!(
        format!("{absent:?}"),
        "OptionalConfiguration { environment: ProcessEnvironment(0 entries; redacted) }"
    );
}

#[test]
fn optional_wrapper_counts_track_mutations_without_equating_missing_and_empty() {
    let mut environment = ProcessEnvironment::new(None::<HashMap<String, String>>);
    assert_eq!(
        format!("{environment:?}"),
        "ProcessEnvironment(0 entries; redacted)"
    );
    assert_ne!(environment, ProcessEnvironment::new(Some(HashMap::new())));

    *environment = Some(single_entry());
    assert_eq!(
        format!("{environment:?}"),
        "ProcessEnvironment(1 entries; redacted)"
    );
    environment
        .as_mut()
        .expect("environment present")
        .insert(ORDINARY_KEY.to_string(), SENTINEL.to_string());
    assert_eq!(format!("{environment:?}"), TWO_ENTRIES);
}
