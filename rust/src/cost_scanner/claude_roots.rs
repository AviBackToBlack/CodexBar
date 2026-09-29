//! Claude transcript roots for the local cost scan.
//!
//! The Claude profile root (`CLAUDE_CONFIG_DIR` or `~/.claude`) is always
//! scanned. claude-swap (`cswap run`) additionally keeps one Claude profile per
//! session home below `~/.claude-swap-backup/sessions/<N>-<label>/projects`,
//! which Claude Code writes to when a swapped session is active. Those homes
//! are added so the aggregate totals include them (upstream 0.65.0,
//! `ClaudeConfigPaths.costProjectsRoots`). Only transcript directories are
//! read; claude-swap credentials and private metadata are never touched.
//!
//! claude-swap uses the legacy `~/.claude-swap-backup` root on Windows and
//! macOS. The XDG data root applies to Linux/WSL only and is not needed here.

use std::fs;
use std::path::{Path, PathBuf};

const SWAP_SESSIONS_DIR: [&str; 2] = [".claude-swap-backup", "sessions"];

/// Existing Claude transcript roots, de-duplicated by resolved path.
///
/// `config_dir` is the raw `CLAUDE_CONFIG_DIR` value: one literal directory,
/// with empty meaning "unset". A root that does not exist is omitted, so an
/// empty result means there is no Claude history to scan.
pub(super) fn claude_projects_roots(config_dir: Option<&str>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![base_projects_dir(config_dir, home)];
    if let Some(home) = home {
        candidates.extend(swap_projects_roots(home));
    }

    let mut seen = Vec::new();
    let mut roots = Vec::new();
    for root in candidates {
        if !root.exists() {
            continue;
        }
        // Shared-history symlinks or junctions resolve to the same directory
        // as the profile they point at; scan that directory once.
        let resolved = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if !seen.contains(&resolved) {
            seen.push(resolved);
            roots.push(root);
        }
    }
    roots
}

fn base_projects_dir(config_dir: Option<&str>, home: Option<&Path>) -> PathBuf {
    if let Some(trimmed) = config_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(trimmed).join("projects");
    }

    let home = home.unwrap_or_else(|| Path::new("."));
    let claude_dir = home.join(".claude").join("projects");
    if claude_dir.exists() {
        return claude_dir;
    }
    home.join(".config").join("claude").join("projects")
}

/// `<home>/.claude-swap-backup/sessions/<N>-<label>/projects` directories,
/// sorted by path. Only immediate children of `sessions` qualify; `<N>` must
/// be a positive integer and `projects` must be a directory (a dangling shared
/// history link is skipped without affecting the other homes).
fn swap_projects_roots(home: &Path) -> Vec<PathBuf> {
    let sessions = SWAP_SESSIONS_DIR
        .iter()
        .fold(home.to_path_buf(), |path, part| path.join(part));
    let Ok(entries) = fs::read_dir(&sessions) else {
        return Vec::new();
    };
    let mut slots: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| is_swap_slot_name(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
        .collect();
    slots.sort();
    slots
        .into_iter()
        .map(|slot| slot.join("projects"))
        .filter(|projects| projects.is_dir())
        .collect()
}

fn is_swap_slot_name(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    name.split_once('-')
        .and_then(|(number, _label)| number.parse::<i64>().ok())
        .is_some_and(|number| number > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_projects(root: &Path) -> PathBuf {
        let projects = root.join("projects");
        fs::create_dir_all(&projects).unwrap();
        projects
    }

    fn swap_home(home: &Path, slot: &str) -> PathBuf {
        make_projects(&home.join(".claude-swap-backup").join("sessions").join(slot))
    }

    #[test]
    fn slot_names_need_a_positive_integer_and_a_dash() {
        for accepted in ["1-first", "12-a-b", "3-", "+5-plus"] {
            assert!(is_swap_slot_name(accepted), "{accepted}");
        }
        for rejected in [
            "0-foo",
            "-1-foo",
            "unrelated",
            "abc-1",
            "7",
            "",
            ".1-hidden",
            " 1-x",
        ] {
            assert!(!is_swap_slot_name(rejected), "{rejected}");
        }
    }

    #[test]
    fn swap_homes_are_added_after_the_profile_root_sorted_by_path() {
        let home = tempfile::tempdir().unwrap();
        let base = make_projects(&home.path().join(".claude"));
        let second = swap_home(home.path(), "2-second");
        let first = swap_home(home.path(), "1-first");
        swap_home(home.path(), "unrelated");
        swap_home(home.path(), "0-zero");

        let roots = claude_projects_roots(None, Some(home.path()));
        assert_eq!(roots, vec![base, first, second]);
    }

    #[test]
    fn literal_config_dir_stays_beside_swap_homes() {
        let home = tempfile::tempdir().unwrap();
        let configured = tempfile::tempdir().unwrap();
        let configured_projects = make_projects(configured.path());
        make_projects(&home.path().join(".claude"));
        let swap = swap_home(home.path(), "2-second");

        let roots = claude_projects_roots(configured.path().to_str(), Some(home.path()));
        assert_eq!(roots, vec![configured_projects, swap]);
    }

    #[test]
    fn empty_config_dir_falls_back_to_the_default_root() {
        let home = tempfile::tempdir().unwrap();
        let default = make_projects(&home.path().join(".config").join("claude"));
        assert_eq!(
            claude_projects_roots(Some("  "), Some(home.path())),
            vec![default]
        );
    }

    #[test]
    fn missing_profile_root_does_not_stop_swap_homes() {
        let home = tempfile::tempdir().unwrap();
        let swap = swap_home(home.path(), "1-first");
        // A slot without a projects directory and a file named like a slot.
        fs::create_dir_all(
            home.path()
                .join(".claude-swap-backup")
                .join("sessions")
                .join("2-empty"),
        )
        .unwrap();
        fs::write(
            home.path()
                .join(".claude-swap-backup")
                .join("sessions")
                .join("3-file"),
            b"x",
        )
        .unwrap();

        assert_eq!(claude_projects_roots(None, Some(home.path())), vec![swap]);
        assert!(claude_projects_roots(None, Some(&home.path().join("nowhere"))).is_empty());
    }

    #[test]
    fn a_profile_root_matching_a_swap_home_by_resolved_path_is_listed_once() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".claude-swap-backup").join("sessions");
        let first = swap_home(home.path(), "1-first");
        let respelled = sessions.join("..").join("sessions").join("1-first");

        let roots = claude_projects_roots(respelled.to_str(), Some(home.path()));
        assert_eq!(roots.len(), 1);
        assert_eq!(
            fs::canonicalize(&roots[0]).unwrap(),
            fs::canonicalize(first).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn shared_history_symlink_is_scanned_once() {
        let home = tempfile::tempdir().unwrap();
        let first = swap_home(home.path(), "1-first");
        let shared = home
            .path()
            .join(".claude-swap-backup")
            .join("sessions")
            .join("3-shared");
        fs::create_dir_all(&shared).unwrap();
        // Creating a symlink needs a privilege on Windows; skip without it.
        if std::os::windows::fs::symlink_dir(&first, shared.join("projects")).is_err() {
            return;
        }

        assert_eq!(claude_projects_roots(None, Some(home.path())), vec![first]);
    }
}
