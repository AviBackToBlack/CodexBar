use super::*;

pub fn focus_session(session: &AgentSession) -> SessionFocusResult {
    match local_process(session) {
        Ok(pid) => focus_process(pid),
        Err(result) => result,
    }
}

/// Flash the session's taskbar button without restoring or activating its
/// window.
///
/// For callers that are not answering a user action, such as auto-resume
/// after a quota reset. Windows does not let a background process activate
/// another app's window, and [`focus_session`]'s `ShowWindow(SW_RESTORE)`
/// would still pop a minimized terminal up over the app the user is working
/// in. The flash stops once the user switches to the window.
///
/// Returns `Ok(())` once the window was found and is flashing; otherwise the
/// same reason [`focus_session`] would report.
pub fn request_session_attention(session: &AgentSession) -> Result<(), SessionFocusResult> {
    flash_process(local_process(session)?)
}

/// The local process that owns `session`'s window, or why there is none.
fn local_process(session: &AgentSession) -> Result<u32, SessionFocusResult> {
    match session.focus_target {
        AgentSessionFocusTarget::Transcript { .. } => Err(SessionFocusResult::unsupported(
            "This file-only session has no focusable Windows window.",
        )),
        AgentSessionFocusTarget::None => Err(SessionFocusResult::unsupported(
            "This session has no focus target on Windows.",
        )),
        AgentSessionFocusTarget::Process { pid } => {
            if is_local_host(&session.host) {
                Ok(pid)
            } else {
                Err(SessionFocusResult::unsupported(
                    "Remote session focus is not supported from this Windows desktop.",
                ))
            }
        }
    }
}

fn is_local_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host.is_empty()
    {
        return true;
    }

    std::env::var("COMPUTERNAME")
        .map(|name| name.eq_ignore_ascii_case(host))
        .unwrap_or(false)
}

/// The first visible top-level window owned by `pid`.
#[cfg(windows)]
fn find_process_window(pid: u32) -> Result<windows::Win32::Foundation::HWND, SessionFocusResult> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    struct Search {
        pid: u32,
        window: Option<HWND>,
    }

    // SAFETY: `find_window` is passed to `EnumWindows` as the callback below;
    // Windows invokes it only with valid top-level HWNDs during enumeration.
    // `data` is the pointer to our stack-allocated `Search` (passed as LPARAM
    // to EnumWindows), so it outlives every callback invocation.
    unsafe extern "system" fn find_window(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` was built from a mutable reference to `search`, which
        // stays alive on this thread for the whole EnumWindows call, so no other
        // code can access the pointee while this callback holds the reference.
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut window_pid = 0;
        // SAFETY: GetWindowThreadProcessId only reads `hwnd` (a window handle
        // handed to us by EnumWindows) and writes through the provided pointer.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut window_pid)) };
        // SAFETY: `hwnd` is a valid top-level window handle supplied by
        // EnumWindows; IsWindowVisible only reads it.
        if window_pid == search.pid && unsafe { IsWindowVisible(hwnd).as_bool() } {
            search.window = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut search = Search { pid, window: None };
    // SAFETY: EnumWindows runs `find_window` synchronously on this thread while
    // `search` lives on the stack; the LPARAM encodes a valid pointer to it.
    let result = unsafe { EnumWindows(Some(find_window), LPARAM(&mut search as *mut _ as isize)) };
    if result.is_err() {
        return Err(SessionFocusResult::failed(
            "Windows could not enumerate application windows.",
        ));
    }

    search.window.ok_or_else(|| {
        SessionFocusResult::failed("No focusable window was found for this session.")
    })
}

#[cfg(windows)]
fn focus_process(pid: u32) -> SessionFocusResult {
    use windows::Win32::UI::WindowsAndMessaging::{SW_RESTORE, SetForegroundWindow, ShowWindow};

    let window = match find_process_window(pid) {
        Ok(window) => window,
        Err(result) => return result,
    };
    // SAFETY: `window` is an HWND returned by EnumWindows, so it refers to a
    // live window; ShowWindow/SetForegroundWindow only read the handle and
    // their results are advisory UI hints whose failures we already handle.
    unsafe {
        let _restored = ShowWindow(window, SW_RESTORE);
        if SetForegroundWindow(window).as_bool() {
            SessionFocusResult::focused()
        } else {
            SessionFocusResult::failed("Windows denied the request to focus this session.")
        }
    }
}

#[cfg(windows)]
fn flash_process(pid: u32) -> Result<(), SessionFocusResult> {
    use windows::Win32::UI::WindowsAndMessaging::{
        FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx,
    };

    let window = find_process_window(pid)?;
    let size = u32::try_from(std::mem::size_of::<FLASHWINFO>())
        .map_err(|_| SessionFocusResult::failed("Windows could not flash this session."))?;
    let info = FLASHWINFO {
        cbSize: size,
        hwnd: window,
        // Flash the taskbar button until the window reaches the foreground.
        dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: `info` is a fully initialized FLASHWINFO that outlives the call
    // and `window` is a live HWND from EnumWindows. The return value is the
    // window's previous caption state, not an error code.
    let _was_active = unsafe { FlashWindowEx(&info) };
    Ok(())
}

#[cfg(not(windows))]
fn focus_process(_pid: u32) -> SessionFocusResult {
    SessionFocusResult::unsupported("Process focus requires the Windows desktop shell.")
}

#[cfg(not(windows))]
fn flash_process(_pid: u32) -> Result<(), SessionFocusResult> {
    Err(SessionFocusResult::unsupported(
        "Process focus requires the Windows desktop shell.",
    ))
}
