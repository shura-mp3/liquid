//! Launch-at-login.
//!
//! Uses `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which is per-user and
//! needs no elevation. A scheduled task or an `HKLM` entry would require admin
//! rights, which LIQUID deliberately never asks for.

#![cfg(windows)]

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

use super::{PlatformError, Result};

const VALUE_NAME: PCWSTR = w!("LIQUID");

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_run_key(write: bool) -> Result<HKEY> {
    let mut key = HKEY::default();
    let access = if write { KEY_WRITE | KEY_READ } else { KEY_READ };
    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            access,
            &mut key,
        )
        .ok()
        .map_err(|e| PlatformError::Win32(format!("RegOpenKeyExW: {e}")))?;
    }
    Ok(key)
}

/// Register or unregister LIQUID for launch at login.
///
/// `minimised` adds the `--minimised` flag so the app can start to the tray without
/// showing a window, which is the usual reason to want this on at all.
pub fn set_launch_at_login(enabled: bool, minimised: bool) -> Result<()> {
    let key = open_run_key(true)?;
    let result = (|| -> Result<()> {
        if enabled {
            let exe = std::env::current_exe()
                .map_err(|e| PlatformError::Win32(format!("current_exe: {e}")))?;
            // Quoted so a path containing spaces is parsed as one argument.
            let mut command = format!("\"{}\"", exe.display());
            if minimised {
                command.push_str(" --minimised");
            }
            let wide = to_wide(&command);
            let bytes = unsafe {
                std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2)
            };
            unsafe {
                RegSetValueExW(key, VALUE_NAME, None, REG_SZ, Some(bytes))
                    .ok()
                    .map_err(|e| PlatformError::Win32(format!("RegSetValueExW: {e}")))?;
            }
        } else {
            unsafe {
                // Deleting a value that is not there is success, not failure.
                let _ = RegDeleteValueW(key, VALUE_NAME);
            }
        }
        Ok(())
    })();

    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

/// Whether LIQUID is currently registered to launch at login.
pub fn launches_at_login() -> bool {
    let Ok(key) = open_run_key(false) else {
        return false;
    };
    let mut size: u32 = 0;
    let present = unsafe {
        RegQueryValueExW(key, VALUE_NAME, None, None, None, Some(&mut size)).is_ok()
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    present
}
