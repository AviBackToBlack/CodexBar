//! Sanitized classification of a failed `agy` print-usage run.
//!
//! Port of upstream `AntigravityCLIPrintFailure` (CodexBar v0.65.0, #3865).
//! Raw stderr must never reach the user or the logs: it can embed
//! account-identifying URLs (Google profile pictures), proxy details or local
//! paths. stderr is only matched against the fixed markers below and every
//! branch produces fixed text, so the classification stays reviewable.

use std::sync::LazyLock;

use regex_lite::Regex;

/// A failed `agy` print-usage invocation, without raw subprocess output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CliPrintFailure {
    ExecutableNotFound,
    LaunchFailed,
    Exited { code: i32, reason: ExitReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExitReason {
    Unspecified,
    Network,
    EligibilityNetwork,
    Ineligible,
}

/// How a non-zero `agy` exit is surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExitClassification {
    /// `agy` showed a login prompt or reported a signed-out session.
    SignedOut,
    Failed(CliPrintFailure),
}

impl CliPrintFailure {
    /// Classify a failure to start the `agy` process.
    pub(super) fn from_spawn_error(error: &std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::NotFound {
            Self::ExecutableNotFound
        } else {
            Self::LaunchFailed
        }
    }

    fn message(self) -> String {
        match self {
            Self::ExecutableNotFound => "agy executable not found".to_string(),
            Self::LaunchFailed => "agy failed to launch".to_string(),
            Self::Exited { code, reason } => {
                let detail = match reason {
                    ExitReason::Unspecified => "",
                    ExitReason::Network => {
                        "; a network request failed (check network or proxy settings)"
                    }
                    ExitReason::EligibilityNetwork => {
                        "; the eligibility check failed on a network request (check network or proxy settings)"
                    }
                    ExitReason::Ineligible => "; the account is not eligible for Antigravity",
                };
                format!("agy exited {code}{detail}")
            }
        }
    }

    /// Fixed text, identical to upstream's `cliReportFailed` description.
    pub(super) fn description(self) -> String {
        format!("Antigravity CLI usage report failed: {}", self.message())
    }
}

/// Upstream `AntigravityCLIAuthenticationPrompt`: only a blocking login
/// prompt, an explicit Antigravity logout or exhausted keyring authentication
/// proves that `agy` cannot recover its credentials. Matched case-insensitively
/// on the ASCII projection of the output. The first pattern also covers
/// upstream's literal `Select login method:` evidence.
static AUTHENTICATION_PROMPTS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        r"(?i)select\s+login\s+method\s*:?",
        r"(?i)you\s+are\s+not\s+logged\s+into\s+antigravity",
        r"(?i)keyring\s*auth\s*:\s*timed\s+out\b",
    ]
    .map(|pattern| Regex::new(pattern).expect("valid agy authentication prompt pattern"))
});

const SIGN_IN_MARKERS: [&str; 7] = [
    "not logged in",
    "not signed in",
    "unauthenticated",
    "authentication required",
    "login required",
    "please log in",
    "please sign in",
];

const ELIGIBILITY_MARKERS: [&str; 5] = [
    "eligibility check failed",
    "not eligible",
    "does not support google tos",
    "unsupported country",
    "unsupported region",
];

/// Transport-shaped fragments Go CLIs print when a request dies on the wire.
/// Word boundaries keep `eof` and friends from matching inside other words.
static NETWORK_MARKERS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\beof\b",
        r"\btimed out\b",
        r"\btimeout\b",
        r"\bdeadline exceeded\b",
        r"\bi/o timeout\b",
        r"\bno such host\b",
        r"\bdns\b",
        r"\bconnection refused\b",
        r"\bconnection reset\b",
        r"\bnetwork is unreachable\b",
        r"\bunreachable\b",
        r"\bproxy\b",
        r"\bcertificate\b",
        r"\btls\b",
        r"\bssl\b",
        r#"get "http"#,
        r#"post "http"#,
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).expect("valid agy network marker pattern"))
    .collect()
});

fn contains_authentication_prompt(output: &[u8]) -> bool {
    let ascii: String = output
        .iter()
        .map(|&byte| {
            if byte.is_ascii() {
                char::from(byte)
            } else {
                ' '
            }
        })
        .collect();
    AUTHENTICATION_PROMPTS
        .iter()
        .any(|pattern| pattern.is_match(&ascii))
}

/// Classify a non-zero `agy` exit from its captured stderr. Nothing from
/// `stderr` is retained in the result.
pub(super) fn classify_exit(code: i32, stderr: &[u8]) -> ExitClassification {
    let text = String::from_utf8_lossy(stderr).to_lowercase();
    if contains_authentication_prompt(stderr)
        || SIGN_IN_MARKERS.iter().any(|marker| text.contains(marker))
    {
        return ExitClassification::SignedOut;
    }
    let eligibility_failed = ELIGIBILITY_MARKERS
        .iter()
        .any(|marker| text.contains(marker));
    let network_failed = NETWORK_MARKERS
        .iter()
        .any(|pattern| pattern.is_match(&text));
    let reason = match (eligibility_failed, network_failed) {
        (true, true) => ExitReason::EligibilityNetwork,
        (true, false) => ExitReason::Ineligible,
        (false, true) => ExitReason::Network,
        (false, false) => ExitReason::Unspecified,
    };
    ExitClassification::Failed(CliPrintFailure::Exited { code, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exited(code: i32, reason: ExitReason) -> ExitClassification {
        ExitClassification::Failed(CliPrintFailure::Exited { code, reason })
    }

    #[test]
    fn descriptions_match_upstream_fixed_text() {
        let cases = [
            (
                CliPrintFailure::ExecutableNotFound,
                "Antigravity CLI usage report failed: agy executable not found",
            ),
            (
                CliPrintFailure::LaunchFailed,
                "Antigravity CLI usage report failed: agy failed to launch",
            ),
            (
                CliPrintFailure::Exited {
                    code: 7,
                    reason: ExitReason::Unspecified,
                },
                "Antigravity CLI usage report failed: agy exited 7",
            ),
            (
                CliPrintFailure::Exited {
                    code: 1,
                    reason: ExitReason::Network,
                },
                "Antigravity CLI usage report failed: agy exited 1; a network request failed (check network or proxy settings)",
            ),
            (
                CliPrintFailure::Exited {
                    code: 1,
                    reason: ExitReason::EligibilityNetwork,
                },
                "Antigravity CLI usage report failed: agy exited 1; the eligibility check failed on a network request (check network or proxy settings)",
            ),
            (
                CliPrintFailure::Exited {
                    code: 1,
                    reason: ExitReason::Ineligible,
                },
                "Antigravity CLI usage report failed: agy exited 1; the account is not eligible for Antigravity",
            ),
        ];
        for (failure, expected) in cases {
            assert_eq!(failure.description(), expected);
        }
    }

    #[test]
    fn stderr_maps_to_upstream_categories() {
        let cases = [
            (
                r#"Eligibility check failed: failed to get profile picture: Get "https://lh3.googleusercontent.com/a/private": EOF"#,
                exited(1, ExitReason::EligibilityNetwork),
            ),
            (
                "Eligibility check failed: account does not support Google ToS",
                exited(1, ExitReason::Ineligible),
            ),
            (
                "You are not logged into Antigravity",
                ExitClassification::SignedOut,
            ),
            (
                r#"Post "https://usage.invalid/v1": dial tcp: no such host"#,
                exited(1, ExitReason::Network),
            ),
        ];
        for (stderr, expected) in cases {
            assert_eq!(classify_exit(1, stderr.as_bytes()), expected, "{stderr}");
        }
    }

    #[test]
    fn blank_and_unrecognized_stderr_stay_unspecified() {
        assert_eq!(classify_exit(2, b"  "), exited(2, ExitReason::Unspecified));
        assert_eq!(
            classify_exit(7, b"synthetic-private-diagnostic"),
            exited(7, ExitReason::Unspecified)
        );
    }

    #[test]
    fn login_prompts_and_sign_in_markers_mean_signed_out() {
        for stderr in [
            "Select login method:",
            "select   LOGIN\nmethod",
            "keyring auth: timed out",
            "Error: Not signed in",
            "please log in to continue",
            "UNAUTHENTICATED: request had invalid credentials",
        ] {
            assert_eq!(
                classify_exit(1, stderr.as_bytes()),
                ExitClassification::SignedOut,
                "{stderr}"
            );
        }
    }

    #[test]
    fn authentication_prompt_ignores_non_ascii_bytes() {
        // Box-drawing and other non-ASCII bytes around the prompt become
        // spaces, so the prompt still matches.
        let stderr = "\u{2502}Select\u{00a0}login method\u{2502}".as_bytes();
        assert_eq!(classify_exit(1, stderr), ExitClassification::SignedOut);
        assert_eq!(
            classify_exit(1, b"\xff\xfeYou are not logged into Antigravity"),
            ExitClassification::SignedOut
        );
    }

    #[test]
    fn network_markers_respect_word_boundaries() {
        assert_eq!(
            classify_exit(3, b"geoffrey sslx proxyless tlsv"),
            exited(3, ExitReason::Unspecified)
        );
        assert_eq!(
            classify_exit(3, b"read: connection reset by peer"),
            exited(3, ExitReason::Network)
        );
        assert_eq!(
            classify_exit(3, b"x509: CERTIFICATE signed by unknown authority"),
            exited(3, ExitReason::Network)
        );
    }

    #[test]
    fn spawn_errors_distinguish_a_missing_executable() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            CliPrintFailure::from_spawn_error(&missing),
            CliPrintFailure::ExecutableNotFound
        );
        assert_eq!(
            CliPrintFailure::from_spawn_error(&denied),
            CliPrintFailure::LaunchFailed
        );
    }

    #[test]
    fn classification_never_retains_stderr_text() {
        let stderr = r#"Eligibility check failed: Get "https://lh3.googleusercontent.com/a/private": EOF C:\Users\someone\agy.exe user@example.com"#;
        let ExitClassification::Failed(failure) = classify_exit(1, stderr.as_bytes()) else {
            panic!("expected a classified failure");
        };
        let description = failure.description();
        for secret in [
            "googleusercontent",
            "private",
            r"C:\Users",
            "user@example.com",
        ] {
            assert!(
                !description.contains(secret),
                "{secret} leaked into {description}"
            );
        }
    }
}
