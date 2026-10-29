//! Tauri commands: the typed IPC surface between the webviews and the native layer.
//!
//! Every command returns `Result<_, String>` so a failure surfaces in the UI as a
//! message the user can act on, rather than a silently rejected promise.

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::media::{PlaybackSnapshot, SessionSummary, TransportAction};
use crate::platform::{self, MonitorInfo, OverlayObstruction};
use crate::state::AppState;
use crate::storage::Settings;

// ---------------------------------------------------------------------------
// Media sessions
// ---------------------------------------------------------------------------

/// What the media subsystem is currently doing, in terms the UI can show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStatus {
    /// False when the session service could not be reached at all.
    pub bridge_running: bool,
    pub error: Option<String>,
    pub snapshot: Option<PlaybackSnapshot>,
    pub sessions: Vec<SessionSummary>,
    pub pinned: Option<String>,
}

#[tauri::command]
pub fn get_media_status(state: State<'_, AppState>) -> MediaStatus {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        match guard.as_ref() {
            Some(bridge) => MediaStatus {
                bridge_running: true,
                error: bridge.last_error(),
                snapshot: bridge.snapshot(),
                sessions: bridge.sessions(),
                pinned: bridge.pinned(),
            },
            None => MediaStatus {
                bridge_running: false,
                error: Some(
                    "The Windows media session service could not be reached. Track information \
                     is unavailable."
                        .into(),
                ),
                snapshot: None,
                sessions: Vec::new(),
                pinned: None,
            },
        }
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        MediaStatus {
            bridge_running: false,
            error: Some("Media sessions are only implemented for Windows.".into()),
            snapshot: None,
            sessions: Vec::new(),
            pinned: None,
        }
    }
}

/// Pin a source so arbitration always follows it, or pass `null` to clear.
#[tauri::command]
pub fn set_pinned_source(
    state: State<'_, AppState>,
    source_app: Option<String>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        let bridge = guard
            .as_ref()
            .ok_or_else(|| "media sessions are unavailable".to_string())?;
        bridge.set_pinned(source_app);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (state, source_app);
        Err("media sessions are only implemented for Windows".into())
    }
}

/// Send a transport command to the active source.
///
/// Fails with an explanation when the source has not advertised the capability,
/// rather than issuing a call that silently does nothing.
#[tauri::command]
pub fn media_transport(state: State<'_, AppState>, action: TransportAction) -> Result<(), String> {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        let bridge = guard
            .as_ref()
            .ok_or_else(|| "media sessions are unavailable".to_string())?;
        bridge.transport(action).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (state, action);
        Err("media sessions are only implemented for Windows".into())
    }
}

#[tauri::command]
pub fn media_seek(state: State<'_, AppState>, position_ms: u64) -> Result<(), String> {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        let bridge = guard
            .as_ref()
            .ok_or_else(|| "media sessions are unavailable".to_string())?;
        bridge.seek(position_ms).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (state, position_ms);
        Err("media sessions are only implemented for Windows".into())
    }
}

// ---------------------------------------------------------------------------
// Audio capture
// ---------------------------------------------------------------------------

use crate::audio::{AudioDevice, CaptureState, CaptureStatus};

#[tauri::command]
pub fn get_capture_status(state: State<'_, AppState>) -> CaptureStatus {
    #[cfg(windows)]
    {
        let guard = state.audio.lock();
        match guard.as_ref() {
            Some(engine) => engine.status(),
            None => CaptureStatus {
                state: CaptureState::Stopped,
                error: Some("Audio capture is not running.".into()),
                ..Default::default()
            },
        }
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        CaptureStatus {
            state: CaptureState::Failed,
            error: Some("Audio capture is only implemented for Windows.".into()),
            ..Default::default()
        }
    }
}

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<AudioDevice>, String> {
    #[cfg(windows)]
    {
        crate::audio::wasapi::list_devices().map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Err("audio devices are only enumerable on Windows".into())
    }
}

/// Switch capture endpoint. `null` follows the system default.
#[tauri::command]
pub fn set_audio_device(
    state: State<'_, AppState>,
    device_id: Option<String>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let guard = state.audio.lock();
        let engine = guard
            .as_ref()
            .ok_or_else(|| "audio capture is not running".to_string())?;
        engine.set_device(device_id.clone());
        drop(guard);
        state.settings.lock().audio_device_id = device_id;
        state.persist()
    }
    #[cfg(not(windows))]
    {
        let _ = (state, device_id);
        Err("audio capture is only implemented for Windows".into())
    }
}

// ---------------------------------------------------------------------------
// Lyrics
// ---------------------------------------------------------------------------

use crate::lyrics::{
    local as lyrics_local, service as lyrics_service, LyricsCandidate, LyricsOrigin, LyricsQuery,
    ResolvedLyrics,
};

/// What LIQUID has for a track: a cached answer, or candidates to score.
///
/// The network call runs on a blocking worker, never on the thread serving the
/// webview — a 12-second HTTP timeout on the UI thread would freeze the overlay.
#[tauri::command]
pub async fn lyrics_lookup(
    state: State<'_, AppState>,
    query: LyricsQuery,
    offset_key: String,
) -> Result<lyrics_service::LookupResult, String> {
    tracing::debug!(title = %query.title, %offset_key, "lyrics: lookup requested");

    // 1. Cache first. Cheap, and it is what keeps a free rate-limited service from
    //    being asked again for every play of the same track.
    {
        let store = state.store.lock();
        match lyrics_service::lookup_cached(store.connection(), &offset_key) {
            Ok(Some(hit)) => return Ok(hit),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "lyrics cache read failed"),
        }
    }

    // 2. Respect the user's choice before reaching the network at all.
    if !state.settings.lock().lyrics_online_lookup {
        return Ok(lyrics_service::LookupResult {
            resolved: None,
            known_absent: false,
            candidates: Vec::new(),
            provider_error: Some("Online lyrics lookup is turned off in Settings → Lyrics.".into()),
        });
    }

    tracing::info!(title = %query.title, artist = ?query.artist, "lyrics: asking provider");

    let client = std::sync::Arc::clone(&state.lyrics_client);
    let candidates = tauri::async_runtime::spawn_blocking(move || {
        lyrics_service::fetch_candidates(&client, &query)
    })
    .await
    .map_err(|e| format!("lyrics worker failed: {e}"))?;

    match candidates {
        Ok(candidates) => {
            tracing::info!(count = candidates.len(), "lyrics: provider replied");
            Ok(lyrics_service::LookupResult {
                resolved: None,
                known_absent: false,
                candidates,
                provider_error: None,
            })
        }
        // A provider failure is reported as itself. Showing "no lyrics" for a
        // network error would be a lie the user cannot act on.
        Err(e) => {
            tracing::warn!(error = %e, "lyrics: provider failed");
            Ok(lyrics_service::LookupResult {
                resolved: None,
                known_absent: false,
                candidates: Vec::new(),
                provider_error: Some(e.to_string()),
            })
        }
    }
}

/// Store the candidate the frontend's matcher chose.
#[tauri::command]
pub fn lyrics_accept(
    state: State<'_, AppState>,
    offset_key: String,
    candidate: LyricsCandidate,
) -> Result<ResolvedLyrics, String> {
    let store = state.store.lock();
    lyrics_service::accept(store.connection(), &offset_key, &candidate).map_err(|e| e.to_string())
}

/// Record that this track genuinely has no lyrics.
#[tauri::command]
pub fn lyrics_accept_absent(state: State<'_, AppState>, offset_key: String) -> Result<(), String> {
    let store = state.store.lock();
    lyrics_service::accept_absent(store.connection(), &offset_key).map_err(|e| e.to_string())
}

/// Import a `.lrc` the user picked. Local files win over anything fetched.
#[tauri::command]
pub fn lyrics_import_file(
    state: State<'_, AppState>,
    offset_key: String,
    path: String,
) -> Result<ResolvedLyrics, String> {
    let loaded = lyrics_local::read_file(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    let store = state.store.lock();
    lyrics_service::accept_local(store.connection(), &offset_key, loaded).map_err(|e| e.to_string())
}

/// Save text from the editor as this track's lyrics.
#[tauri::command]
pub fn lyrics_save_edited(
    state: State<'_, AppState>,
    offset_key: String,
    text: String,
) -> Result<ResolvedLyrics, String> {
    let edited = ResolvedLyrics {
        text,
        origin: LyricsOrigin::UserEdited,
        instrumental: false,
        provider_id: None,
        attribution: None,
    };
    let store = state.store.lock();
    lyrics_service::accept_local(store.connection(), &offset_key, edited).map_err(|e| e.to_string())
}

/// Drop whatever is cached for a track, so the next lookup starts fresh.
#[tauri::command]
pub fn lyrics_forget(state: State<'_, AppState>, offset_key: String) -> Result<(), String> {
    let store = state.store.lock();
    crate::lyrics::cache::forget(store.connection(), &offset_key).map_err(|e| e.to_string())
}

/// Per-track lyric offset in ms. It is added to the playback position, so a
/// **positive** value shows lyrics earlier and a negative one shows them later.
#[tauri::command]
pub fn get_lyric_offset(state: State<'_, AppState>, offset_key: String) -> i64 {
    state.store.lock().lyrics_offset(&offset_key).unwrap_or(0)
}

#[tauri::command]
pub fn set_lyric_offset(
    state: State<'_, AppState>,
    offset_key: String,
    offset_ms: i64,
) -> Result<(), String> {
    // Bound it: an offset larger than any plausible track is a mistake, and it would
    // park every lyric off the end of the timeline with no obvious way back.
    let clamped = offset_ms.clamp(-30_000, 30_000);
    state
        .store
        .lock()
        .set_lyrics_offset(&offset_key, clamped)
        .map_err(|e| format!("could not save the offset: {e}"))
}

/// Force an immediate re-read of the media sessions.
///
/// The bridge is event-driven with a slow tick, so a source that publishes a session
/// without raising an event can take a second to appear. This is the "nothing showed
/// up, try again" button rather than making the tick faster for everyone.
#[tauri::command]
pub fn refresh_media_sessions(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        let bridge = guard
            .as_ref()
            .ok_or_else(|| "media sessions are unavailable".to_string())?;
        bridge.refresh();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err("media sessions are only implemented for Windows".into())
    }
}

/// Album art as a data URL, or `None` when this track has none.
#[tauri::command]
pub fn get_artwork(state: State<'_, AppState>, key: String) -> Option<String> {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        guard
            .as_ref()
            .and_then(|b| b.artwork(&key))
            .map(|a| a.to_data_url())
    }
    #[cfg(not(windows))]
    {
        let _ = (state, key);
        None
    }
}

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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub app_version: String,
    pub monotonic_clock_degraded: bool,
    pub monotonic_now_ms: f64,
    pub monitor_count: usize,
    pub launches_at_login: bool,
    /// Phase 4 subsystem. Reported as unavailable rather than faked.
    pub audio_capture: &'static str,
    /// Live description of the media bridge, not a hard-coded string.
    pub media_session: String,
    pub artwork_cached: bool,
    /// Entries held by the bounded artwork cache, so growth is visible.
    pub artwork_cache_entries: usize,
    /// Rows in the lyrics cache. Surfaced because this one is deliberately
    /// unbounded in size (it is small text, on disk, and the whole point is to stop
    /// re-querying a free service) — so it should at least be observable.
    pub lyrics_cache_entries: i64,
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
        media_session: describe_media(&state),
        artwork_cached: {
            #[cfg(windows)]
            {
                let guard = state.media.lock();
                guard
                    .as_ref()
                    .and_then(|b| b.snapshot())
                    .and_then(|s| s.artwork_key)
                    .map(|k| {
                        let g = state.media.lock();
                        g.as_ref().and_then(|b| b.artwork(&k)).is_some()
                    })
                    .unwrap_or(false)
            }
            #[cfg(not(windows))]
            false
        },
        artwork_cache_entries: {
            #[cfg(windows)]
            {
                let guard = state.media.lock();
                guard.as_ref().map(|b| b.artwork_entries()).unwrap_or(0)
            }
            #[cfg(not(windows))]
            0
        },
        lyrics_cache_entries: crate::lyrics::cache::entry_count(state.store.lock().connection()),
    }
}

/// One line describing the media bridge's real state.
fn describe_media(state: &State<'_, AppState>) -> String {
    #[cfg(windows)]
    {
        let guard = state.media.lock();
        match guard.as_ref() {
            None => "unavailable — the session service could not be reached".to_string(),
            Some(bridge) => {
                if let Some(e) = bridge.last_error() {
                    return format!("error: {e}");
                }
                let sessions = bridge.sessions().len();
                match bridge.snapshot() {
                    Some(s) => format!(
                        "connected — following {} ({} session{})",
                        s.source_app.as_deref().unwrap_or("unknown source"),
                        sessions,
                        if sessions == 1 { "" } else { "s" }
                    ),
                    None => format!(
                        "connected — no track playing ({} session{})",
                        sessions,
                        if sessions == 1 { "" } else { "s" }
                    ),
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        "only implemented for Windows".to_string()
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

/// Called by each webview once React has mounted.
///
/// Exists because window creation succeeding tells you nothing about whether the
/// *page* loaded. Phase 1 was signed off on externally-probed window styles while
/// every window was in fact showing WebView2's `ERR_CONNECTION_REFUSED` page — the
/// native side had no idea. This log line is the evidence that the frontend is
/// really running, and it is the first thing to check when a window looks blank.
#[tauri::command]
pub fn report_frontend_ready(window: WebviewWindow, route: String, href: String) {
    tracing::info!(
        label = %window.label(),
        %route,
        %href,
        "frontend ready"
    );
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
