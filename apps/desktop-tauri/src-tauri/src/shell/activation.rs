//! When a CodexBar window may take keyboard focus.
//!
//! CodexBar is a tray utility. It takes the foreground only on a direct user
//! action: a tray click, a tray menu item, the global hotkey, or a click on a
//! control inside one of its own windows. Startup, a second instance handing
//! off to this one, refreshes, events and proof automation may show a
//! surface, but they must not move the foreground away from the app the user
//! is working in.
//!
//! `WebviewWindow::set_focus` is not a polite request on Windows. tao 0.34.8
//! (`src/platform_impl/windows/window.rs`, `Window::set_focus`, lines
//! 175-186) calls `force_window_active` (lines 1500-1527), which retries a
//! refused `SetForegroundWindow` after injecting a synthetic Alt press and
//! release through `SendInput` ("a little hack which can 'steal' the
//! foreground window permission"). That bypasses the foreground lock Windows
//! applies to background processes, and the injected Alt can open the menu
//! bar of the app the user is typing in. So `set_focus` is reserved for
//! [`Activation::UserAction`].
//!
//! Showing is kept apart from focusing: every CodexBar window is built with
//! `focused(false)` (`"focus": false` for `main` in `tauri.conf.json`), which
//! makes tao show it with `SW_SHOWNOACTIVATE` instead of `SW_SHOW` (tao
//! `window_state.rs`, `WindowFlags::apply_diff`, lines 325-337). [`apply`] is
//! the only place the focusing step happens.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::WebviewWindow;

/// Why a surface is being shown, which decides whether it may take focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    /// A direct user action (tray click, tray menu item, hotkey, a click in
    /// CodexBar's own UI). Windows lets the process that received the input
    /// take the foreground, so the window is focused.
    UserAction,
    /// Nothing the user clicked in CodexBar, but they may still expect the
    /// window in front: a launch from Start or Explorer, or a second instance
    /// they started handing off to this one. Windows is asked once with a
    /// plain `SetForegroundWindow`; if the foreground lock refuses, the
    /// window stays visible without focus.
    IfAllowed,
    /// Background work (proof automation, hide-to-tray recovery). The
    /// foreground is never touched.
    Never,
}

/// Set once at startup in proof mode (`CODEXBAR_PROOF_MODE`): surfaces are
/// shown for automation but never activated, whatever the caller asked for.
static SUPPRESSED: AtomicBool = AtomicBool::new(false);

/// Stop every CodexBar window from taking focus for the rest of the process.
pub fn suppress_all() {
    SUPPRESSED.store(true, Ordering::Relaxed);
}

fn is_suppressed() -> bool {
    SUPPRESSED.load(Ordering::Relaxed)
}

/// What [`apply`] does for one activation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusStep {
    /// Leave the foreground alone.
    Skip,
    /// `WebviewWindow::set_focus` (tao `force_window_active`).
    Force,
    /// One plain `SetForegroundWindow` that Windows may refuse.
    Request,
}

fn focus_step(activation: Activation, suppressed: bool) -> FocusStep {
    if suppressed {
        return FocusStep::Skip;
    }
    match activation {
        Activation::UserAction => FocusStep::Force,
        Activation::IfAllowed => FocusStep::Request,
        Activation::Never => FocusStep::Skip,
    }
}

/// Focus an already shown window as far as `activation` allows.
///
/// Call it after `WebviewWindow::show`. Both go through the event loop in
/// order, so the window is visible by the time the foreground request runs.
pub fn apply(window: &WebviewWindow, activation: Activation) -> Result<(), String> {
    match focus_step(activation, is_suppressed()) {
        FocusStep::Skip => Ok(()),
        FocusStep::Force => window.set_focus().map_err(|e| e.to_string()),
        FocusStep::Request => request_foreground(window),
    }
}

#[cfg(windows)]
fn request_foreground(window: &WebviewWindow) -> Result<(), String> {
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let Some(hwnd) = root_hwnd(&target) else {
                return;
            };
            // SAFETY: `hwnd` is the live top-level window of `target`, which
            // this closure keeps alive; these calls only read window state or
            // ask the window manager for the foreground, and never write
            // through the handle.
            unsafe {
                if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                    return;
                }
                if GetForegroundWindow() == hwnd {
                    return;
                }
                if SetForegroundWindow(hwnd) == 0 {
                    tracing::debug!(
                        "shell: Windows kept the current foreground window; the surface stays unfocused"
                    );
                }
            }
        })
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn request_foreground(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// Let the CodexBar instance that is already running take the foreground when
/// this process hands off to it.
///
/// A second launch is usually a user action (Start menu, Explorer, a
/// shortcut), so this process may set the foreground but the running one may
/// not. `tauri-plugin-single-instance` 2.4.1 (`src/platform_impl/windows.rs`)
/// finds the running instance through a hidden window of class `{id}-sic`
/// and title `{id}-siw`, sends it `WM_COPYDATA` and exits. Passing our
/// foreground permission on with `AllowSetForegroundWindow` first lets the
/// running instance's [`Activation::IfAllowed`] request succeed without the
/// `SendInput` Alt hack. Does nothing when no other instance is running.
#[cfg(windows)]
pub fn grant_foreground_to_running_instance(identifier: &str) {
    let class = wide(&format!("{identifier}-sic"));
    let title = wide(&format!("{identifier}-siw"));
    // SAFETY: both buffers are NUL-terminated UTF-16 strings that outlive the
    // call; the returned handle is only passed back to user32 as a value.
    let hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    if hwnd == 0 {
        return;
    }
    let mut pid = 0u32;
    // SAFETY: `pid` is a live out-parameter owned by this frame.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 || pid == std::process::id() {
        return;
    }
    // SAFETY: plain value arguments; the call only updates the window
    // manager's foreground permission for `pid`.
    if unsafe { AllowSetForegroundWindow(pid) } == 0 {
        tracing::debug!("shell: could not pass foreground permission to the running instance");
    }
}

#[cfg(not(windows))]
pub fn grant_foreground_to_running_instance(_identifier: &str) {}

/// Restore a minimized window without activating it. Run it on the main
/// thread.
///
/// tao's `unminimize` uses `ShowWindow(SW_RESTORE)`, which activates the
/// window (tao `window_state.rs`, `WindowFlags::apply_diff`, lines 390-402).
/// `SW_SHOWNOACTIVATE` restores the previous size and position and leaves
/// the foreground where it is. Call `unminimize` afterwards to refresh tao's
/// cached minimized flag: tao re-reads `IsIconic` before it diffs
/// (`window.rs`, `Window::set_minimized`, lines 584-598), finds the window
/// already restored and issues no `SW_RESTORE`.
#[cfg(windows)]
pub fn restore_minimized_without_activation(window: &impl raw_window_handle::HasWindowHandle) {
    const SW_SHOWNOACTIVATE: i32 = 4;
    let Some(hwnd) = root_hwnd(window) else {
        return;
    };
    // SAFETY: `hwnd` is the live top-level window behind `window`; IsIconic
    // only reads its state and ShowWindow only changes its show state.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

/// Other platforms have no foreground-lock hack to avoid, so the runtime's
/// own `unminimize` does the restore.
#[cfg(not(windows))]
pub fn restore_minimized_without_activation(_window: &impl raw_window_handle::HasWindowHandle) {}

#[cfg(windows)]
fn root_hwnd(window: &impl raw_window_handle::HasWindowHandle) -> Option<isize> {
    const GA_ROOT: u32 = 2;
    let handle = window.window_handle().ok()?;
    let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() else {
        return None;
    };
    let inner = h.hwnd.get();
    // SAFETY: `inner` is a live window handle from tao; GetAncestor only reads
    // the window tree.
    let root = unsafe { GetAncestor(inner, GA_ROOT) };
    Some(if root != 0 { root } else { inner })
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
#[link(name = "user32")]
// SAFETY: FFI declarations for user32 window-manager calls; every call site
// passes live window handles, NUL-terminated strings or caller-owned
// out-parameters.
unsafe extern "system" {
    fn GetAncestor(hwnd: isize, flags: u32) -> isize;
    fn IsWindowVisible(hwnd: isize) -> i32;
    fn IsIconic(hwnd: isize) -> i32;
    fn GetForegroundWindow() -> isize;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn FindWindowW(class_name: *const u16, window_name: *const u16) -> isize;
    fn GetWindowThreadProcessId(hwnd: isize, process_id: *mut u32) -> u32;
    fn AllowSetForegroundWindow(process_id: u32) -> i32;
    fn ShowWindow(hwnd: isize, cmd_show: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::{Activation, FocusStep, focus_step};

    #[test]
    fn user_actions_take_focus() {
        assert_eq!(focus_step(Activation::UserAction, false), FocusStep::Force);
    }

    #[test]
    fn launches_and_handoffs_only_ask_windows_for_the_foreground() {
        assert_eq!(focus_step(Activation::IfAllowed, false), FocusStep::Request);
    }

    #[test]
    fn background_work_never_touches_the_foreground() {
        assert_eq!(focus_step(Activation::Never, false), FocusStep::Skip);
    }

    #[test]
    fn proof_mode_never_activates_any_surface() {
        for activation in [
            Activation::UserAction,
            Activation::IfAllowed,
            Activation::Never,
        ] {
            assert_eq!(focus_step(activation, true), FocusStep::Skip);
        }
    }
}
