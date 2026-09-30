use super::*;
use chrono::TimeZone;

#[test]
fn reset_diagnostic_codes_are_fixed_and_redacted() {
    let codes = [
        ResetDiagnosticReason::CandidateCreated.code(),
        ResetDiagnosticReason::SourceNotExactOAuth.code(),
        ResetDiagnosticReason::ExpiredCandidate.code(),
        ResetDiagnosticReason::ChangedCreditInventory.code(),
        ResetDiagnosticReason::StoreRequested.code(),
    ];
    assert_eq!(
        codes,
        [
            "candidateCreated",
            "sourceNotExactOAuth",
            "expiredCandidate",
            "changedCreditInventory",
            "storeRequested",
        ]
    );
    assert!(codes.iter().all(|code| {
        !code.contains('@') && !code.contains(':') && !code.contains('/') && !code.contains('\\')
    }));
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap()
}

fn snapshot(used: f64, reset_days: i64, captured_minutes: i64) -> UsageSnapshot {
    let captured = now() + chrono::Duration::minutes(captured_minutes);
    let weekly = RateWindow::with_details(
        used,
        Some(7 * 24 * 60),
        Some(now() + chrono::Duration::days(reset_days)),
        None,
    );
    let mut snapshot = UsageSnapshot::new(RateWindow::new(20.0)).with_secondary(weekly);
    snapshot.updated_at = captured;
    snapshot.login_method = Some("ChatGPT Pro".to_string());
    snapshot
}

fn inventory(id: &str) -> CreditInventory {
    CreditInventory {
        available_count: 1,
        credits: vec![CreditIdentity {
            id: id.to_string(),
            reset_type: "weekly".to_string(),
            status: "available".to_string(),
            expires_at: Some(now() + chrono::Duration::days(3)),
        }],
    }
}

fn baseline() -> AccountState {
    let previous = snapshot(45.0, 2, 0);
    AccountState {
        published_weekly: previous.secondary.clone(),
        published_at: previous.updated_at,
        plan: previous.login_method.clone(),
        credit_inventory: Some(inventory("credit-a")),
        candidate: None,
    }
}

#[test]
fn inventory_retains_consumed_status_rows_but_counts_only_available_credits() {
    let reset = ResetCredits {
        available_count: 1,
        credits: vec![
            ResetCredit {
                id: Some("available-a".into()),
                reset_type: Some("weekly".into()),
                status: Some("available".into()),
                expires_at: None,
            },
            ResetCredit {
                id: Some("redeeming-b".into()),
                reset_type: Some("weekly".into()),
                status: Some("redeeming".into()),
                expires_at: None,
            },
            ResetCredit {
                id: Some("redeemed-c".into()),
                reset_type: Some("weekly".into()),
                status: Some("redeemed".into()),
                expires_at: None,
            },
        ],
    };
    let inventory = super::inventory(Some(&reset), now()).expect("credit inventory");
    assert_eq!(inventory.available_count, 1);
    assert_eq!(inventory.credits.len(), 3);
    assert!(
        inventory
            .credits
            .iter()
            .any(|credit| credit.status == "redeeming")
    );
    assert!(
        inventory
            .credits
            .iter()
            .any(|credit| credit.status == "redeemed")
    );
}
#[test]
fn early_low_usage_requires_confirmation_without_spending_credit() {
    let mut state = baseline();
    let initial = snapshot(0.0, 9, 1);
    let inv = inventory("credit-a");
    assert_eq!(
        initial_decision(&mut state, &initial, Some(&inv), true, now()),
        InitialDecision::RequiresConfirmation
    );
    let confirmation = snapshot(0.0, 9, 2);
    assert_eq!(
        confirmation_decision(
            &mut state,
            &initial,
            Some(&inv),
            &confirmation,
            Some(&inv),
            true,
            now(),
        ),
        ConfirmationDecision::Preserve
    );
    assert!(state.candidate.is_some());
    assert_eq!(state.credit_inventory.as_ref().unwrap().available_count, 1);
}

#[test]
fn delayed_candidate_publishes_after_sixty_seconds_and_expires_after_thirty_minutes() {
    let mut state = baseline();
    let initial = snapshot(0.0, 9, 1);
    let confirmation = snapshot(0.0, 9, 2);
    let inv = inventory("credit-a");
    assert_eq!(
        confirmation_decision(
            &mut state,
            &initial,
            Some(&inv),
            &confirmation,
            Some(&inv),
            true,
            now(),
        ),
        ConfirmationDecision::Preserve
    );
    let current = snapshot(0.0, 9, 3);
    let candidate = state.candidate.clone().unwrap();
    assert_eq!(
        delayed_candidate_decision(
            &state,
            &candidate,
            &current,
            Some(&inv),
            true,
            now() + chrono::Duration::seconds(59),
        ),
        DelayedDecision::Retain
    );
    assert_eq!(
        delayed_candidate_decision(
            &state,
            &candidate,
            &current,
            Some(&inv),
            true,
            now() + chrono::Duration::seconds(60),
        ),
        DelayedDecision::Publish
    );
    assert_eq!(
        delayed_candidate_decision(
            &state,
            &candidate,
            &current,
            Some(&inv),
            true,
            now() + chrono::Duration::minutes(31),
        ),
        DelayedDecision::Discard
    );
}

#[test]
fn credits_only_refresh_retains_candidate_and_account_scope_hashes_differ() {
    let mut state = baseline();
    state.candidate = Some(DelayedCandidate {
        evidence_version: EVIDENCE_VERSION,
        first_observed_at: now(),
        created_at: now(),
        snapshot_updated_at: now(),
        weekly: snapshot(0.0, 9, 1).secondary.unwrap(),
        plan: Some("ChatGPT Pro".to_string()),
        inventory: inventory("credit-a"),
    });
    let mut credits_only = UsageSnapshot::new(RateWindow::new(20.0));
    credits_only.updated_at = now() + chrono::Duration::minutes(1);
    // A credits-only refresh has no weekly window and may omit both plan and
    // reset-credit inventory. It must not consume the pending evidence.
    let candidate_before = serde_json::to_value(&state.candidate).unwrap();
    assert_eq!(
        initial_decision(
            &mut state,
            &credits_only,
            None,
            true,
            now() + chrono::Duration::minutes(1),
        ),
        InitialDecision::Preserve
    );
    assert_eq!(
        serde_json::to_value(&state.candidate).unwrap(),
        candidate_before
    );
    assert_ne!(
        scope_key(Some("account-a"), Path::new("C:/a/auth.json")),
        scope_key(Some("account-b"), Path::new("C:/b/auth.json"))
    );
}

#[test]
fn credits_only_refresh_candidate_survives_state_reload_until_full_usage() {
    let mut state = baseline();
    state.candidate = Some(DelayedCandidate {
        evidence_version: EVIDENCE_VERSION,
        first_observed_at: now(),
        created_at: now(),
        snapshot_updated_at: now(),
        weekly: snapshot(0.0, 9, 1).secondary.unwrap(),
        plan: Some("ChatGPT Pro".to_string()),
        inventory: inventory("credit-a"),
    });
    let candidate_before = serde_json::to_value(&state.candidate).unwrap();
    let mut credits_only = UsageSnapshot::new(RateWindow::new(20.0));
    credits_only.updated_at = now() + chrono::Duration::minutes(1);

    assert_eq!(
        initial_decision(
            &mut state,
            &credits_only,
            None,
            true,
            now() + chrono::Duration::minutes(1),
        ),
        InitialDecision::Preserve
    );

    // Model the StateFile envelope used by save/load without touching the
    // user's real LocalAppData during a unit test.
    let encoded = serde_json::to_vec(&StateFile {
        version: STATE_VERSION,
        accounts: HashMap::from([(String::from("scope"), state)]),
    })
    .unwrap();
    let mut reloaded_file: StateFile = serde_json::from_slice(&encoded).unwrap();
    let mut reloaded = reloaded_file.accounts.remove("scope").unwrap();
    assert_eq!(
        serde_json::to_value(&reloaded.candidate).unwrap(),
        candidate_before
    );

    let mut incompatible = reloaded.clone();
    let mut incompatible_usage = snapshot(0.0, 9, 3);
    incompatible_usage.login_method = Some("ChatGPT Plus".to_string());
    assert_eq!(
        initial_decision(
            &mut incompatible,
            &incompatible_usage,
            Some(&inventory("credit-a")),
            true,
            now() + chrono::Duration::seconds(60),
        ),
        InitialDecision::RequiresConfirmation
    );
    assert!(incompatible.candidate.is_none());

    let full_usage = snapshot(0.0, 9, 3);
    assert_eq!(
        initial_decision(
            &mut reloaded,
            &full_usage,
            Some(&inventory("credit-a")),
            true,
            now() + chrono::Duration::seconds(60),
        ),
        InitialDecision::Publish
    );
    assert!(reloaded.candidate.is_none());
}

#[test]
fn consumed_credit_allows_immediate_confirmation() {
    let mut state = baseline();
    let initial = snapshot(0.0, 2, 1);
    let confirmation = snapshot(0.0, 2, 2);
    let consumed = CreditInventory {
        available_count: 0,
        credits: Vec::new(),
    };
    assert_eq!(
        confirmation_decision(
            &mut state,
            &initial,
            Some(&consumed),
            &confirmation,
            Some(&consumed),
            true,
            now(),
        ),
        ConfirmationDecision::Publish
    );
}

const WEEK_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Unused weekly window whose reset date sits `boundary_ahead` seconds after its own capture time.
fn rolling_snapshot(
    used: f64,
    window_minutes: u32,
    captured_seconds: i64,
    boundary_ahead: i64,
) -> UsageSnapshot {
    let captured = now() + chrono::Duration::seconds(captured_seconds);
    let weekly = RateWindow::with_details(
        used,
        Some(window_minutes),
        Some(captured + chrono::Duration::seconds(boundary_ahead)),
        None,
    );
    let mut snapshot = UsageSnapshot::new(RateWindow::new(20.0)).with_secondary(weekly);
    snapshot.updated_at = captured;
    snapshot.login_method = Some("ChatGPT Pro".to_string());
    snapshot
}

fn rolling_current(offset_seconds: i64) -> UsageSnapshot {
    rolling_snapshot(0.0, 7 * 24 * 60, offset_seconds, WEEK_SECONDS - 1)
}

/// Candidate created from two unused observations whose boundaries roll with capture time.
fn rolling_state() -> AccountState {
    let mut state = baseline();
    let inv = inventory("credit-a");
    let initial = rolling_snapshot(0.0, 7 * 24 * 60, 1, WEEK_SECONDS - 1);
    let confirmation = rolling_snapshot(0.0, 7 * 24 * 60, 2, WEEK_SECONDS - 2);
    assert_eq!(
        confirmation_decision(
            &mut state,
            &initial,
            Some(&inv),
            &confirmation,
            Some(&inv),
            true,
            now(),
        ),
        ConfirmationDecision::Preserve
    );
    assert!(state.candidate.is_some());
    state
}

fn rolling_decision(
    state: &AccountState,
    current: &UsageSnapshot,
    inv: &CreditInventory,
    age_seconds: i64,
) -> DelayedDecision {
    let candidate = state.candidate.clone().unwrap();
    delayed_candidate_decision(
        state,
        &candidate,
        current,
        Some(inv),
        true,
        now() + chrono::Duration::seconds(age_seconds),
    )
}

#[test]
fn unused_rolling_weekly_boundaries_confirm_across_refresh_intervals() {
    let inv = inventory("credit-a");
    for offset in [180, 300, 900] {
        let state = rolling_state();
        let current = rolling_current(offset);
        assert_eq!(
            rolling_decision(&state, &current, &inv, offset),
            DelayedDecision::Publish,
            "offset {offset}"
        );
    }
    let state = rolling_state();
    assert_eq!(
        rolling_decision(&state, &rolling_current(30), &inv, 30),
        DelayedDecision::Retain,
        "minimum age still applies"
    );
    // Equivalent boundaries keep working exactly as before.
    assert_eq!(
        rolling_decision(
            &state,
            &rolling_snapshot(0.0, 7 * 24 * 60, 120, WEEK_SECONDS - 118),
            &inv,
            120
        ),
        DelayedDecision::Publish
    );
}

#[test]
fn ordinary_publication_after_rolling_confirmation_is_unchanged() {
    let mut state = rolling_state();
    let ordinary = rolling_snapshot(2.0, 7 * 24 * 60, 300, WEEK_SECONDS - 1);
    assert_eq!(
        initial_decision(
            &mut state,
            &ordinary,
            Some(&inventory("credit-a")),
            true,
            now() + chrono::Duration::seconds(300),
        ),
        InitialDecision::Publish
    );
}

#[test]
fn rolling_weekly_confirmation_rejects_incompatible_observations() {
    let inv = inventory("credit-a");
    let week_minutes = 7 * 24 * 60;
    let cases: Vec<(&str, UsageSnapshot)> = vec![
        (
            "nonzero usage",
            rolling_snapshot(0.5, week_minutes, 300, WEEK_SECONDS - 1),
        ),
        (
            "wrong window minutes",
            rolling_snapshot(0.0, 300, 300, WEEK_SECONDS - 1),
        ),
        (
            "boundary just outside capture plus one week",
            rolling_snapshot(0.0, week_minutes, 300, WEEK_SECONDS + 121),
        ),
        (
            "boundary far from capture plus one week",
            rolling_snapshot(0.0, week_minutes, 300, 600_000),
        ),
    ];
    for (name, current) in cases {
        let state = rolling_state();
        assert_eq!(
            rolling_decision(&state, &current, &inv, 300),
            DelayedDecision::Discard,
            "{name}"
        );
    }
}

#[test]
fn rolling_weekly_confirmation_rejects_a_boundary_that_moves_backward() {
    let inv = inventory("credit-a");
    let mut state = rolling_state();
    // Both windows stay within two minutes of capture plus one week, but the
    // later observation resets earlier than the candidate.
    let candidate = state.candidate.as_mut().unwrap();
    candidate.weekly.resets_at =
        Some(candidate.snapshot_updated_at + chrono::Duration::seconds(WEEK_SECONDS + 100));
    let current = rolling_snapshot(0.0, 7 * 24 * 60, 62, WEEK_SECONDS - 100);
    assert_eq!(
        rolling_decision(&state, &current, &inv, 62),
        DelayedDecision::Discard
    );
    // The same pair with a non-decreasing boundary confirms.
    let current = rolling_snapshot(0.0, 7 * 24 * 60, 62, WEEK_SECONDS + 100);
    assert_eq!(
        rolling_decision(&state, &current, &inv, 62),
        DelayedDecision::Publish
    );
}

#[test]
fn rolling_weekly_confirmation_rejects_nonzero_or_mismatched_candidate_window() {
    let inv = inventory("credit-a");
    for (name, mutate) in [
        (
            "candidate used",
            (|weekly: &mut RateWindow| weekly.used_percent = 0.5) as fn(&mut RateWindow),
        ),
        ("candidate window minutes", |weekly| {
            weekly.window_minutes = Some(300);
        }),
        ("candidate boundary not near a week", |weekly| {
            weekly.resets_at = weekly.resets_at.map(|at| at + chrono::Duration::hours(1));
        }),
    ] {
        let mut state = rolling_state();
        if let Some(candidate) = state.candidate.as_mut() {
            mutate(&mut candidate.weekly);
        }
        assert_eq!(
            rolling_decision(&state, &rolling_current(300), &inv, 300),
            DelayedDecision::Discard,
            "{name}"
        );
    }
}

#[test]
fn rolling_weekly_confirmation_keeps_inventory_and_expiry_guards() {
    let state = rolling_state();
    let current = rolling_current(300);
    assert_eq!(
        rolling_decision(&state, &current, &inventory("credit-b"), 300),
        DelayedDecision::Discard,
        "inventory changed"
    );
    let inv = inventory("credit-a");
    let current = rolling_current(31 * 60);
    assert_eq!(
        rolling_decision(&state, &current, &inv, 31 * 60),
        DelayedDecision::Discard,
        "candidate expired"
    );
}

fn plan_snapshot(plan: Option<&str>, used: f64, captured_minutes: i64) -> UsageSnapshot {
    let mut snapshot = snapshot(used, 9, captured_minutes);
    snapshot.login_method = plan.map(str::to_string);
    snapshot
}

/// Plus subscription with a stale 80% weekly baseline that resets in one day.
fn plus_baseline() -> AccountState {
    let mut previous = snapshot(80.0, 1, 0);
    previous.login_method = Some("ChatGPT Plus".to_string());
    AccountState {
        published_weekly: previous.secondary.clone(),
        published_at: previous.updated_at,
        plan: previous.login_method.clone(),
        credit_inventory: Some(inventory("credit-a")),
        candidate: None,
    }
}

#[test]
fn plan_upgrade_starts_a_new_baseline_and_publishes_the_new_plan() {
    let mut state = plus_baseline();
    let inv = inventory("credit-a");
    let initial = plan_snapshot(Some("ChatGPT Pro"), 0.0, 10);
    let confirmation = plan_snapshot(Some("ChatGPT Pro"), 0.0, 11);
    assert_eq!(
        initial_decision(&mut state, &initial, Some(&inv), true, now()),
        InitialDecision::RequiresConfirmation
    );
    assert!(state.published_weekly.is_none());
    assert!(state.credit_inventory.is_none());
    assert!(state.candidate.is_none());
    assert_eq!(
        confirmation_decision(
            &mut state,
            &initial,
            Some(&inv),
            &confirmation,
            Some(&inv),
            true,
            now(),
        ),
        ConfirmationDecision::Publish
    );
}

#[test]
fn same_plan_near_zero_reading_keeps_the_previous_weekly_pinned() {
    // Identical to the upgrade scenario, but the plan did not change: the old
    // weekly window stays pinned until the confirmation is trustworthy.
    let mut state = plus_baseline();
    let inv = inventory("credit-a");
    let initial = plan_snapshot(Some("ChatGPT Plus"), 0.0, 10);
    let confirmation = plan_snapshot(Some("ChatGPT Plus"), 0.0, 11);
    assert_eq!(
        initial_decision(&mut state, &initial, Some(&inv), true, now()),
        InitialDecision::RequiresConfirmation
    );
    assert!(state.published_weekly.is_some());
    assert_eq!(
        confirmation_decision(
            &mut state,
            &initial,
            Some(&inv),
            &confirmation,
            Some(&inv),
            true,
            now(),
        ),
        ConfirmationDecision::Preserve
    );
}

#[test]
fn plan_upgrade_does_not_pin_the_previous_plan_weekly_window() {
    let mut state = plus_baseline();
    let current = plan_snapshot(Some("ChatGPT Pro"), 5.0, 10);
    assert_eq!(
        initial_decision(&mut state, &current, None, true, now()),
        InitialDecision::Publish
    );
    let preserved = preserve_weekly(&state, current.clone());
    let used = |snapshot: &UsageSnapshot| snapshot.secondary.as_ref().map(|w| w.used_percent);
    assert_eq!(used(&preserved), Some(5.0));
    assert_eq!(used(&preserved), used(&current));
}

#[test]
fn plan_change_discards_a_pending_candidate() {
    let mut state = plus_baseline();
    state.candidate = Some(DelayedCandidate {
        evidence_version: EVIDENCE_VERSION,
        first_observed_at: now(),
        created_at: now(),
        snapshot_updated_at: now(),
        weekly: RateWindow::new(0.0),
        plan: Some("ChatGPT Plus".to_string()),
        inventory: inventory("credit-a"),
    });
    let current = plan_snapshot(Some("ChatGPT Pro"), 5.0, 10);
    assert_eq!(
        initial_decision(&mut state, &current, None, true, now()),
        InitialDecision::Publish
    );
    assert!(state.candidate.is_none());
}

#[test]
fn same_unknown_stale_or_non_oauth_plans_keep_the_baseline() {
    let cases: [(&str, Option<&str>, bool); 4] = [
        (
            "same plan with case and spacing",
            Some(" chatgpt plus "),
            true,
        ),
        ("unknown fresh plan", None, true),
        ("blank fresh plan", Some("  "), true),
        ("not exact OAuth", Some("ChatGPT Pro"), false),
    ];
    for (name, plan, exact_oauth) in cases {
        let mut state = plus_baseline();
        let current = plan_snapshot(plan, 5.0, 10);
        initial_decision(&mut state, &current, None, exact_oauth, now());
        assert!(state.published_weekly.is_some(), "{name}");
        assert!(state.credit_inventory.is_some(), "{name}");
    }

    let mut unknown_stored = plus_baseline();
    unknown_stored.plan = None;
    let fresh = plan_snapshot(Some("ChatGPT Pro"), 5.0, 10);
    initial_decision(&mut unknown_stored, &fresh, None, true, now());
    assert!(
        unknown_stored.published_weekly.is_some(),
        "unknown stored plan"
    );

    let mut older = plus_baseline();
    let stale = plan_snapshot(Some("ChatGPT Pro"), 5.0, -1);
    initial_decision(&mut older, &stale, None, true, now());
    assert!(older.published_weekly.is_some(), "older observation");
}

#[test]
fn unknown_plan_publication_preserves_the_last_known_plan() {
    for unknown_plan in [None, Some("  ")] {
        let mut state = plus_baseline();
        let inventory = inventory("credit-a");
        let unknown = plan_snapshot(unknown_plan, 50.0, 10);
        assert_eq!(
            initial_decision(&mut state, &unknown, Some(&inventory), true, now()),
            InitialDecision::Publish
        );
        commit_publication(&mut state, &unknown, Some(inventory));
        assert_eq!(state.plan.as_deref(), Some("ChatGPT Plus"));

        let changed = plan_snapshot(Some("ChatGPT Pro"), 50.0, 11);
        assert_eq!(
            initial_decision(&mut state, &changed, None, true, now()),
            InitialDecision::Publish
        );
        assert!(state.published_weekly.is_none());
        assert!(state.credit_inventory.is_none());
    }
}

#[test]
fn near_zero_confirmation_must_report_the_initial_plan() {
    let inv = inventory("credit-a");
    for confirmation_plan in [Some("ChatGPT Plus"), None] {
        for has_baseline in [false, true] {
            let mut state = if has_baseline {
                baseline()
            } else {
                AccountState::default()
            };
            let initial = plan_snapshot(Some("ChatGPT Pro"), 0.0, 10);
            let confirmation = plan_snapshot(confirmation_plan, 0.0, 11);
            assert_eq!(
                confirmation_decision(
                    &mut state,
                    &initial,
                    Some(&inv),
                    &confirmation,
                    Some(&inv),
                    true,
                    now(),
                ),
                ConfirmationDecision::Preserve,
                "{confirmation_plan:?} baseline {has_baseline}"
            );
            assert!(state.candidate.is_none());
        }
    }
}

#[test]
fn nonzero_confirmation_can_publish_its_own_plan() {
    let mut state = AccountState::default();
    let initial = plan_snapshot(Some("ChatGPT Pro"), 0.0, 10);
    let confirmation = plan_snapshot(Some("ChatGPT Plus"), 5.0, 11);
    assert_eq!(
        confirmation_decision(&mut state, &initial, None, &confirmation, None, true, now()),
        ConfirmationDecision::Publish
    );
}
