//! Native overlay window behaviour.
//!
//! The extended-style combination here is deliberate, because combining these
//! blindly produces a window that cannot be closed or that steals focus from games:
//!
//! - `WS_EX_TOPMOST`    — sit above ordinary windows.
//! - `WS_EX_TOOLWINDOW` — keep the overlay out of the taskbar and Alt-Tab. The main
//!                        settings window deliberately does NOT get this.
//! - `WS_EX_NOACTIVATE` — never take foreground activation. Without this, clicking
//!                        the overlay pulls focus out of a fullscreen game and can
//!                        minimise it.
//! - `WS_EX_TRANSPARENT` — click-through. Applied and removed on demand, never left
//!                        on without a way back (see the recovery note below).
//!
//! **`WS_EX_LAYERED` is deliberately NOT set.** Tauri 2.12 creates a transparent
//! window with `WS_EX_NOREDIRECTIONBITMAP`, so per-pixel alpha comes from
//! DirectComposition rather than from a layered window's redirection surface. The
//! two are mutually exclusive — a layered window needs the redirection surface that
//! flag removes — so forcing `WS_EX_LAYERED` here risks breaking transparency
//! outright. It was verified empirically that the window works without it.
//!
//! **Styles must be re-applied after any tao window operation.** tao keeps its own
//! `WindowFlags` model and rewrites both `GWL_STYLE` and `GWL_EXSTYLE` wholesale
//! when it re-applies them, which happens on `show()` among other calls. Setting our
//! styles once at creation looked like it worked — the write verified — and was then
//! silently reverted by the first `show()`. Hence `apply_overlay_styles` is
//! idempotent and is called again after every visibility change.
//!
//! **Recovery.** `WS_EX_TRANSPARENT` makes the window unclickable, so enabling it
//! without an escape route would leave the user unable to turn it off. Click-through
//! is therefore always paired with a global hotkey and a tray menu item, both of
//! which live outside the overlay window and keep working.

#![cfg(windows)]

use windows::Win32::Foundation::{GetLastError, SetLastError, HWND, RECT, WIN32_ERROR};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos,
    GWL_EXSTYLE, GWL_STYLE, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_THICKFRAME,
};

use super::{OverlayObstruction, PlatformError, Result};

/// Wraps a raw window handle. Window handles are main-thread-only, so this type is
/// deliberately neither `Send` nor `Sync`.
#[derive(Debug, Clone, Copy)]
pub struct WindowHandle(HWND);

impl WindowHandle {
    /// # Safety
    /// `raw` must be a live HWND owned by this process.
    pub unsafe fn from_raw(raw: isize) -> Result<Self> {
        if raw == 0 {
            return Err(PlatformError::NoHandle("null hwnd".into()));
        }
        Ok(Self(HWND(raw as *mut core::ffi::c_void)))
    }

    pub fn raw(self) -> isize {
        self.0 .0 as isize
    }
}

fn ex_style(hwnd: HWND) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) }
}

/// Write the extended style and **verify it stuck**.
///
/// `SetWindowLongPtrW` returns the previous value, and 0 is a legitimate previous
/// value, so its return tells us nothing about success. Ignoring that was hiding a
/// real failure: the styles silently did not apply and the only way to notice was
/// probing the window from another process. So the value is read back and compared,
/// and `GetLastError` is captured via the documented clear-then-call pattern.
fn set_ex_style(hwnd: HWND, style: isize) -> Result<()> {
    unsafe {
        SetLastError(WIN32_ERROR(0));
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        let err = GetLastError();

        let applied = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if applied != style {
            return Err(PlatformError::Win32(format!(
                "extended style did not apply: wanted 0x{style:08X}, window has 0x{applied:08X}, \
                 last error {err:?}"
            )));
        }
    }
    Ok(())
}

/// Apply the overlay style set.
///
/// Idempotent and cheap, because it must be re-applied after every tao window
/// operation that rewrites the style word. Click-through is preserved rather than
/// cleared, so re-asserting styles cannot silently make the overlay interactive
/// again behind the user's back.
pub fn apply_overlay_styles(h: WindowHandle) -> Result<()> {
    let hwnd = h.0;
    let current = ex_style(hwnd);
    let wanted =
        current | WS_EX_TOPMOST.0 as isize | WS_EX_TOOLWINDOW.0 as isize | WS_EX_NOACTIVATE.0 as isize;
    if wanted != current {
        set_ex_style(hwnd, wanted)?;
    }
    assert_topmost(h)
}

/// True when every style the overlay depends on is currently present.
pub fn has_overlay_styles(h: WindowHandle) -> bool {
    let ex = ex_style(h.0);
    let required =
        WS_EX_TOPMOST.0 as isize | WS_EX_TOOLWINDOW.0 as isize | WS_EX_NOACTIVATE.0 as isize;
    ex & required == required
}

/// Re-assert topmost Z-order.
///
/// Other applications legitimately take the topmost slot (installers, some games,
/// screen recorders), which silently drops the overlay behind them. This is called
/// on foreground-change events rather than from a polling timer: polling every frame
/// to re-assert Z-order is exactly the kind of needless background work this project
/// is supposed to avoid.
pub fn assert_topmost(h: WindowHandle) -> Result<()> {
    unsafe {
        SetWindowPos(
            h.0,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .map_err(|e| PlatformError::Win32(format!("SetWindowPos topmost: {e}")))
    }
}

/// Drop out of the topmost band — used by the OBS capture surface, which should not
/// float above the user's scene editor.
pub fn clear_topmost(h: WindowHandle) -> Result<()> {
    unsafe {
        SetWindowPos(
            h.0,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .map_err(|e| PlatformError::Win32(format!("SetWindowPos notopmost: {e}")))
    }
}

/// Turn click-through on or off.
///
/// Callers must debounce this. Upstream `tauri-apps/tauri#15947` reports transparent
/// regions turning black on Windows 10 when cursor-event state is toggled rapidly,
/// so the command layer rate-limits it rather than exposing it per-frame.
pub fn set_click_through(h: WindowHandle, enabled: bool) -> Result<()> {
    let hwnd = h.0;
    let current = ex_style(hwnd);
    let next = if enabled {
        current | WS_EX_TRANSPARENT.0 as isize
    } else {
        current & !(WS_EX_TRANSPARENT.0 as isize)
    };
    if next == current {
        return Ok(());
    }
    set_ex_style(hwnd, next)
}

pub fn is_click_through(h: WindowHandle) -> bool {
    ex_style(h.0) & WS_EX_TRANSPARENT.0 as isize != 0
}

/// Whether the overlay currently holds the topmost extended style.
pub fn is_topmost(h: WindowHandle) -> bool {
    ex_style(h.0) & WS_EX_TOPMOST.0 as isize != 0
}

/// Best-effort guess at whether something is covering the monitor in a way the
/// overlay cannot sit above.
///
/// This is a heuristic and is reported as "suspected", not asserted: a borderless
/// window that happens to fill the screen is indistinguishable from exclusive
/// fullscreen by geometry alone. The remedy offered to the user is a recommendation,
/// never an attempt to force visibility.
pub fn detect_obstruction(h: WindowHandle) -> OverlayObstruction {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == h.0 {
            return OverlayObstruction::None;
        }

        let mut rect = RECT::default();
        if GetWindowRect(fg, &mut rect).is_err() {
            return OverlayObstruction::None;
        }

        let monitor: HMONITOR = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return OverlayObstruction::None;
        }

        let covers_monitor = rect.left <= info.rcMonitor.left
            && rect.top <= info.rcMonitor.top
            && rect.right >= info.rcMonitor.right
            && rect.bottom >= info.rcMonitor.bottom;

        // A window with no caption and no resize frame that covers the entire
        // monitor is the usual shape of a fullscreen game.
        let style = GetWindowLongPtrW(fg, GWL_STYLE);
        let borderless =
            style & WS_CAPTION.0 as isize == 0 && style & WS_THICKFRAME.0 as isize == 0;

        if covers_monitor && borderless {
            OverlayObstruction::SuspectedExclusiveFullscreen
        } else {
            OverlayObstruction::None
        }
    }
}
