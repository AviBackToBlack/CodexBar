//! Start a console CLI in its own console window without taking focus.
//!
//! Auto-resume reopens a captured CLI session after a background refresh, not
//! in answer to a user action, so the new console must not come up over the
//! app the user is working in. `std::process::Command` cannot set
//! `STARTUPINFOW.wShowWindow` on stable Rust (`CommandExt::show_window` is
//! unstable, rust-lang/rust#127544). On Windows this module calls
//! `CreateProcessW` directly with `STARTF_USESHOWWINDOW` and
//! `SW_SHOWMINNOACTIVE`, the request `start /min` makes: the console starts
//! minimized on the taskbar and the foreground window keeps focus.

use std::path::Path;

/// Start `program` with `args` in a new console window that starts minimized
/// and does not take focus, with `cwd` as its working directory.
///
/// Batch shims (`.cmd` and `.bat`, such as npm's `claude.cmd`) run through
/// `cmd.exe` from the Windows system directory, with the script path and each
/// argument quoted. cmd.exe has no escape for `"`, `%` or control characters
/// inside quotes, so a script path or argument containing one is rejected.
///
/// The child inherits this process's environment but no handles, and runs in
/// its own process group. Returns once the process has started.
#[cfg(windows)]
pub fn spawn_minimized_console(program: &Path, args: &[String], cwd: &Path) -> Result<(), String> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CREATE_NEW_PROCESS_GROUP, CreateProcessW, PROCESS_INFORMATION,
        STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE;
    use windows::core::{PCWSTR, PWSTR};

    let program =
        std::path::absolute(program).map_err(|error| format!("invalid program path: {error}"))?;
    let LaunchLine {
        application,
        mut command_line,
    } = launch_line(&program, args, &system_directory()?)?;
    let application = wide_nul(application.as_os_str())?;
    let cwd = wide_nul(cwd.as_os_str())?;
    let startup = STARTUPINFOW {
        cb: u32::try_from(std::mem::size_of::<STARTUPINFOW>())
            .map_err(|error| format!("invalid startup info size: {error}"))?,
        dwFlags: STARTF_USESHOWWINDOW,
        wShowWindow: u16::try_from(SW_SHOWMINNOACTIVE.0)
            .map_err(|error| format!("invalid show-window command: {error}"))?,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: `application`, `command_line` and `cwd` are NUL-terminated UTF-16
    // buffers and `startup` is an initialized STARTUPINFOW; all of them outlive
    // the call. `command_line` is a mutable buffer, as CreateProcessW requires.
    // The kernel fills `info` on success.
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            PWSTR(command_line.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR(cwd.as_ptr()),
            &startup,
            &mut info,
        )
    }
    .map_err(|error| error.to_string())?;
    // SAFETY: CreateProcessW succeeded, so `hThread` is a valid handle owned
    // by this process; the OwnedHandle closes it exactly once.
    drop(unsafe { OwnedHandle::from_raw_handle(info.hThread.0) });
    // SAFETY: CreateProcessW succeeded, so `hProcess` is a valid handle owned
    // by this process; the OwnedHandle closes it exactly once. The child keeps
    // running after its handle closes.
    drop(unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) });
    Ok(())
}

/// Start `program` with `args` in `cwd`. Only Windows has console windows to
/// keep in the background.
#[cfg(not(windows))]
pub fn spawn_minimized_console(program: &Path, args: &[String], cwd: &Path) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .spawn()
        .map(|_child| ())
        .map_err(|error| error.to_string())
}

/// What `CreateProcessW` runs for one launch.
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct LaunchLine {
    /// Absolute path of the module to execute (`lpApplicationName`).
    application: std::path::PathBuf,
    /// Full command line as NUL-terminated UTF-16 (`lpCommandLine`).
    command_line: Vec<u16>,
}

/// Build the module path and command line for `program`, which must be
/// absolute.
///
/// Executables get the MSVC argument quoting their C runtime parses. Batch
/// scripts run as `"<system>\cmd.exe" /d /v:off /s /c ""<script>" "<arg>" ..."`:
/// `/d` skips AutoRun commands, `/v:off` keeps `!` literal, and `/s` makes
/// cmd.exe strip only the outer pair of quotes. Inside the remaining quotes,
/// `& | < > ^ ( )` are literal.
#[cfg(windows)]
fn launch_line(program: &Path, args: &[String], system_dir: &Path) -> Result<LaunchLine, String> {
    if !is_batch_script(program) {
        let args: Vec<std::ffi::OsString> = args.iter().map(std::ffi::OsString::from).collect();
        let command_line = crate::managed_process::build_command_line(program, &args)
            .map_err(|error| error.to_string())?;
        return Ok(LaunchLine {
            application: program.to_path_buf(),
            command_line,
        });
    }

    let cmd = system_dir.join("cmd.exe");
    let mut command_line = Vec::new();
    push_cmd_quoted(cmd.as_os_str(), &mut command_line)?;
    command_line.extend(" /d /v:off /s /c \"".encode_utf16());
    push_cmd_quoted(program.as_os_str(), &mut command_line)?;
    for arg in args {
        command_line.push(u16::from(b' '));
        push_cmd_quoted(std::ffi::OsStr::new(arg), &mut command_line)?;
    }
    command_line.push(u16::from(b'"'));
    command_line.push(0);
    Ok(LaunchLine {
        application: cmd,
        command_line,
    })
}

#[cfg(windows)]
fn is_batch_script(program: &Path) -> bool {
    program.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
    })
}

/// Append `value` wrapped in double quotes, for a cmd.exe command line.
#[cfg(windows)]
fn push_cmd_quoted(value: &std::ffi::OsStr, output: &mut Vec<u16>) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    let start = output.len();
    output.push(u16::from(b'"'));
    for unit in value.encode_wide() {
        if unit < 0x20 || unit == u16::from(b'"') || unit == u16::from(b'%') {
            output.truncate(start);
            return Err(
                "a batch script path or argument contains a quote, percent sign or control character"
                    .to_string(),
            );
        }
        output.push(unit);
    }
    output.push(u16::from(b'"'));
    Ok(())
}

#[cfg(windows)]
fn wide_nul(value: &std::ffi::OsStr) -> Result<Vec<u16>, String> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err("path contains an embedded NUL".to_string());
    }
    wide.push(0);
    Ok(wide)
}

/// The Windows system directory (`C:\Windows\System32`), from
/// `GetSystemDirectoryW` rather than the environment.
#[cfg(windows)]
fn system_directory() -> Result<std::path::PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "kernel32")]
    // SAFETY: FFI declaration for GetSystemDirectoryW; the only call site
    // passes a live, writable buffer and its length in UTF-16 units.
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }

    let mut buffer = vec![0_u16; 260];
    loop {
        let capacity = u32::try_from(buffer.len())
            .map_err(|error| format!("invalid system directory buffer: {error}"))?;
        // SAFETY: `buffer` is a live, writable allocation of `capacity` UTF-16
        // units, and GetSystemDirectoryW writes at most `capacity` units.
        let written = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), capacity) };
        let written = usize::try_from(written)
            .map_err(|error| format!("invalid system directory length: {error}"))?;
        if written == 0 {
            return Err("Windows did not report its system directory".to_string());
        }
        if written < buffer.len() {
            buffer.truncate(written);
            return Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                &buffer,
            )));
        }
        // Too small: `written` is the size needed, including the NUL.
        buffer.resize(written, 0);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn text(command_line: &[u16]) -> String {
        let (last, line) = command_line.split_last().expect("command line is empty");
        assert_eq!(*last, 0, "command line must be NUL-terminated");
        String::from_utf16(line).expect("command line is not UTF-16")
    }

    #[test]
    fn executables_use_msvc_quoting() {
        let program = PathBuf::from(r"C:\Program Files\Codex\codex.exe");
        let args = ["resume".to_string(), "0199a-session".to_string()];

        let launch = launch_line(&program, &args, Path::new(r"C:\Windows\System32"))
            .expect("executable launch line");

        assert_eq!(launch.application, program);
        assert_eq!(
            text(&launch.command_line),
            r#""C:\Program Files\Codex\codex.exe" resume 0199a-session"#
        );
    }

    #[test]
    fn batch_shims_run_through_system_cmd() {
        let program = PathBuf::from(r"C:\Users\dev\AppData\Roaming\npm\claude.cmd");
        let args = ["--resume".to_string(), "session-1".to_string()];

        let launch = launch_line(&program, &args, Path::new(r"C:\Windows\System32"))
            .expect("batch launch line");

        assert_eq!(
            launch.application,
            PathBuf::from(r"C:\Windows\System32\cmd.exe")
        );
        assert_eq!(
            text(&launch.command_line),
            r#""C:\Windows\System32\cmd.exe" /d /v:off /s /c ""C:\Users\dev\AppData\Roaming\npm\claude.cmd" "--resume" "session-1"""#
        );
    }

    #[test]
    fn batch_detection_ignores_case_and_keeps_cmd_metacharacters_quoted() {
        let program = PathBuf::from(r"C:\Program Files (x86)\R&D\codex.BAT");

        let launch = launch_line(&program, &[], Path::new(r"C:\Windows\System32"))
            .expect("batch launch line");

        assert_eq!(
            launch.application,
            PathBuf::from(r"C:\Windows\System32\cmd.exe")
        );
        assert_eq!(
            text(&launch.command_line),
            r#""C:\Windows\System32\cmd.exe" /d /v:off /s /c ""C:\Program Files (x86)\R&D\codex.BAT"""#
        );
    }

    #[test]
    fn batch_values_cmd_cannot_quote_are_rejected() {
        let system = Path::new(r"C:\Windows\System32");
        let script = PathBuf::from(r"C:\npm\claude.cmd");
        for arg in ["a\"b", "%PATH%", "line\nbreak", "tab\there"] {
            assert!(
                launch_line(&script, &[arg.to_string()], system).is_err(),
                "argument {arg:?} must be rejected"
            );
        }
        assert!(launch_line(Path::new(r"C:\100%\claude.cmd"), &[], system).is_err());
    }

    #[test]
    fn system_directory_holds_cmd() {
        let system = system_directory().expect("system directory");
        assert!(system.is_absolute(), "{}", system.display());
        assert!(system.join("cmd.exe").is_file(), "{}", system.display());
    }
}
