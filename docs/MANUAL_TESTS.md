# Manual Windows validation procedure

Some behaviour cannot be asserted in CI. Z-order against a real game, focus stealing,
per-monitor DPI, display hot-plug and tray interaction all depend on a live Windows
desktop with real third-party windows in it.

This file is the procedure. Each check has an explicit pass condition so a result is
recorded rather than judged. **Record the outcome per release; do not report a check
as passing unless it was run.**

Environment for the runs recorded below:
Windows 10 Home 22H2 (19045), **two displays** (`\\.\DISPLAY1` at 0,0 and
`\\.\DISPLAY2` at 1920,0, both 1920x1080 at 100 %), NVIDIA RTX 4060 Ti,
WebView2 156.0.4314.50.

Some checks that look like they need a human can in fact be automated by probing the
window from *outside* the process, which is strictly better evidence than the app
reporting on itself. `scripts/verify-overlay-styles.ps1` does this: it enumerates
top-level windows owned by `liquid.exe` and reads `WS_EX_*` via `GetWindowLongPtrW`.
Those results are recorded as AUTOMATED below.

Legend: PASS / FAIL / NOT RUN

---

## P1 — Overlay fundamentals

### P1.1 Overlay floats above an ordinary window
1. Launch LIQUID. Confirm the overlay widgets are visible.
2. Open File Explorer, maximise it, click its title bar to focus it.
3. **Pass:** the LIQUID widgets remain visible on top of Explorer.

### P1.2 Overlay floats above a browser
Repeat P1.1 with a maximised browser window playing a video.
**Pass:** widgets remain visible.

### P1.3 Overlay floats above a borderless-windowed game
1. Start any game in **borderless windowed** mode.
2. **Pass:** widgets remain visible over it.
3. Note the game and mode used.

### P1.4 Exclusive fullscreen is correctly reported, not silently broken
1. Start a game in **exclusive fullscreen**.
2. **Expected:** the overlay is *not* visible. This is a platform limitation, not a bug.
3. Alt-Tab out, open Settings → Overlays.
4. **Pass:** the obstruction column reported `suspected exclusive fullscreen` while the
   game was focused, and the panel explains the borderless-windowed remedy.

### P1.5 Overlay does not steal focus
1. Open Notepad and start typing.
2. Click directly on a LIQUID widget's drag bar and drag it.
3. Click back into Notepad without using Alt-Tab.
4. **Pass:** Notepad never lost its caret to the overlay; the overlay never appeared in
   Alt-Tab or on the taskbar.

### P1.6 Overlay is absent from the taskbar and Alt-Tab
1. Press Alt-Tab and inspect the taskbar.
2. **Pass:** only the LIQUID settings window appears (when open); no overlay window does.

---

## P2 — Click-through and recovery

### P2.1 Click-through lets clicks reach the desktop
1. Position a widget over a desktop icon or an Explorer window.
2. Enable click-through (`Ctrl+Alt+C`).
3. Click where the widget is.
4. **Pass:** the click reached the window beneath, not the widget.

### P2.2 Hotkey recovery
1. With click-through on, press `Ctrl+Alt+C`.
2. **Pass:** widgets respond to the mouse again.

### P2.3 Tray recovery
1. Enable click-through.
2. Right-click the tray icon → *Restore interactivity*.
3. **Pass:** widgets respond to the mouse again. This path must work even if the hotkey
   is unavailable, because it is the last line of defence against lock-out.

### P2.4 Click-through state survives a restart
1. Enable click-through, quit LIQUID from the tray, relaunch.
2. **Pass:** click-through is still on, and P2.2/P2.3 still recover it.

### P2.5 Rapid-toggle transparency regression (`tauri-apps/tauri#15947`)
Transparent regions have been reported turning black on Windows 10 when cursor-event
state is toggled rapidly. LIQUID debounces transitions to 120 ms; this verifies it.
1. Press `Ctrl+Alt+C` 50 times as fast as possible.
2. **Pass:** no widget shows a black rectangle instead of glass; transparency still works.

---

## P3 — Monitors and DPI

### P3.1 One overlay per monitor
1. Settings → Monitors; note the count.
2. Settings → Overlays; count the overlay windows.
3. **Pass:** the counts match, and each monitor shows widgets.

### P3.2 Per-monitor DPI
On a mixed-DPI setup (e.g. 100 % and 150 %):
1. Compare widget physical size on each monitor.
2. **Pass:** widgets are the same apparent size; text is crisp on both; the reported scale
   factor differs per monitor in Settings → Monitors.

### P3.3 Negative monitor coordinates
With a display positioned left of or above the primary one:
**Pass:** Settings → Monitors reports negative `x`/`y`, and that monitor's overlay is
positioned correctly rather than off-screen.

### P3.4 Display hot-plug
1. With LIQUID running, unplug a secondary display.
2. **Pass:** LIQUID does not crash; widgets that were on it are reachable again.
3. Re-plug it. **Pass:** still stable.

### P3.5 Resolution change
Change the primary display's resolution while LIQUID is running.
**Pass:** no crash, widgets stay on-screen and reachable.

---

## P4 — Hotkeys and tray

### P4.1 All default hotkeys fire
Test each of `Ctrl+Alt+L/M/G/C/V/O`.
**Pass:** each produces its documented effect.

### P4.2 Hotkey conflict is reported, not silently swallowed
1. Run another app that owns `Ctrl+Alt+L`, then start LIQUID.
2. **Pass:** `logs\liquid.log` contains a warning naming the combination; LIQUID starts
   normally and the other app keeps its shortcut.

### P4.3 Tray menu
**Pass:** left click opens settings; right click shows the menu; every item works; *Quit*
exits cleanly with no orphaned process in Task Manager.

---

## P5 — Settings and lifecycle

### P5.1 Settings apply live
Drag the glass sliders.
**Pass:** the preview and the overlay update immediately, with no restart.

### P5.2 Settings persist
Change theme, FPS cap and several glass values; quit from the tray; relaunch.
**Pass:** every value is retained.

### P5.3 Reset to defaults
**Pass:** *Reset everything to defaults* restores defaults and the UI reflects it at once.

### P5.4 Corrupt database recovery
1. Quit LIQUID. Open `%APPDATA%\com.shura.liquid\liquid.db` and set a settings row to
   invalid JSON.
2. Relaunch.
3. **Pass:** LIQUID starts with defaults for that key and logs a warning, rather than
   failing to start.

### P5.5 Clean exit
**Pass:** quitting from the tray leaves no `liquid.exe` in Task Manager and no WebView2
child processes.

### P5.6 Launch at login
1. Enable it; check `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
2. **Pass:** a `LIQUID` value exists with the quoted exe path. No admin prompt appeared.
3. Disable it. **Pass:** the value is gone.

---

## P6 — Visual and accessibility

### P6.1 Glass legibility over a busy desktop
Place the lyrics widget over a bright, detailed image.
**Pass:** text remains readable at the default settings.

### P6.2 Theme contrast
Cycle all seven themes.
**Pass:** text is legible in each; nothing becomes invisible.

### P6.3 Long strings do not break layout
Resize widgets narrow with the longest available text.
**Pass:** text wraps or ellipses; no overflow outside the glass panel.

### P6.4 Multilingual text
Render Japanese, Korean, Chinese, Arabic and accented Latin text in the lyrics widget.
**Pass:** all scripts render without tofu boxes or clipping.

### P6.5 Reduced motion
Enable Windows *Show animations* off / `prefers-reduced-motion`.
**Pass:** animations are reduced to near-instant.

### P6.6 Keyboard navigation
**Pass:** every control in Settings is reachable by Tab with a visible focus ring.

---

## P7 — Results log

### Run 2026-10-29 — Phase 1

| Check | Result | Evidence |
|---|---|---|
| Overlay windows created, one per monitor | **AUTOMATED PASS** | Log: `overlay-0` on `\\.\DISPLAY1` at 0,0 and `overlay-1` on `\\.\DISPLAY2` at 1920,0, each 1920x1040 (work area, so the taskbar is excluded) at scale 1.0. |
| `WS_EX_TOPMOST` present on every overlay | **AUTOMATED PASS** | External probe: ex-style `0x08040198` on both windows. |
| `WS_EX_TOOLWINDOW` present (P1.6, no taskbar/Alt-Tab) | **AUTOMATED PASS** | Same probe. |
| `WS_EX_NOACTIVATE` present (P1.5, no focus stealing) | **AUTOMATED PASS** | Same probe. Style presence is verified; the *behavioural* check against Notepad still needs a human. |
| P2.1/P2.2 click-through toggles via the real global hotkey | **AUTOMATED PASS** | `WScript.Shell.SendKeys('^%c')` flipped `WS_EX_TRANSPARENT` on → off across both overlays, with matching log lines. Style presence verified; that clicks actually land on the window beneath still needs a human. |
| P4.1 hotkey registration | **AUTOMATED PASS (partial)** | All six default combinations registered; `Ctrl+Alt+C` confirmed to fire end to end. The other five fire into the webview and were not observed. |
| P5.2 settings persistence | **AUTOMATED PASS** | Rust test suite: round-trip, reopen, and migration-idempotency tests. |
| Database migration from empty | **AUTOMATED PASS** | Log: `applied migration version=1` on a freshly deleted data directory. |
| Memory at idle | **MEASURED** | 47.8 MB working set with two overlays, the settings window and the tray active. |
| P1.1–P1.4 Z-order over real applications | **NOT RUN** | Needs a human looking at a screen with Explorer, a browser and a game in both fullscreen modes. |
| P1.5 behavioural focus check | **NOT RUN** | Needs someone typing into Notepad while dragging a widget. |
| P2.3 tray recovery | **NOT RUN** | Needs a human to click the tray menu. |
| P2.4 click-through persists across restart | **NOT RUN** | |
| P2.5 rapid-toggle black-transparency regression | **NOT RUN** | The failure mode is *visual*. Styles can be probed, blackness cannot. |
| P3.2 mixed-DPI | **NOT RUN** | Both displays on this machine are at 100 %; needs one set to 150 %. |
| P3.3 negative coordinates | **NOT RUN** | Both displays are at x >= 0; needs one placed left of the primary. |
| P3.4/P3.5 hot-plug and resolution change | **NOT RUN** | |
| P4.2 hotkey conflict reporting | **NOT RUN** | Needs another app holding one of the combinations. |
| P4.3 tray menu | **NOT RUN** | |
| P5.1/P5.3/P5.4/P5.5/P5.6 | **NOT RUN** | |
| P6.1–P6.6 visual and accessibility | **NOT RUN** | All inherently visual. |

**Why the NOT RUN rows are not pass rows.** Phase 1 was built in a non-interactive
session. Everything that could be verified by probing the OS from another process was
verified and is recorded above with its evidence. Everything that needs eyes on a screen —
Z-order against a real game, whether a click truly passes through to the window beneath,
whether transparency turns black after rapid toggling, whether text is legible over a busy
desktop — was **not** observed, and marking it as passing would be fabrication.

**Phase 1 is therefore "built and externally probed", not "verified".** The visual and
behavioural rows need one pass on a live desktop.

Automated suites executed in the same session:
**153 TypeScript tests passing**, **20 Rust tests passing**, debug build and release build
produced.
