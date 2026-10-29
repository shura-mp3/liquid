//! Shared application state.
//!
//! One owner for the handles every command needs. Locks are held for the shortest
//! possible span: these are touched from command handlers on the main thread, and a
//! long hold would stall the window message pump.

use std::path::PathBuf;
use std::time::Instant;

use parking_lot::Mutex;

use std::sync::Arc;

use crate::storage::{Settings, SettingsStore};
use crate::timing::MonotonicClock;

#[cfg(windows)]
use crate::media::gsmtc::GsmtcBridge;

/// Minimum gap between click-through transitions.
///
/// Upstream `tauri-apps/tauri#15947` reports transparent regions going black on
/// Windows 10 when cursor-event state is toggled rapidly. Debouncing is cheap
/// insurance, and nothing legitimate needs to toggle click-through faster than this.
pub const CLICK_THROUGH_DEBOUNCE_MS: u128 = 120;

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub store: Mutex<SettingsStore>,
    /// Shared with the media bridge thread so every playback observation is stamped
    /// on the same monotonic base the UI interpolates against.
    pub clock: Arc<MonotonicClock>,
    /// `None` when the media session service could not be reached at all. The
    /// difference between "no bridge" and "bridge with no session" is reported to
    /// the user rather than both showing as an empty HUD.
    #[cfg(windows)]
    pub media: Mutex<Option<GsmtcBridge>>,
    last_click_through_change: Mutex<Option<Instant>>,
    /// Root for the database, logs and the lyrics cache. Held here so Phase 3's
    /// lyrics cache and Phase 7's history export resolve paths from one place rather
    /// than each re-deriving them.
    #[allow(dead_code)]
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(store: SettingsStore, data_dir: PathBuf) -> Self {
        let settings = store.load();
        Self {
            settings: Mutex::new(settings),
            store: Mutex::new(store),
            clock: Arc::new(MonotonicClock::new()),
            #[cfg(windows)]
            media: Mutex::new(None),
            last_click_through_change: Mutex::new(None),
            data_dir,
        }
    }

    /// Returns true when enough time has passed to allow another click-through
    /// transition. Records the attempt when it allows one.
    pub fn may_toggle_click_through(&self) -> bool {
        let mut last = self.last_click_through_change.lock();
        let now = Instant::now();
        match *last {
            Some(previous)
                if now.duration_since(previous).as_millis() < CLICK_THROUGH_DEBOUNCE_MS =>
            {
                false
            }
            _ => {
                *last = Some(now);
                true
            }
        }
    }

    /// Persist the current in-memory settings.
    pub fn persist(&self) -> Result<(), String> {
        let settings = self.settings.lock().clone();
        self.store
            .lock()
            .save(&settings)
            .map_err(|e| format!("could not save settings: {e}"))
    }
}
