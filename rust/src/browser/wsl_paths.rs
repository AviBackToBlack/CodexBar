//! WSL-aware browser detection and path resolution
//!
//! When running inside WSL, Windows browser data lives under /mnt/c/...
//! This module provides path resolvers that detect WSL and map
//! browser profile paths to their Windows host equivalents.

use crate::wsl;

use super::detection::{BrowserDetector, BrowserType, DetectedBrowser};

/// WSL-aware browser detector.
///
/// On native Linux, returns empty (no Windows browsers).
/// On WSL, detects Windows browsers via /mnt/c/ paths.
pub struct WslBrowserDetector;

impl WslBrowserDetector {
    /// Detect Windows browsers visible from WSL via /mnt/c/ paths.
    pub fn detect_all() -> Vec<DetectedBrowser> {
        if !wsl::is_wsl() {
            return Vec::new();
        }

        let appdata_local = match wsl::windows_appdata_local() {
            Some(p) => p,
            None => return Vec::new(),
        };

        let appdata_roaming = wsl::windows_appdata_roaming();
        BrowserType::all()
            .iter()
            .copied()
            .filter_map(|browser_type| {
                BrowserDetector::detect_in_roots(
                    browser_type,
                    &appdata_local,
                    appdata_roaming.as_deref(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_detector_reads_wsl_shaped_roots_without_roaming() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("mnt/c/Users/test/AppData/Local");
        let chrome = local.join("Google/Chrome Beta/User Data");
        std::fs::create_dir_all(chrome.join("Default")).unwrap();
        std::fs::create_dir_all(chrome.join("Profile 2")).unwrap();
        std::fs::create_dir_all(chrome.join("Profile 1")).unwrap();
        std::fs::create_dir_all(chrome.join("Other")).unwrap();

        let detected =
            BrowserDetector::detect_in_roots(BrowserType::ChromeBeta, &local, None).unwrap();
        assert_eq!(detected.user_data_dir, chrome);
        assert_eq!(detected.profiles[0].name, "Default");
        assert!(detected.profiles[0].is_default);
        let mut other_names: Vec<_> = detected.profiles[1..]
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        other_names.sort_unstable();
        assert_eq!(other_names, ["Profile 1", "Profile 2"]);
        assert!(
            detected.profiles[1..]
                .iter()
                .all(|profile| !profile.is_default)
        );
        assert!(BrowserDetector::detect_in_roots(BrowserType::Firefox, &local, None).is_none());
    }

    #[test]
    fn shared_detector_reads_firefox_from_wsl_roaming_root() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("mnt/c/Users/test/AppData/Local");
        let roaming = temp.path().join("mnt/c/Users/test/AppData/Roaming");
        let firefox = roaming.join("Mozilla/Firefox/Profiles");
        std::fs::create_dir_all(firefox.join("abc.default-release")).unwrap();
        std::fs::create_dir_all(firefox.join("def.other")).unwrap();
        std::fs::create_dir_all(local.join("Mozilla/Firefox/Profiles/fake.default")).unwrap();

        let detected =
            BrowserDetector::detect_in_roots(BrowserType::Firefox, &local, Some(&roaming)).unwrap();
        assert_eq!(detected.user_data_dir, firefox);
        assert_eq!(detected.profiles.len(), 2);
        assert!(
            detected
                .profiles
                .iter()
                .any(|profile| profile.name == "abc.default-release" && profile.is_default)
        );
        assert!(
            detected
                .profiles
                .iter()
                .any(|profile| profile.name == "def.other" && !profile.is_default)
        );
    }

    #[test]
    fn test_wsl_browser_detection() {
        let browsers = WslBrowserDetector::detect_all();
        if !wsl::is_wsl() {
            assert!(browsers.is_empty());
        }
    }
}
