//! Creating and governing overlay windows.
//!
//! Every path that can put a window on screen goes through here, so the safety
//! rules cannot be bypassed by a caller that forgets them.

use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

use super::{OverlayState, DEFAULT_HEIGHT, DEFAULT_TOP_MARGIN, DEFAULT_WIDTH, READY_TIMEOUT};
use crate::platform::MonitorInfo;
use crate::state::AppState;

/// Create one overlay per monitor, hidden, small, and click-through.
///
/// Idempotent: a label that already exists is skipped, so a display hot-plug or a
/// double call cannot stack duplicate windows on top of each other.
pub fn create_overlays(app: &AppHandle, monitors: &[MonitorInfo]) -> tauri::Result<()> {
    let state = app.state::<AppState>();

    // A parking spot to the left of, and above, every monitor. Derived from the
    // actual layout rather than a magic constant, because a display can sit at
    // negative coordinates and a hard-coded "far away" could land on it.
    let park = offscreen_point(monitors);

    for (index, monitor) in monitors.iter().enumerate() {
        let label = format!("overlay-{index}");

        if app.get_webview_window(&label).is_some() || state.overlays.contains(&label) {
            tracing::debug!(%label, "overlay already exists; not creating a duplicate");
            continue;
        }

        let scale = if monitor.scale_factor > 0.0 {
            monitor.scale_factor
        } else {
            1.0
        };

        // Logical units, because Tauri positions and sizes in them. The monitor
        // rectangle is physical, so it is converted rather than assumed to match.
        let work_x = f64::from(monitor.work_x) / scale;
        let work_y = f64::from(monitor.work_y) / scale;
        let work_w = f64::from(monitor.work_width) / scale;

        // Top-centre of the monitor: visible without covering anything important.
        let x = work_x + (work_w - DEFAULT_WIDTH).max(0.0) / 2.0;
        let y = work_y + DEFAULT_TOP_MARGIN;

        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .title("LIQUID Overlay")
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            // Never take activation: clicking must not pull focus from a game.
            .focused(false)
            .shadow(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            // Created off-screen, not hidden.
            //
            // `visible(false)` looks safer and is actually a deadlock: WebView2
            // does not navigate a hidden window, so the frontend never loads, never
            // reports ready, and the window is never shown. Parking it outside the
            // desktop achieves the real goal — it cannot cover anything — while
            // letting it load. It is moved into place only once it reports ready.
            .position(park.0, park.1)
            .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
            .visible(true)
            .build()?;

        state.overlays.register(&label, &monitor.id, true, (x, y));
        state.overlays.set_state(&label, OverlayState::Loading);

        // Apply the native styles, including click-through, *before* the window
        // could ever be shown.
        if let Err(e) = apply_styles(&window, true) {
            tracing::error!(%label, error = %e, "could not apply overlay styles");
            state.overlays.fail(&label, e);
        }

        tracing::info!(
            %label, monitor = %monitor.id, scale,
            target_x = x, target_y = y,
            parked_x = park.0, parked_y = park.1,
            w = DEFAULT_WIDTH, h = DEFAULT_HEIGHT,
            "overlay created off-screen, click-through, awaiting frontend"
        );
    }
    Ok(())
}

/// A point guaranteed to be outside every monitor.
///
/// Computed from the real layout: a display can legitimately sit at negative
/// coordinates, so "somewhere far to the left" has to mean far to the left of the
/// leftmost one, not a fixed number.
fn offscreen_point(monitors: &[MonitorInfo]) -> (f64, f64) {
    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    (
        f64::from(min_x) - DEFAULT_WIDTH - 400.0,
        f64::from(min_y) - DEFAULT_HEIGHT - 400.0,
    )
}

/// Apply the overlay extended styles and the click-through state.
fn apply_styles(window: &tauri::WebviewWindow, click_through: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let hwnd = window
            .hwnd()
            .map_err(|e| format!("no window handle: {e}"))?;
        let handle = unsafe {
            crate::platform::overlay::WindowHandle::from_raw(hwnd.0 as isize)
                .map_err(|e| e.to_string())?
        };
        crate::platform::overlay::apply_overlay_styles(handle).map_err(|e| e.to_string())?;
        crate::platform::overlay::set_click_through(handle, click_through)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (window, click_through);
        Ok(())
    }
}

/// The frontend of `label` has mounted and is rendering.
///
/// This is the only route from `Loading` to a window the user can see.
pub fn mark_ready(app: &AppHandle, label: &str) {
    let state = app.state::<AppState>();

    match state.overlays.state(label) {
        Some(OverlayState::Created) | Some(OverlayState::Loading) => {}
        // Recovery. A frontend that reloads — after a hot reload in development,
        // or a webview crash in production — reports in again, and an overlay
        // previously written off by the watchdog should be allowed back. Refusing
        // this left a perfectly healthy overlay permanently dead. Safety is not
        // weakened: an emergency hide latches separately and still wins.
        Some(OverlayState::Failed) => {
            tracing::info!(%label, "overlay recovered after being failed");
        }
        Some(OverlayState::Destroyed) => return,
        // Already ready or on screen: a duplicate signal is harmless.
        Some(_) => return,
        None => return,
    }

    state.overlays.set_state(label, OverlayState::Ready);
    tracing::info!(%label, "overlay frontend ready");

    let wants_visible = state.settings.lock().overlay_visible;
    if wants_visible && !state.overlays.emergency_hidden() {
        if let Err(e) = show(app, label) {
            tracing::warn!(%label, error = %e, "could not show overlay after it became ready");
        }
    } else if let Some(window) = app.get_webview_window(label) {
        // Not wanted on screen: hide it where it is, still parked off-screen.
        let _ = window.hide();
        state.overlays.set_state(label, OverlayState::Hidden);
    }
}

/// Show one overlay, if its state allows it.
pub fn show(app: &AppHandle, label: &str) -> Result<(), String> {
    let state = app.state::<AppState>();

    if state.overlays.emergency_hidden() {
        return Ok(());
    }
    let Some(current) = state.overlays.state(label) else {
        return Ok(());
    };
    if !current.may_show() {
        // Not an error: this is the guard doing its job.
        tracing::debug!(%label, ?current, "refusing to show an overlay that is not ready");
        return Ok(());
    }

    let Some(window) = app.get_webview_window(label) else {
        return Ok(());
    };

    // Move it off the parking spot and into place. Done before showing so it
    // never appears at the parked coordinates first.
    if let Some((tx, ty)) = state.overlays.target(label) {
        window
            .set_position(LogicalPosition::new(tx, ty))
            .map_err(|e| e.to_string())?;
    }
    window.show().map_err(|e| e.to_string())?;
    // tao rewrites the extended-style word when it re-applies its own flags, which
    // `show()` triggers, so the styles are re-asserted afterwards or they are lost.
    let click_through = state
        .overlays
        .info()
        .into_iter()
        .find(|i| i.label == label)
        .map(|i| i.click_through)
        .unwrap_or(true);
    apply_styles(&window, click_through)?;

    state.overlays.set_state(label, OverlayState::Visible);
    Ok(())
}

pub fn hide(app: &AppHandle, label: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        window.hide().map_err(|e| e.to_string())?;
    }
    let state = app.state::<AppState>();
    // A failed overlay stays failed; hiding must not launder it into a normal state.
    if !state.overlays.state(label).is_some_and(|s| s.is_terminal()) {
        state.overlays.set_state(label, OverlayState::Hidden);
    }
    Ok(())
}

pub fn show_all(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>().overlays.set_emergency_hidden(false);
    for label in app.state::<AppState>().overlays.labels() {
        show(app, &label)?;
    }
    Ok(())
}

/// Hide every overlay immediately and latch them hidden.
///
/// The emergency escape. Deliberately native and dependency-free: it touches no
/// webview, waits on no JavaScript, and works when every frontend is hung — which
/// is exactly the situation it exists for. Latching matters too, so a readiness
/// signal arriving a moment later cannot put a window straight back on screen.
pub fn emergency_hide_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.overlays.set_emergency_hidden(true);

    for label in state.overlays.labels() {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.hide();
        }
        if !state
            .overlays
            .state(&label)
            .is_some_and(|s| s.is_terminal())
        {
            state.overlays.set_state(&label, OverlayState::Hidden);
        }
    }
    tracing::warn!("emergency hide: all overlays hidden and latched");
}

/// Turn click-through on or off for every overlay.
pub fn set_click_through_all(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    for label in state.overlays.labels() {
        if let Some(window) = app.get_webview_window(&label) {
            apply_styles(&window, enabled)?;
        }
        state.overlays.set_click_through(&label, enabled);
    }
    Ok(())
}

/// Watch for overlays that never become ready, and hide them.
///
/// Runs on its own thread and hops to the main thread to touch windows, because
/// window handles are main-thread-only. Without this, an overlay whose frontend
/// never loads would sit in `Loading` forever — invisible, so harmless, but with no
/// diagnosis offered to the user.
pub fn spawn_watchdog(app: &AppHandle) {
    let handle = app.clone();
    std::thread::Builder::new()
        .name("liquid-overlay-watchdog".into())
        .spawn(move || {
            // Check a few times across the timeout window rather than once.
            let interval = READY_TIMEOUT / 5;
            loop {
                std::thread::sleep(interval);

                let state = handle.state::<AppState>();
                let stuck = state.overlays.timed_out();
                if stuck.is_empty() {
                    // Nothing outstanding and nothing still loading: the watchdog
                    // has no further work to do.
                    let any_loading =
                        state.overlays.info().iter().any(|i| {
                            matches!(i.state, OverlayState::Created | OverlayState::Loading)
                        });
                    if !any_loading {
                        tracing::debug!("overlay watchdog finished: all overlays settled");
                        return;
                    }
                    continue;
                }

                for label in stuck {
                    tracing::error!(
                        %label,
                        timeout_s = READY_TIMEOUT.as_secs(),
                        "overlay never reported ready; failing and hiding it"
                    );
                    state.overlays.fail(
                        &label,
                        format!(
                            "The overlay's interface did not load within {}s. It has been \
                             hidden so it cannot block the desktop.",
                            READY_TIMEOUT.as_secs()
                        ),
                    );

                    let hide_handle = handle.clone();
                    let hide_label = label.clone();
                    let _ = handle.run_on_main_thread(move || {
                        if let Some(window) = hide_handle.get_webview_window(&hide_label) {
                            let _ = window.hide();
                        }
                    });
                }
            }
        })
        .expect("could not spawn the overlay watchdog");
}

/// Remove overlay windows whose monitor has gone away.
pub fn cleanup_stale(app: &AppHandle, monitors: &[MonitorInfo]) {
    let state = app.state::<AppState>();
    let live: Vec<String> = (0..monitors.len())
        .map(|i| format!("overlay-{i}"))
        .collect();

    for label in state.overlays.labels() {
        if live.contains(&label) {
            continue;
        }
        tracing::info!(%label, "closing a stale overlay whose monitor is gone");
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
        state.overlays.remove(&label);
    }
}

/// Belt and braces at startup: hide any overlay that somehow exists before the
/// manager knows about it, so a leftover window from a previous run or a crashed
/// session cannot be sitting on screen.
pub fn hide_unmanaged(app: &AppHandle) {
    let state = app.state::<AppState>();
    for (label, window) in app.webview_windows() {
        if label.starts_with("overlay-") && !state.overlays.contains(&label) {
            tracing::warn!(%label, "hiding an unmanaged overlay window");
            let _ = window.hide();
        }
    }
}
