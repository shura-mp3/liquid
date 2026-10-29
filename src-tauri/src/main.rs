// A release build must not pop a console window behind the overlay.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod commands;
mod diagnostics;
mod lyrics;
mod media;
mod overlays;
mod platform;
mod state;
mod storage;
mod timing;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use state::AppState;
use storage::SettingsStore;

/// Actions a hotkey or tray item can trigger. Kept as an enum rather than raw
/// strings so a typo is a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Hide every overlay at once. The escape hatch, handled natively.
    EmergencyHide,
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
            "emergencyHide" => Self::EmergencyHide,
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
            Self::EmergencyHide => "liquid://emergency-hide",
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
        Action::EmergencyHide => {
            // Deliberately does not touch a webview: this must work when every
            // frontend is hung, which is the only time it is needed.
            overlays::manager::emergency_hide_all(app);
            let _ = commands::show_settings_window(app.clone());
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

/// Create the per-monitor overlays through the manager.
///
/// The manager owns every safety rule — small by default, hidden until the
/// frontend reports in, click-through on, no duplicates — so nothing here can
/// accidentally put an unverified window on screen.
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

    overlays::manager::create_overlays(app, &monitors)?;
    overlays::manager::cleanup_stale(app, &monitors);
    overlays::manager::hide_unmanaged(app);
    overlays::manager::spawn_watchdog(app);
    Ok(())
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    // First in the menu on purpose: when an overlay is misbehaving this is what
    // the user is hunting for, and it must not be buried.
    let panic_hide = MenuItem::with_id(
        app,
        "emergency-hide",
        "Hide all overlays now",
        true,
        None::<&str>,
    )?;
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

    let top_separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &panic_hide,
            &top_separator,
            &show,
            &toggle_overlay,
            &rescue,
            &separator,
            &quit,
        ],
    )?;

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
            "emergency-hide" => run_action(app, Action::EmergencyHide),
            "show" => run_action(app, Action::OpenSettings),
            "overlay" => {
                let next = {
                    let state = app.state::<AppState>();
                    let next = !state.settings.lock().overlay_visible;
                    state.settings.lock().overlay_visible = next;
                    let _ = state.persist();
                    next
                };
                let result = if next {
                    overlays::manager::show_all(app)
                } else {
                    for label in app.state::<AppState>().overlays.labels() {
                        let _ = overlays::manager::hide(app, &label);
                    }
                    Ok(())
                };
                if let Err(e) = result {
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

/// Start WASAPI loopback capture and forward analysed frames to the webviews.
///
/// Failure is not fatal: everything else keeps working and the diagnostics panel
/// reports why the visualisers are flat, rather than leaving them mysteriously dead.
#[cfg(windows)]
fn start_audio_engine(app: &tauri::AppHandle) {
    use tauri::Emitter;

    let (band_count, device_id, clock) = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock();
        (
            settings.audio_band_count,
            settings.audio_device_id.clone(),
            std::sync::Arc::clone(&state.clock),
        )
    };

    let emit_handle = app.clone();
    let engine = audio::wasapi::AudioEngine::start(
        band_count,
        move || clock.now_ms(),
        move |frame| {
            // Targeted at the overlays rather than broadcast.
            //
            // `emit` goes to every webview, so the settings window was also being
            // handed ~50 frames a second it has no use for — a third more IPC
            // traffic and a third more JSON serialisation, all of it marshalled
            // through the event loop that also pumps window messages. Starving
            // that loop is a plausible contributor to an overlay going "Not
            // Responding", so the frames now go only where they are drawn.
            for (label, _) in emit_handle.webview_windows() {
                if label.starts_with("overlay-") || label == "island" {
                    if let Err(e) = emit_handle.emit_to(&label, "liquid://audio", &frame) {
                        tracing::debug!(error = %e, %label, "could not emit audio frame");
                    }
                }
            }
        },
    );
    if let Some(id) = device_id {
        engine.set_device(Some(id));
    }

    let state = app.state::<AppState>();
    *state.audio.lock() = Some(engine);
    tracing::info!(band_count, "audio engine started");
}

#[cfg(not(windows))]
fn start_audio_engine(_app: &tauri::AppHandle) {
    tracing::warn!("audio capture is only implemented for Windows");
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
            commands::lyrics_lookup,
            commands::lyrics_accept,
            commands::lyrics_accept_absent,
            commands::lyrics_import_file,
            commands::lyrics_save_edited,
            commands::lyrics_forget,
            commands::get_lyric_offset,
            commands::set_lyric_offset,
            commands::get_capture_status,
            commands::list_audio_devices,
            commands::set_audio_device,
            commands::emergency_hide_overlays,
            commands::get_overlay_states,
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
            start_audio_engine(&handle);

            // Deliberately does NOT show the overlays. Each one becomes visible
            // only when its own frontend signals readiness, via the manager. The
            // previous code showed them here unconditionally, which is how a page
            // that never loaded became a fullscreen white blocker.
            if want_click_through {
                tracing::info!("click-through was enabled previously; it stays on");
            }
            let _ = overlay_visible;

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
