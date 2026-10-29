//! Platform facade.
//!
//! Every Win32 call in LIQUID lives below this module. Application logic depends on
//! the types and functions re-exported here, never on the `windows` crate directly,
//! so the platform-specific surface stays small and reviewable.
//!
//! (The directory is named `platform` rather than `windows` because a crate-root
//! module called `windows` is ambiguous with the `windows` crate in `use`
//! declarations.)

use serde::{Deserialize, Serialize};

pub mod monitors;
pub mod overlay;
pub mod startup;

/// A monitor as LIQUID sees it: identified, positioned, and with its own scaling.
///
/// Nothing here assumes a primary monitor at the origin or a 1920x1080 layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorInfo {
    /// Stable-ish device name, e.g. `\\.\DISPLAY1`. Used to remember placement.
    pub id: String,
    /// Full monitor rectangle in virtual-desktop coordinates. May be negative.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Work area, i.e. excluding the taskbar. Overlays snap to this.
    pub work_x: i32,
    pub work_y: i32,
    pub work_width: i32,
    pub work_height: i32,
    pub is_primary: bool,
    /// Effective DPI scale, 1.0 at 96 DPI. Per-monitor, not per-system.
    pub scale_factor: f64,
}

/// Why the overlay may not be able to sit above what is on screen.
///
/// Reported to the UI so the user gets an explanation instead of a window that
/// mysteriously does not appear. LIQUID does not attempt to defeat any of these.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayObstruction {
    /// Nothing in the way.
    None,
    /// A window is covering the whole monitor with no border — probably exclusive
    /// fullscreen. Remedy: ask the user to switch to borderless windowed.
    SuspectedExclusiveFullscreen,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("window handle unavailable: {0}")]
    NoHandle(String),
    #[error("win32 call failed: {0}")]
    Win32(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;
