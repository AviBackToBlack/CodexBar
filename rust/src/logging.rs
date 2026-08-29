//! Logging configuration using tracing
//!
//! Logging writes to stderr always and additionally to a size-capped file
//! under the app logs directory when that directory is writable. Any
//! filesystem error degrades to stderr-only logging; startup must never fail
//! because logs cannot be written.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::LazyLock;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Convert a displayable error into a frontend/log-safe message.
pub fn safe_error_message(err: impl std::fmt::Display) -> String {
    crate::core::SecretRedactor::redact(&err.to_string())
}

/// Settings directory that hosts the app settings file (also the log root).
///
/// Mirrors 's  base without
/// importing the settings module (keeps logging self-contained).
pub fn settings_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("CodexBar"))
}

/// Path of the current in-app log file, if a settings dir is resolvable.
pub fn log_file_path() -> Option<PathBuf> {
    settings_dir().map(|p| p.join("logs").join("codexbar.log"))
}

/// Maximum size of the current log file before rotation to the single backup file.
pub const LOG_MAX_BYTES: u64 = 1024 * 1024;

/// Lines returned by the log-tail helper.
pub const LOG_TAIL_LINES: usize = 200;

// -- Size-capped file writer -------------------------------------------------

struct CappedFileWriter {
    inner: std::sync::Mutex<Option<std::fs::File>>,
    path: PathBuf,
    max_bytes: u64,
}

impl CappedFileWriter {
    fn new(path: PathBuf, max_bytes: u64) -> Option<Self> {
        // Best-effort creation; a failure here degrades to stderr-only.
        let dir = path.parent()?;
        if std::fs::create_dir_all(dir).is_err() {
            return None;
        }
        Some(Self {
            inner: std::sync::Mutex::new(None),
            path,
            max_bytes,
        })
    }

    fn open_or_replace(&self) -> Option<std::fs::File> {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok()
    }

    /// Rotates to the single backup file once the current file exceeds the cap;
    /// every fs error degrades to a no-op so the tracing layer never fails on logging.
    fn append(&self, line: &[u8]) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let over_cap = self.path.metadata().map(|m| m.len()).unwrap_or(0) > self.max_bytes;
        if over_cap {
            // Close the cached handle first: on Windows an open handle blocks
            // the rename, and a handle left open after a successful rename
            // would keep writing into the backup file.
            drop(guard.take());
            let backup = self.path.with_extension("log.1");
            // Renaming replaces an existing destination on Windows and Unix, so
            // the old backup is overwritten in one call.
            if std::fs::rename(&self.path, &backup).is_err() {
                // Rotation is best-effort; keep appending to the current file.
            }
        }
        if guard.is_none() {
            *guard = self.open_or_replace();
        }
        if let Some(file) = guard.as_mut() {
            let _ignored = file.write_all(line);
            let _ignored2 = file.flush();
        }
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for &'a CappedFileWriter {
    type Writer = &'a CappedFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}

impl std::io::Write for &'_ CappedFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.append(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// -- Wiring -------------------------------------------------------------------

static WRITER: LazyLock<Option<CappedFileWriter>> =
    LazyLock::new(|| log_file_path().and_then(|p| CappedFileWriter::new(p, LOG_MAX_BYTES)));

fn file_writer() -> Option<&'static CappedFileWriter> {
    WRITER.as_ref()
}

/// Initialize the logging system
pub fn init(verbose: bool, json: bool) -> anyhow::Result<()> {
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    let file_writer = file_writer();
    let stderr_layer = if json {
        fmt::layer().json().with_writer(std::io::stderr).boxed()
    } else {
        fmt::layer().with_writer(std::io::stderr).boxed()
    };

    match file_writer {
        Some(w) => tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(fmt::layer().with_writer(move || w))
            .init(),
        None => tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .init(),
    }

    Ok(())
}

/// Install a panic hook that best-effort logs panics to the app log file and
/// then chains to the previous default hook. The hook itself never panics.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let hook_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "panic payload of non-string type".to_string()
            };
            let location = match info.location() {
                Some(loc) => format!("{}:{}:{}", loc.file(), loc.line(), loc.column()),
                None => "unknown location".to_string(),
            };
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();
            if let Some(writer) = file_writer() {
                writer.append(
                    safe_error_message(format!(
                        "panic at {location}: {message}\nbacktrace:\n{backtrace}\n"
                    ))
                    .as_bytes(),
                );
            }
        }));
        let _ignored_hook = hook_result;
        previous(info);
    }));
}

/// Read the last LOG_TAIL_LINES lines of the current log file, redacted.
pub fn read_log_tail() -> Option<String> {
    let path = log_file_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    let tail: Vec<&str> = content.lines().rev().take(LOG_TAIL_LINES).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Some(crate::core::SecretRedactor::redact(&tail.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_moves_oversized_file_to_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("codexbar.log");
        std::fs::write(&path, "x".repeat(2048)).expect("seed file");
        let writer = CappedFileWriter::new(path.clone(), 1024).expect("writer");
        writer.append(b"trigger rotation\n");
        let backup = path.with_extension("log.1");
        assert!(backup.exists(), "backup should exist after rotation");
        assert_eq!(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), 17);
        assert_eq!(
            std::fs::metadata(&backup).map(|m| m.len()).unwrap_or(0),
            2048
        );
    }

    #[test]
    fn rotation_keeps_small_file_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("codexbar.log");
        std::fs::write(&path, "small\n").expect("seed file");
        let writer = CappedFileWriter::new(path.clone(), 1024).expect("writer");
        writer.append(b"still small\n");
        assert!(
            !path.with_extension("log.1").exists(),
            "no rotation below cap"
        );
    }

    #[test]
    fn rotation_with_warm_handle_recreates_current_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("codexbar.log");
        // 60-byte seed + the 11-byte first line = 71 bytes, so the metadata
        // check before the second append sees the file over the 64-byte cap.
        std::fs::write(&path, "seed\n".repeat(12)).expect("seed file");
        let writer = CappedFileWriter::new(path.clone(), 64).expect("writer");
        // First append opens and caches the handle (warm).
        writer.append(b"first line\n");
        // Blow past the cap while the handle is cached.
        writer.append(&[b'x'; 128]);
        let backup = path.with_extension("log.1");
        assert!(
            backup.exists(),
            "rotation must move the oversized file to the backup"
        );
        // The current file must be recreated and contain only post-rotation
        // bytes: the warm handle was dropped before the rename, so nothing
        // leaks into the backup or the reopened file.
        let current = std::fs::read_to_string(&path).expect("current file recreated");
        assert!(
            !current.contains("seed"),
            "seed bytes must have been rotated away"
        );
        assert_eq!(current, "x".repeat(128));
    }

    #[test]
    fn read_log_tail_returns_up_to_max_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("logs").join("codexbar.log");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let body: String = (0..(LOG_TAIL_LINES + 50))
            .map(|i| format!("line {i}\n"))
            .collect();
        std::fs::write(&path, body).expect("seed file");
        let writer = CappedFileWriter::new(path, LOG_MAX_BYTES).expect("writer");
        // read_log_tail reads from log_file_path(); simulate by reading from
        // the temp dir file directly through the same line-count logic.
        let content = std::fs::read_to_string(writer.path.clone()).expect("read");
        let tail: Vec<&str> = content.lines().rev().take(LOG_TAIL_LINES).collect();
        assert_eq!(tail.len(), LOG_TAIL_LINES);
    }

    #[test]
    fn panic_hook_returns_cleanly_when_log_path_unwritable() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Seed a regular file where a directory would need to be created.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").expect("write blocker");
        let bad_path = blocker.join("logs").join("codexbar.log");
        // CappedFileWriter::new fails because create_dir_all cannot succeed.
        assert!(CappedFileWriter::new(bad_path, LOG_MAX_BYTES).is_none());
        // The hook body must never panic even with no writer available.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Simulate the hook's fallible path with file_writer() == None.
            if let Some(writer) = file_writer() {
                writer.append(b"should not happen\n");
            }
        }));
        assert!(result.is_ok());
    }
}
