//! Overlay lifecycle management.
//!
//! This module exists because of a serious defect in the first implementation: each
//! overlay was created at the **full work-area size of its monitor**, made topmost,
//! and shown unconditionally without waiting for its page to load. When the webview
//! failed to paint — a hang, a crash, an error page — the result was a fullscreen
//! opaque white window pinned above everything, swallowing every click, with the
//! title "LIQUID Overlay (Not Responding)". The desktop became unusable.
//!
//! Three compounding mistakes produced that, and all three are fixed here:
//!
//! 1. **Fullscreen by default.** A HUD does not need to be the size of the screen,
//!    and a window that large has no business existing until it is known to work.
//!    Overlays now start at [`DEFAULT_WIDTH`]x[`DEFAULT_HEIGHT`] *logical* pixels.
//! 2. **Shown before ready.** Visibility now requires the frontend to have signalled
//!    readiness. A watchdog fails and hides any overlay that never does.
//! 3. **Click-through off by default.** An unstyled or blank page has normal pointer
//!    behaviour and captures everything. Click-through is now the default state, so
//!    even a completely broken overlay cannot trap the mouse.
//!
//! The guiding rule: **a failed overlay must be invisible and intangible, never a
//! barrier.**

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub mod manager;

/// Default overlay size in **logical** pixels, scaled per monitor at creation.
/// Deliberately small: enough for a now-playing capsule, nowhere near a blocker.
pub const DEFAULT_WIDTH: f64 = 420.0;
pub const DEFAULT_HEIGHT: f64 = 160.0;

/// Distance from the top of the monitor's work area.
pub const DEFAULT_TOP_MARGIN: f64 = 24.0;

/// How long an overlay may stay in `Loading` before it is declared failed.
///
/// Generous enough for a cold start with a dev server still warming up, short
/// enough that a broken overlay does not linger. It never *blocks* anything while
/// loading — it is hidden — so this is about reporting, not safety.
pub const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Explicit lifecycle. The invariant that matters: a window is only ever shown in
/// `Visible`, and reaching `Visible` requires passing through `Ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayState {
    /// The window exists but the webview has not reported in.
    Created,
    /// Navigation started; still waiting for the frontend to signal readiness.
    Loading,
    /// The frontend signalled readiness. Safe to show.
    Ready,
    /// Shown on screen.
    Visible,
    /// Deliberately hidden by the user or by an emergency hide.
    Hidden,
    /// Never became ready, or errored. Hidden and left that way.
    Failed,
    Destroyed,
}

impl OverlayState {
    /// Whether it is safe to put this window on screen.
    ///
    /// `Created` and `Loading` are excluded on purpose: that is precisely the
    /// window in which a blank or hung page would otherwise be displayed.
    pub fn may_show(self) -> bool {
        matches!(self, Self::Ready | Self::Visible | Self::Hidden)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Destroyed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayInfo {
    pub label: String,
    pub monitor_id: String,
    pub state: OverlayState,
    /// Milliseconds spent waiting for readiness, for diagnostics.
    pub age_ms: u64,
    pub click_through: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
struct Entry {
    monitor_id: String,
    state: OverlayState,
    created_at: Instant,
    click_through: bool,
    error: Option<String>,
    /// Where the overlay belongs once it is known to work.
    ///
    /// Windows are created **off-screen** rather than hidden: WebView2 does not
    /// navigate a hidden window, so `visible(false)` deadlocks — the window waits
    /// to be shown until it is ready, and can never become ready because it was
    /// never shown. Off-screen gets it loading while keeping it incapable of
    /// covering anything.
    target: (f64, f64),
}

/// Registry of overlay windows and their states.
#[derive(Default)]
pub struct OverlayRegistry {
    entries: Mutex<HashMap<String, Entry>>,
    /// When set, every overlay stays hidden regardless of other requests. Set by
    /// the emergency hide so nothing can put a window back on screen behind the
    /// user's back.
    emergency_hidden: Mutex<bool>,
}

impl OverlayRegistry {
    /// Register a newly created window. Returns false when the label already
    /// exists, which is how duplicate overlays are prevented.
    pub fn register(
        &self,
        label: &str,
        monitor_id: &str,
        click_through: bool,
        target: (f64, f64),
    ) -> bool {
        let mut entries = self.entries.lock();
        if entries.contains_key(label) {
            return false;
        }
        entries.insert(
            label.to_string(),
            Entry {
                monitor_id: monitor_id.to_string(),
                state: OverlayState::Created,
                created_at: Instant::now(),
                click_through,
                error: None,
                target,
            },
        );
        true
    }

    /// Where this overlay should sit once it is ready.
    pub fn target(&self, label: &str) -> Option<(f64, f64)> {
        self.entries.lock().get(label).map(|e| e.target)
    }

    pub fn contains(&self, label: &str) -> bool {
        self.entries.lock().contains_key(label)
    }

    pub fn set_state(&self, label: &str, state: OverlayState) {
        if let Some(entry) = self.entries.lock().get_mut(label) {
            entry.state = state;
        }
    }

    pub fn state(&self, label: &str) -> Option<OverlayState> {
        self.entries.lock().get(label).map(|e| e.state)
    }

    pub fn fail(&self, label: &str, reason: impl Into<String>) {
        if let Some(entry) = self.entries.lock().get_mut(label) {
            entry.state = OverlayState::Failed;
            entry.error = Some(reason.into());
        }
    }

    pub fn set_click_through(&self, label: &str, enabled: bool) {
        if let Some(entry) = self.entries.lock().get_mut(label) {
            entry.click_through = enabled;
        }
    }

    pub fn remove(&self, label: &str) {
        self.entries.lock().remove(label);
    }

    pub fn labels(&self) -> Vec<String> {
        self.entries.lock().keys().cloned().collect()
    }

    /// Labels that have been waiting longer than `READY_TIMEOUT` without becoming
    /// ready. These are the ones the watchdog fails and hides.
    pub fn timed_out(&self) -> Vec<String> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|(_, e)| {
                matches!(e.state, OverlayState::Created | OverlayState::Loading)
                    && e.created_at.elapsed() > READY_TIMEOUT
            })
            .map(|(label, _)| label.clone())
            .collect()
    }

    pub fn emergency_hidden(&self) -> bool {
        *self.emergency_hidden.lock()
    }

    pub fn set_emergency_hidden(&self, hidden: bool) {
        *self.emergency_hidden.lock() = hidden;
    }

    pub fn info(&self) -> Vec<OverlayInfo> {
        let entries = self.entries.lock();
        let mut out: Vec<OverlayInfo> = entries
            .iter()
            .map(|(label, e)| OverlayInfo {
                label: label.clone(),
                monitor_id: e.monitor_id.clone(),
                state: e.state,
                age_ms: e.created_at.elapsed().as_millis() as u64,
                click_through: e.click_through,
                error: e.error.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loading_overlay_must_never_be_shown() {
        // This is the whole point of the module: the blank-white-blocker window
        // existed because a window could be shown while its page was still loading.
        assert!(!OverlayState::Created.may_show());
        assert!(!OverlayState::Loading.may_show());
        assert!(!OverlayState::Failed.may_show());
        assert!(!OverlayState::Destroyed.may_show());
    }

    #[test]
    fn ready_and_hidden_overlays_may_be_shown() {
        assert!(OverlayState::Ready.may_show());
        assert!(OverlayState::Visible.may_show());
        assert!(OverlayState::Hidden.may_show());
    }

    #[test]
    fn the_default_size_is_nowhere_near_a_blocker() {
        // A HUD, not a screen cover. Guards against a regression to full work area.
        const { assert!(DEFAULT_WIDTH <= 600.0, "overlay default is too wide") };
        const { assert!(DEFAULT_HEIGHT <= 300.0, "overlay default is too tall") };
    }

    #[test]
    fn registering_the_same_label_twice_is_refused() {
        let r = OverlayRegistry::default();
        assert!(r.register("overlay-0", "DISPLAY1", true, (0.0, 0.0)));
        assert!(
            !r.register("overlay-0", "DISPLAY1", true, (0.0, 0.0)),
            "duplicates must be refused"
        );
        assert_eq!(r.labels().len(), 1);
    }

    #[test]
    fn a_new_overlay_starts_in_created_not_visible() {
        let r = OverlayRegistry::default();
        r.register("overlay-0", "DISPLAY1", true, (0.0, 0.0));
        assert_eq!(r.state("overlay-0"), Some(OverlayState::Created));
        assert!(!r.state("overlay-0").unwrap().may_show());
    }

    #[test]
    fn failing_records_a_reason() {
        let r = OverlayRegistry::default();
        r.register("overlay-0", "DISPLAY1", true, (0.0, 0.0));
        r.fail("overlay-0", "did not become ready");
        let info = &r.info()[0];
        assert_eq!(info.state, OverlayState::Failed);
        assert_eq!(info.error.as_deref(), Some("did not become ready"));
    }

    #[test]
    fn the_watchdog_only_picks_overlays_that_never_became_ready() {
        let r = OverlayRegistry::default();
        r.register("stuck", "DISPLAY1", true, (0.0, 0.0));
        r.register("fine", "DISPLAY1", true, (0.0, 0.0));
        r.set_state("fine", OverlayState::Ready);

        // Nothing has aged out yet.
        assert!(r.timed_out().is_empty());

        // Backdate the stuck one past the timeout.
        {
            let mut entries = r.entries.lock();
            let e = entries.get_mut("stuck").unwrap();
            e.created_at = Instant::now() - READY_TIMEOUT - Duration::from_secs(1);
            let f = entries.get_mut("fine").unwrap();
            f.created_at = Instant::now() - READY_TIMEOUT - Duration::from_secs(1);
        }

        let timed_out = r.timed_out();
        assert_eq!(
            timed_out,
            vec!["stuck".to_string()],
            "a ready overlay must not be failed"
        );
    }

    #[test]
    fn a_failed_overlay_can_be_revived_by_a_fresh_readiness_signal() {
        // The watchdog writing an overlay off must not be permanent: a webview
        // that reloads and reports in again is healthy and should come back.
        let r = OverlayRegistry::default();
        r.register("overlay-0", "DISPLAY1", true, (10.0, 20.0));
        r.fail("overlay-0", "did not become ready");
        assert_eq!(r.state("overlay-0"), Some(OverlayState::Failed));

        r.set_state("overlay-0", OverlayState::Ready);
        assert!(r.state("overlay-0").unwrap().may_show());
    }

    #[test]
    fn the_target_position_survives_a_failure() {
        let r = OverlayRegistry::default();
        r.register("overlay-0", "DISPLAY1", true, (750.0, 24.0));
        r.fail("overlay-0", "nope");
        // Recovery has to put the window back where it belonged.
        assert_eq!(r.target("overlay-0"), Some((750.0, 24.0)));
    }

    #[test]
    fn emergency_hide_is_sticky_until_cleared() {
        let r = OverlayRegistry::default();
        assert!(!r.emergency_hidden());
        r.set_emergency_hidden(true);
        assert!(r.emergency_hidden());
        r.set_emergency_hidden(false);
        assert!(!r.emergency_hidden());
    }

    #[test]
    fn removing_an_overlay_frees_its_label_for_reuse() {
        let r = OverlayRegistry::default();
        r.register("overlay-0", "DISPLAY1", true, (0.0, 0.0));
        r.remove("overlay-0");
        assert!(!r.contains("overlay-0"));
        assert!(
            r.register("overlay-0", "DISPLAY1", true, (0.0, 0.0)),
            "stale label must be reusable"
        );
    }

    #[test]
    fn info_is_stable_in_label_order() {
        let r = OverlayRegistry::default();
        r.register("overlay-2", "C", true, (0.0, 0.0));
        r.register("overlay-0", "A", true, (0.0, 0.0));
        r.register("overlay-1", "B", true, (0.0, 0.0));
        let labels: Vec<String> = r.info().into_iter().map(|i| i.label).collect();
        assert_eq!(labels, vec!["overlay-0", "overlay-1", "overlay-2"]);
    }
}
