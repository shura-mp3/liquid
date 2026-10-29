// A release build must not pop a console window behind the overlay.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod diagnostics;
mod lyrics;
mod media;
mod platform;
mod state;
mod storage;
mod timing;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use state::AppState;
use storage::SettingsStore;

/// Actions a hotkey or tray item can trigger. Kept as an enum rather than raw
/// strings so a typo is a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    ToggleLyrics,
    ToggleHud,
    ToggleGlass,
    ToggleClickThrough,
    ToggleVisualiser,
    OpenSettings,
}

impl Action {
    fn from_settings_key(key: &str) -> Option<Self> {
        Some(match key {
            "toggleLyrics" => Self::ToggleLyrics,
            "toggleHud" => Self::ToggleHud,
            "toggleGlass" => Self::ToggleGlass,
            "toggleClickThrough" => Self::ToggleClickThrough,
            "toggleVisualiser" => Self::ToggleVisualiser,
            "openSettings" => Self::OpenSettings,
            _ => return None,
        })
    }

    /// Event name forwarded to the webviews for actions the UI owns.
    fn event(self) -> &'static str {
        match self {
            Self::ToggleLyrics => "liquid://toggle-lyrics",
            Self::ToggleHud => "liquid://toggle-hud",
            Self::ToggleGlass => "liquid://toggle-glass",
            Self::ToggleVisualiser => "liquid://toggle-visualiser",
            Self::ToggleClickThrough => "liquid://toggle-click-through",
            Self::OpenSettings => "liquid://open-settings",
        }
    }
}

fn run_action(app: &tauri::AppHandle, action: Action) {
    use tauri::Emitter;

    match action {
        Action::OpenSettings => {
            if let Err(e) = commands::show_settings_window(app.clone()) {
                tracing::warn!(error = %e, "could not show settings window");
            }
        }
        Action::ToggleClickThrough => {
            // Handled natively: the whole point of this hotkey is that it works when
            // the overlay cannot be clicked, so it must not depend on the webview.
            let state = app.state::<AppState>();
            match commands::toggle_click_through(app.clone(), state) {
                Ok(enabled) => {
                    tracing::info!(enabled, "click-through toggled");
                    let _ = app.emit(action.event(), enabled);
                }
                Err(e) => tracing::warn!(error = %e, "could not toggle click-through"),
            }
        }
        other => {
            if let Err(e) = app.emit(other.event(), ()) {
                tracing::warn!(error = %e, event = other.event(), "could not emit action");
            }
        }
    }
}

/// Create one transparent overlay window per monitor.
///
/// One window per monitor rather than a single spanning window: a spanning window
/// breaks per-monitor DPI, forces the largest monitor's backing store everywhere,
/// and makes click-through regions ambiguous.
fn create_overlays(app: &tauri::AppHandle) -> tauri::Result<()> {
    let monitors = match commands::get_monitors() {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => {
            tracing::error!("no monitors reported; not creating overlays");
            return Ok(());
        }
        Err(e) => {
            tracing::error!(error = %e, "monitor enumeration failed; not creating overlays");
            return Ok(());
        }
    };

    for (index, monitor) in monitors.iter().enumerate() {
        let label = format!("overlay-{index}");
        if app.get_webview_window(&label).is_some() {
            continue;
        }

        // Physical pixels from the OS, divided by this monitor's own scale factor,
        // because Tauri positions and sizes in logical units.
        let scale = if monitor.scale_factor > 0.0 {
            monitor.scale_factor
        } else {
            1.0
        };
        let x = f64::from(monitor.work_x) / scale;
        let y = f64::from(monitor.work_y) / scale;
        let w = f64::from(monitor.work_width) / scale;
        let h = f64::from(monitor.work_height) / scale;

        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .title("LIQUID Overlay")
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            // Never take activation: clicking the overlay must not pull focus out
            // of a game or editor.
            .focused(false)
            .shadow(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .position(x, y)
            .inner_size(w, h)
            .visible(false)
            .build()?;

        // Styles that Tauri does not expose are applied directly.
        if let Err(e) = commands::init_overlay(window.clone()) {
            tracing::error!(label = %label, error = %e, "could not apply overlay styles");
        }

        tracing::info!(
            label = %label,
            monitor = %monitor.id,
            scale,
            x, y, w, h,
            "overlay created"
        );
    }
    Ok(())
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open LIQUID settings", true, None::<&str>)?;
    let toggle_overlay =
        MenuItem::with_id(app, "overlay", "Show / hide overlay", true, None::<&str>)?;
    // The click-through escape hatch. Lives here precisely because the tray keeps
    // working when the overlay itself is unclickable.
    let rescue = MenuItem::with_id(
        app,
        "rescue",
        "Restore interactivity (disable click-through)",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit LIQUID", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &toggle_overlay, &rescue, &separator, &quit])?;

    TrayIconBuilder::with_id("liquid-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| tauri::Error::AssetNotFound("default window icon missing".into()))?,
        )
        .tooltip("LIQUID")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => run_action(app, Action::OpenSettings),
            "overlay" => {
                let state = app.state::<AppState>();
                let next = !state.settings.lock().overlay_visible;
                if let Err(e) = commands::set_overlay_visible(app.clone(), state, next) {
                    tracing::warn!(error = %e, "could not toggle overlay visibility");
                }
            }
            "rescue" => {
                let state = app.state::<AppState>();
                if let Err(e) = commands::restore_interactivity(app.clone(), state) {
                    tracing::warn!(error = %e, "rescue failed");
                } else {
                    tracing::info!("interactivity restored from tray");
                }
            }
            "quit" => app.exit(0),
            other => tracing::debug!(id = other, "unhandled tray menu item"),
        })
        .on_tray_icon_event(|tray, event| {
            // Left click opens settings, which is what a user expects from a tray
            // icon; the menu is on right click.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                run_action(tray.app_handle(), Action::OpenSettings);
            }
        })
        .build(app)?;

    Ok(())
}

fn register_hotkeys(app: &tauri::AppHandle) {
    let bindings: Vec<(String, Action)> = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock();
        settings
            .hotkeys
            .iter()
            .filter_map(|(key, combo)| Action::from_settings_key(key).map(|a| (combo.clone(), a)))
            .collect()
    };

    for (combo, action) in bindings {
        let handle = app.clone();
        // A combination already owned by another application fails to register.
        // That is reported, not swallowed: silently overriding or silently losing a
        // shortcut both leave the user guessing.
        match app
            .global_shortcut()
            .on_shortcut(combo.as_str(), move |_app, _shortcut, event| {
                if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    run_action(&handle, action);
                }
            }) {
            Ok(()) => tracing::info!(%combo, ?action, "hotkey registered"),
            Err(e) => tracing::warn!(
                %combo,
                ?action,
                error = %e,
                "hotkey could not be registered — it is probably already taken"
            ),
        }
    }
}

/// Start the Windows media-session bridge and forward its updates to the webviews.
///
/// Failure here is not fatal: the overlay, glass and settings all still work, and the
/// UI reports that track information is unavailable rather than silently showing an
/// empty HUD forever.
#[cfg(windows)]
fn start_media_bridge(app: &tauri::AppHandle) {
    use tauri::Emitter;

    let clock = {
        let state = app.state::<AppState>();
        std::sync::Arc::clone(&state.clock)
    };

    let emit_handle = app.clone();
    let bridge = media::gsmtc::GsmtcBridge::start(clock, move |snapshot, sessions| {
        // Fired from the bridge thread. AppHandle is Send + Sync, so emitting from
        // here is fine and avoids hopping through the main thread for every tick.
        let payload = serde_json::json!({
            "snapshot": snapshot,
            "sessions": sessions,
        });
        if let Err(e) = emit_handle.emit("liquid://playback", payload) {
            tracing::debug!(error = %e, "could not emit playback update");
        }
    });

    let state = app.state::<AppState>();
    *state.media.lock() = Some(bridge);
    tracing::info!("media session bridge started");
}

#[cfg(not(windows))]
fn start_media_bridge(_app: &tauri::AppHandle) {
    tracing::warn!("media sessions are only implemented for Windows");
}

fn main() {
    let start_minimised = std::env::args().any(|a| a == "--minimised");

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_monitors,
            commands::get_settings,
            commands::save_settings,
            commands::reset_settings,
            commands::init_overlay,
            commands::set_click_through,
            commands::toggle_click_through,
            commands::restore_interactivity,
            commands::set_overlay_visible,
            commands::assert_topmost,
            commands::overlay_status,
            commands::get_diagnostics,
            commands::set_launch_at_login,
            commands::show_settings_window,
            commands::report_frontend_ready,
            commands::get_media_status,
            commands::set_pinned_source,
            commands::media_transport,
            commands::media_seek,
            commands::get_artwork,
            commands::refresh_media_sessions,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            let data_dir = handle
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("liquid"));

            // Keep the log guard alive for the process lifetime by handing it to
            // Tauri's state; dropping it would silently stop file logging.
            if let Some(guard) = diagnostics::log::init(&data_dir.join("logs")) {
                app.manage(guard);
            }

            let store = SettingsStore::open(&data_dir.join("liquid.db")).map_err(|e| {
                tracing::error!(error = %e, "could not open settings database");
                e
            })?;
            let app_state = AppState::new(store, data_dir);
            let overlay_visible = app_state.settings.lock().overlay_visible;
            let want_click_through = app_state.settings.lock().click_through;
            app.manage(app_state);

            build_tray(&handle)?;
            create_overlays(&handle)?;
            register_hotkeys(&handle);
            start_media_bridge(&handle);

            // Restore the saved overlay state.
            if overlay_visible {
                let state = handle.state::<AppState>();
                if let Err(e) = commands::set_overlay_visible(handle.clone(), state, true) {
                    tracing::warn!(error = %e, "could not show overlays");
                }
            }
            if want_click_through {
                let state = handle.state::<AppState>();
                if let Err(e) = commands::set_click_through(handle.clone(), state, true) {
                    tracing::warn!(error = %e, "could not restore click-through");
                }
            }

            // The settings window starts hidden so `--minimised` can start straight
            // to the tray; otherwise it is shown now.
            if !start_minimised {
                if let Some(main) = handle.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            } else {
                tracing::info!("started minimised to tray");
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window hides it instead of quitting: the app is a
            // tray-resident HUD, and the overlay should survive closing settings.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running LIQUID");
}
