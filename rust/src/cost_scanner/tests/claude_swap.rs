//! Upstream 0.65.0 `CostUsageScannerClaudeSwapTests`, translated.

use super::*;

fn write_project(projects: PathBuf, rows: &[String]) -> PathBuf {
    let project = projects.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("session.jsonl"), rows.join("\n") + "\n").unwrap();
    projects
}

/// Upstream `row(env:day:id:input:)`: one priced Sonnet 4.5 request.
fn swap_row(timestamp: &str, id: &str, input: u64) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{timestamp}","requestId":"request-{id}","message":{{"id":"{id}","model":"claude-sonnet-4-5-20250929","usage":{{"input_tokens":{input},"output_tokens":10}}}}}}"#
    )
}

/// Aggregate `roots` the way the summary scan does: one `seen` set across
/// every root, so a row copied between homes is counted once.
fn scan_roots(roots: &[PathBuf]) -> (CostSummary, u32) {
    let scanner = CostScanner::new(1);
    let cutoff = Utc::now() - Duration::days(1);
    let mut seen = HashSet::new();
    let mut pricing = ClaudeScanPricingResolver::default();
    let mut summary = CostSummary::default();
    let read_failures = scanner.walk_claude_roots(roots, &cutoff, None, &mut |path| {
        let result = scan_claude_file_with_pricing(
            path,
            &cutoff,
            &mut seen,
            None,
            &mut pricing,
            &mut ClaudeIncompleteTracker::default(),
            |row| {
                assert!(add_claude_record_to_summary(&mut summary, row));
            },
        );
        assert!(result.is_complete(), "{result:?}");
    });
    (summary, read_failures)
}

#[test]
fn two_swap_homes_contribute_once_across_copied_and_shared_history() {
    let home = tempfile::tempdir().unwrap();
    let timestamp = Utc::now().to_rfc3339();
    let sessions = home.path().join(".claude-swap-backup").join("sessions");
    let first = write_project(
        sessions.join("1-first").join("projects"),
        &[swap_row(&timestamp, "first", 100)],
    );
    // The second home carries a copy of the first home's row.
    let second = write_project(
        sessions.join("2-second").join("projects"),
        &[
            swap_row(&timestamp, "first", 100),
            swap_row(&timestamp, "second", 200),
        ],
    );
    for decoy in ["unrelated", "0-zero"] {
        write_project(
            sessions.join(decoy).join("projects"),
            &[swap_row(&timestamp, decoy, 999)],
        );
    }
    // Shared history resolves to the first home; a dangling link is skipped.
    // Creating either link needs the symlink privilege on Windows, and the
    // expected roots are the same with or without them.
    #[cfg(windows)]
    {
        for (slot, target) in [
            ("3-shared", first.clone()),
            ("4-missing", home.path().join("gone")),
        ] {
            std::fs::create_dir_all(sessions.join(slot)).unwrap();
            drop(std::os::windows::fs::symlink_dir(
                target,
                sessions.join(slot).join("projects"),
            ));
        }
    }

    let roots = claude_roots::claude_projects_roots(None, Some(home.path()));
    assert_eq!(roots.paths, vec![first, second]);
    assert_eq!(roots.read_failures, 0);
    // Swap homes alone count as a token-cost source.
    assert!(roots.has_possible_roots());

    // Upstream rescans two minutes later; totals must not drift on a rerun.
    for _ in 0..2 {
        let (summary, read_failures) = scan_roots(&roots.paths);
        assert_eq!(read_failures, 0);
        assert_eq!(summary.input_tokens + summary.output_tokens, 320);
        assert_eq!(summary.cached_tokens, 0);
        assert!(
            (summary.total_cost_usd - 0.0012).abs() < 1e-9,
            "{}",
            summary.total_cost_usd
        );
        assert!(summary.unknown_models.is_empty());
    }
}

#[test]
fn literal_configured_home_remains_included_beside_discovered_swap_homes() {
    let home = tempfile::tempdir().unwrap();
    let timestamp = Utc::now().to_rfc3339();
    // A literal value with a comma and a space is one directory, not a list.
    let configured = home.path().join("custom, home");
    let configured_projects = write_project(
        configured.join("projects"),
        &[swap_row(&timestamp, "literal", 100)],
    );
    let swap = write_project(
        home.path()
            .join(".claude-swap-backup")
            .join("sessions")
            .join("2-second")
            .join("projects"),
        &[swap_row(&timestamp, "second", 200)],
    );

    let roots = claude_roots::claude_projects_roots(configured.to_str(), Some(home.path()));
    assert_eq!(roots.paths, vec![configured_projects, swap]);
    assert_eq!(roots.read_failures, 0);

    let (summary, read_failures) = scan_roots(&roots.paths);
    assert_eq!(read_failures, 0);
    assert_eq!(summary.input_tokens + summary.output_tokens, 320);

    // A root that disappears before traversal is recorded as a read failure.
    let mut scan_paths = roots.paths;
    scan_paths.push(home.path().join("missing").join("projects"));
    assert_eq!(scan_roots(&scan_paths).1, 1);
}
