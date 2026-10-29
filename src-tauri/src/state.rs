//! Shared application state.
//!
//! One owner for the handles every command needs. Locks are held for the shortest
//! possible span: these are touched from command handlers on the main thread, and a
//! long hold would stall the window message pump.

use std::path::PathBuf;
use std::time::Instant;

use parking_lot::Mutex;

use crate::storage::{Settings, SettingsStore};
use crate::timing::MonotonicClock;

/// Minimum gap between click-through transitions.
///
/// Upstream `tauri-apps/tauri#15947` reports transparent regions going black on
/// Windows 10 when cursor-event state is toggled rapidly. Debouncing is cheap
/// insurance, and nothing legitimate needs to toggle click-through faster than this.
pub const CLICK_THROUGH_DEBOUNCE_MS: u128 = 120;

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub store: Mutex<SettingsStore>,
    pub clock: MonotonicClock,
    last_click_through_change: Mutex<Option<Instant>>,
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(store: SettingsStore, data_dir: PathBuf) -> Self {
        let settings = store.load();
        Self {
            settings: Mutex::new(settings),
            store: Mutex::new(store),
            clock: MonotonicClock::new(),
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
            Some(previous) if now.duration_since(previous).as_millis() < CLICK_THROUGH_DEBOUNCE_MS => {
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
