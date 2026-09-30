use super::*;

fn write_project(projects: PathBuf, rows: &[String]) -> PathBuf {
    let project = projects.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("session.jsonl"), rows.join("\n") + "\n").unwrap();
    projects
}

#[test]
fn claude_swap_homes_are_scanned_once_beside_a_literal_config_dir() {
    let home = tempfile::tempdir().unwrap();
    let configured = tempfile::tempdir().unwrap();
    let timestamp = Utc::now().to_rfc3339();
    let row = |id: &str| claude_transcript_line(&timestamp, "requestId", id, id);
    let sessions = home.path().join(".claude-swap-backup").join("sessions");
    let configured_projects = write_project(configured.path().join("projects"), &[row("literal")]);
    let first = write_project(sessions.join("1-first").join("projects"), &[row("first")]);
    // The second home carries a copy of the first home's row.
    let second = write_project(
        sessions.join("2-second").join("projects"),
        &[row("first"), row("second")],
    );
    write_project(sessions.join("0-ignored").join("projects"), &[row("decoy")]);

    let roots = claude_roots::claude_projects_roots(configured.path().to_str(), Some(home.path()));
    assert_eq!(roots.paths, vec![configured_projects, first, second]);
    assert_eq!(roots.read_failures, 0);

    // A root that disappears before traversal is recorded as a read failure.
    let mut scan_roots = roots.paths;
    scan_roots.push(home.path().join("missing").join("projects"));
    let scanner = CostScanner::new(1);
    let cutoff = Utc::now() - Duration::days(1);
    let mut seen = HashSet::new();
    let mut pricing = ClaudeScanPricingResolver::default();
    let mut counted = 0usize;
    let read_failures = scanner.walk_claude_roots(&scan_roots, &cutoff, None, &mut |path| {
        counted +=
            scan_claude_file_with_pricing(path, &cutoff, &mut seen, None, &mut pricing, |_| {})
                .counted;
    });

    assert_eq!(read_failures, 1);
    assert_eq!(counted, 3, "literal + first + second; the copy counts once");
}
