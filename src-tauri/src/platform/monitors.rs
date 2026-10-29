//! Display enumeration with per-monitor DPI.
//!
//! Mixed-DPI setups are the normal case, not an edge case, so scale factor is read
//! per monitor via `GetDpiForMonitor` rather than taken from a system-wide value.
//! Nothing here assumes the primary monitor is at the origin or leftmost — monitor
//! coordinates are routinely negative.

#![cfg(windows)]

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use super::{MonitorInfo, Result};

/// Windows reports DPI as an integer; 96 is the 1.0 baseline.
const BASELINE_DPI: f64 = 96.0;

/// `MONITORINFOF_PRIMARY` from `winuser.h`. The `windows` crate no longer exports
/// this constant, so it is declared here with its documented value.
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// Continue enumeration. `BOOL::from(true)` rather than a re-exported `TRUE`, which
/// also moved between `windows` crate versions.
const CONTINUE_ENUM: BOOL = BOOL(1);

unsafe extern "system" fn enum_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let out = &mut *(data.0 as *mut Vec<MonitorInfo>);

    let mut info = MONITORINFOEXW {
        monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    if !GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _).as_bool() {
        // Skip this monitor rather than aborting the whole enumeration: losing one
        // display is recoverable, losing the list is not.
        return CONTINUE_ENUM;
    }

    let mut dpi_x: u32 = 96;
    let mut dpi_y: u32 = 96;
    // Falls back to the 96 default if the call fails, which keeps a sane 1.0 scale.
    let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);

    let name_end = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    let id = String::from_utf16_lossy(&info.szDevice[..name_end]);

    let m = info.monitorInfo.rcMonitor;
    let w = info.monitorInfo.rcWork;

    out.push(MonitorInfo {
        id,
        x: m.left,
        y: m.top,
        width: m.right - m.left,
        height: m.bottom - m.top,
        work_x: w.left,
        work_y: w.top,
        work_width: w.right - w.left,
        work_height: w.bottom - w.top,
        is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        scale_factor: f64::from(dpi_x) / BASELINE_DPI,
    });

    CONTINUE_ENUM
}

/// Enumerate every connected monitor.
pub fn enumerate() -> Result<Vec<MonitorInfo>> {
    let mut out: Vec<MonitorInfo> = Vec::new();
    unsafe {
        // The return value is false only if enumeration was interrupted; our callback
        // always continues, so an error here means something systemic. Either way the
        // partial list is still usable.
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut out as *mut _ as isize),
        );
    }
    Ok(out)
}

/// Clamp a rectangle so it stays reachable on at least one monitor.
///
/// Called when displays change: a widget parked on a monitor that has been
/// unplugged would otherwise sit at coordinates the user cannot reach, with no way
/// to drag it back.
pub fn rescue_offscreen(
    monitors: &[MonitorInfo],
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> (i32, i32) {
    if monitors.is_empty() {
        return (x, y);
    }

    // Enough of the widget visible to grab and drag it.
    const MIN_VISIBLE: i32 = 80;

    let visible_somewhere = monitors.iter().any(|m| {
        let ox = (x + width).min(m.x + m.width) - x.max(m.x);
        let oy = (y + height).min(m.y + m.height) - y.max(m.y);
        ox >= MIN_VISIBLE.min(width) && oy >= MIN_VISIBLE.min(height)
    });
    if visible_somewhere {
        return (x, y);
    }

    // Move it to the work area of the primary monitor, or the first one we have.
    let target = monitors
        .iter()
        .find(|m| m.is_primary)
        .unwrap_or(&monitors[0]);

    let nx = target
        .work_x
        .max((target.work_x + target.work_width - width).min(target.work_x + 40));
    let ny = target
        .work_y
        .max((target.work_y + target.work_height - height).min(target.work_y + 40));
    (nx, ny)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: &str, x: i32, y: i32, w: i32, h: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            id: id.into(),
            x,
            y,
            width: w,
            height: h,
            work_x: x,
            work_y: y,
            work_width: w,
            work_height: h,
            is_primary: primary,
            scale_factor: 1.0,
        }
    }

    #[test]
    fn leaves_a_visible_widget_alone() {
        let ms = vec![monitor(r"\\.\DISPLAY1", 0, 0, 1920, 1080, true)];
        assert_eq!(rescue_offscreen(&ms, 100, 100, 300, 200), (100, 100));
    }

    #[test]
    fn rescues_a_widget_from_an_unplugged_monitor() {
        // Widget was at x=2400 on a second monitor that is now gone.
        let ms = vec![monitor(r"\\.\DISPLAY1", 0, 0, 1920, 1080, true)];
        let (x, y) = rescue_offscreen(&ms, 2400, 300, 300, 200);
        assert!(x >= 0 && x < 1920, "x was {x}");
        assert!(y >= 0 && y < 1080, "y was {y}");
    }

    #[test]
    fn handles_monitors_at_negative_coordinates() {
        // A display to the left of the primary has negative coordinates.
        let ms = vec![
            monitor(r"\\.\DISPLAY1", 0, 0, 1920, 1080, true),
            monitor(r"\\.\DISPLAY2", -1920, 0, 1920, 1080, false),
        ];
        assert_eq!(rescue_offscreen(&ms, -1800, 100, 300, 200), (-1800, 100));
    }

    #[test]
    fn rescues_a_widget_that_is_only_marginally_visible() {
        let ms = vec![monitor(r"\\.\DISPLAY1", 0, 0, 1920, 1080, true)];
        // Only 10px of a 300px widget peeking in: not enough to grab.
        let (x, _) = rescue_offscreen(&ms, 1910, 100, 300, 200);
        assert!(x < 1910, "widget should have been moved, got x={x}");
    }

    #[test]
    fn does_nothing_when_no_monitors_are_reported() {
        assert_eq!(rescue_offscreen(&[], 50, 60, 300, 200), (50, 60));
    }
}
