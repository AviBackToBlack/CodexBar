use super::*;
use std::fs;

const PREFIXES: &[&str] = &[
    "https_platform.minimax.io_",
    "https_www.minimax.io_",
    "https_minimax.io_",
    "https_platform.minimaxi.com_",
    "https_minimaxi.com_",
    "https_www.minimaxi.com_",
];

fn indexed_db() -> StorageKind {
    StorageKind::IndexedDb {
        origin_prefixes: PREFIXES,
    }
}

fn mkdir(root: &Path, relative: &str) {
    fs::create_dir_all(root.join(relative)).unwrap();
}

fn paths(root: &Path, candidates: &[StorageCandidate]) -> Vec<String> {
    candidates
        .iter()
        .map(|candidate| {
            candidate
                .path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn each_storage_kind_resolves_to_its_profile_directory() {
    let root = tempfile::tempdir().unwrap();
    let database = "IndexedDB/https_platform.minimax.io_0.indexeddb.leveldb";
    for path in ["Local Storage/leveldb", "Session Storage", database] {
        mkdir(root.path(), &format!("Default/{path}"));
    }

    let cases = [
        (
            StorageKind::LocalStorage,
            "Default/Local Storage/leveldb",
            "",
        ),
        (
            StorageKind::SessionStorage,
            "Default/Session Storage",
            " (Session Storage)",
        ),
        (indexed_db(), &format!("Default/{database}"), " (IndexedDB)"),
    ];
    for (kind, expected_path, suffix) in cases {
        let found = candidates_in_user_data_dir(root.path(), "Google Chrome", kind);
        assert_eq!(paths(root.path(), &found), [expected_path]);
        assert_eq!(found[0].label, format!("Google Chrome Default{suffix}"));
    }
}

#[test]
fn missing_stores_and_missing_user_data_dir_yield_nothing() {
    let root = tempfile::tempdir().unwrap();
    mkdir(root.path(), "Default");
    for kind in [
        StorageKind::LocalStorage,
        StorageKind::SessionStorage,
        indexed_db(),
    ] {
        assert!(candidates_in_user_data_dir(root.path(), "Chrome", kind).is_empty());
        assert!(
            candidates_in_user_data_dir(&root.path().join("absent"), "Chrome", kind).is_empty()
        );
    }
}

#[test]
fn only_default_profile_and_user_profiles_are_visited_in_name_order() {
    let root = tempfile::tempdir().unwrap();
    for profile in [
        "user-work",
        "Profile 2",
        "Default",
        "Guest Profile",
        "System Profile",
        ".hidden",
        "Profile1",
    ] {
        mkdir(root.path(), &format!("{profile}/Local Storage/leveldb"));
    }
    // A file named like a profile is not a profile.
    fs::write(root.path().join("Profile 9"), b"x").unwrap();

    let found = candidates_in_user_data_dir(root.path(), "Edge", StorageKind::LocalStorage);
    assert_eq!(
        found
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>(),
        ["Edge Default", "Edge Profile 2", "Edge user-work"]
    );
}

#[test]
fn indexed_db_keeps_only_allowed_origin_databases_that_are_directories() {
    let root = tempfile::tempdir().unwrap();
    let allowed = [
        "https_platform.minimax.io_0",
        "https_www.minimax.io_0",
        "https_minimax.io_0",
        "https_platform.minimaxi.com_0",
        "https_www.minimaxi.com_0",
        "https_minimaxi.com_0",
    ]
    .map(|origin| format!("{origin}.indexeddb.leveldb"));
    let excluded = [
        "https_other.example_0.indexeddb.leveldb",
        "https_minimax.io.evil_0.indexeddb.leveldb",
        "https_minimax.io_0.indexeddb.blob",
        ".https_minimax.io_0.indexeddb.leveldb",
    ];
    for profile in ["Default", "Profile 2", "user-work", "Guest Profile"] {
        for database in allowed.iter().map(String::as_str).chain(excluded) {
            mkdir(root.path(), &format!("{profile}/IndexedDB/{database}"));
        }
    }
    fs::write(
        root.path()
            .join("Default/IndexedDB/https_minimax.io_1.indexeddb.leveldb"),
        b"x",
    )
    .unwrap();

    let found = candidates_in_user_data_dir(root.path(), "Fixture", indexed_db());

    assert_eq!(found.len(), 3 * allowed.len());
    let mut expected_names: Vec<&str> = allowed.iter().map(String::as_str).collect();
    expected_names.sort_unstable();
    for (profile, group) in ["Default", "Profile 2", "user-work"]
        .iter()
        .zip(found.chunks(allowed.len()))
    {
        let names: Vec<_> = group
            .iter()
            .map(|candidate| candidate.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, expected_names);
        assert!(
            group
                .iter()
                .all(|candidate| candidate.label == format!("Fixture {profile} (IndexedDB)"))
        );
    }
}
