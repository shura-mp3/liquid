//! Tauri commands: the typed IPC surface between the webviews and the native layer.
//!
//! Every command returns `Result<_, String>` so a failure surfaces in the UI as a
//! message the user can act on, rather than a silently rejected promise.

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::platform::{self, MonitorInfo, OverlayObstruction};
use crate::state::AppState;
use crate::storage::Settings;

/// Pull the raw HWND out of a Tauri window.
///
/// Tauri links its own version of the `windows` crate, so its `HWND` is a different
/// Rust type from ours even when the versions match. Both are a pointer, so the
/// handle is passed across as an `isize` rather than trying to unify the types.
#[cfg(windows)]
fn handle_of(window: &WebviewWindow) -> Result<platform::overlay::WindowHandle, String> {
    let hwnd = window
        .hwnd()
        .map_err(|e| format!("no window handle for {}: {e}", window.label()))?;
    unsafe {
        platform::overlay::WindowHandle::from_raw(hwnd.0 as isize)
            .map_err(|e| format!("invalid window handle: {e}"))
    }
}

#[cfg(not(windows))]
fn handle_of(_window: &WebviewWindow) -> Result<(), String> {
    Err("overlay control is only implemented for Windows".into())
}

fn overlay_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with("overlay-") || label == "island")
        .map(|(_, window)| window)
        .collect()
}

#[tauri::command]
pub fn get_monitors() -> Result<Vec<MonitorInfo>, String> {
    #[cfg(windows)]
    {
        platform::monitors::enumerate().map_err(|e| format!("could not enumerate monitors: {e}"))
    }
    #[cfg(not(windows))]
    {
        Err("monitor enumeration is only implemented for Windows".into())
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    *state.settings.lock() = settings;
    state.persist()
}

#[tauri::command]
pub fn reset_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state
        .store
        .lock()
        .reset()
        .map_err(|e| format!("could not reset settings: {e}"))?;
    let defaults = Settings::default();
    *state.settings.lock() = defaults.clone();
    state.persist()?;
    Ok(defaults)
}

/// Apply the overlay style set to a window. Called once per overlay after creation.
#[tauri::command]
pub fn init_overlay(window: WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    {
        let handle = handle_of(&window)?;
        platform::overlay::apply_overlay_styles(handle)
            .map_err(|e| format!("could not apply overlay styles: {e}"))
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("overlay styles are only implemented for Windows".into())
    }
}

#[tauri::command]
pub fn set_click_through(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    if !state.may_toggle_click_through() {
        // Not an error: report the unchanged state so a jittery UI control settles
        // instead of retrying and provoking the Win10 black-transparency bug.
        return Ok(state.settings.lock().click_through);
    }

    #[cfg(windows)]
    for window in overlay_windows(&app) {
        let handle = handle_of(&window)?;
        platform::overlay::set_click_through(handle, enabled)
            .map_err(|e| format!("could not set click-through: {e}"))?;
    }
    #[cfg(not(windows))]
    let _ = &app;

    state.settings.lock().click_through = enabled;
    state.persist()?;
    Ok(enabled)
}

#[tauri::command]
pub fn toggle_click_through(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    let next = !state.settings.lock().click_through;
    set_click_through(app, state, next)
}

/// Unconditionally restore interactivity.
///
/// The escape hatch for click-through. Bound to the tray menu and a global hotkey,
/// both of which keep working when the overlay itself cannot be clicked, so the user
/// can never be locked out. It bypasses the debounce on purpose: a rescue must not
/// be rate-limited.
#[tauri::command]
pub fn restore_interactivity(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(windows)]
    for window in overlay_windows(&app) {
        let handle = handle_of(&window)?;
        platform::overlay::set_click_through(handle, false)
            .map_err(|e| format!("could not clear click-through: {e}"))?;
        let _ = platform::overlay::assert_topmost(handle);
        let _ = window.show();
    }
    #[cfg(not(windows))]
    let _ = &app;

    {
        let mut settings = state.settings.lock();
        settings.click_through = false;
        settings.overlay_visible = true;
    }
    state.persist()
}

#[tauri::command]
pub fn set_overlay_visible(
    app: AppHandle,
    state: State<'_, AppState>,
    visible: bool,
) -> Result<(), String> {
    for window in overlay_windows(&app) {
        let result = if visible {
            window.show()
        } else {
            window.hide()
        };
        result.map_err(|e| format!("could not change overlay visibility: {e}"))?;

        // tao rewrites the whole extended-style word from its own flag model when it
        // re-applies window flags, which `show()` does — so the overlay styles must
        // be re-asserted afterwards or they are silently lost.
        #[cfg(windows)]
        if visible {
            let handle = handle_of(&window)?;
            platform::overlay::apply_overlay_styles(handle)
                .map_err(|e| format!("could not re-apply overlay styles after show: {e}"))?;
        }
    }
    state.settings.lock().overlay_visible = visible;
    state.persist()
}

/// Re-assert topmost Z-order for every overlay.
///
/// Exposed because other applications take the topmost slot legitimately; the UI
/// calls this on foreground-change rather than from a timer.
#[tauri::command]
pub fn assert_topmost(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    for window in overlay_windows(&app) {
        let handle = handle_of(&window)?;
        // Re-assert the full style set, not just Z-order: tao can rewrite the style
        // word out from under us, so this doubles as the self-healing path.
        platform::overlay::apply_overlay_styles(handle)
            .map_err(|e| format!("could not re-assert overlay styles: {e}"))?;
    }
    #[cfg(not(windows))]
    let _ = &app;
    Ok(())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStatus {
    pub label: String,
    pub topmost: bool,
    pub click_through: bool,
    pub visible: bool,
    /// Whether the full required extended-style set is currently present. Shown so a
    /// style that tao has clobbered is visible instead of being a mystery.
    pub styles_ok: bool,
    pub obstruction: OverlayObstruction,
}

/// Report the overlay's real state, read back from the OS rather than from our own
/// bookkeeping. Diagnostics that echo intent instead of reality are worthless.
#[tauri::command]
pub fn overlay_status(app: AppHandle) -> Result<Vec<OverlayStatus>, String> {
    let mut out = Vec::new();
    for window in overlay_windows(&app) {
        #[cfg(windows)]
        {
            let handle = handle_of(&window)?;
            out.push(OverlayStatus {
                label: window.label().to_string(),
                topmost: platform::overlay::is_topmost(handle),
                click_through: platform::overlay::is_click_through(handle),
                visible: window.is_visible().unwrap_or(false),
                styles_ok: platform::overlay::has_overlay_styles(handle),
                obstruction: platform::overlay::detect_obstruction(handle),
            });
        }
        #[cfg(not(windows))]
        {
            let _ = &window;
        }
    }
    Ok(out)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub app_version: String,
    pub monotonic_clock_degraded: bool,
    pub monotonic_now_ms: f64,
    pub monitor_count: usize,
    pub launches_at_login: bool,
    /// Phase 2+ subsystems. Reported as unavailable rather than faked.
    pub audio_capture: &'static str,
    pub media_session: &'static str,
}

#[tauri::command]
pub fn get_diagnostics(state: State<'_, AppState>) -> Diagnostics {
    Diagnostics {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        monotonic_clock_degraded: state.clock.is_degraded(),
        monotonic_now_ms: state.clock.now_ms(),
        monitor_count: get_monitors().map(|m| m.len()).unwrap_or(0),
        #[cfg(windows)]
        launches_at_login: platform::startup::launches_at_login(),
        #[cfg(not(windows))]
        launches_at_login: false,
        audio_capture: "not implemented (Phase 4)",
        media_session: "not implemented (Phase 2)",
    }
}

#[tauri::command]
pub fn set_launch_at_login(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let minimised = state.settings.lock().start_minimised;
        platform::startup::set_launch_at_login(enabled, minimised)
            .map_err(|e| format!("could not change launch-at-login: {e}"))?;
    }
    state.settings.lock().launch_at_login = enabled;
    state.persist()
}

#[tauri::command]
pub fn show_settings_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "settings window is not open".to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}
