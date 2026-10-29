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

## P8 — Media sessions (Phase 2)

### P8.1 Real metadata from a real application
1. Play something in Spotify, a browser, or any local player.
2. Open Settings → Music sources.
3. **Pass:** the application is listed, the track title and artist match, and the
   overlay's Now playing panel shows the same.

### P8.2 Capabilities are read, not assumed
1. With a source playing, open Settings → Music sources → Reported capabilities.
2. **Pass:** the flags match what that source genuinely supports. A hard-coded
   implementation shows all six lit; a correct one does not.
3. Use `tests/fixtures/media-session.html` for the decisive case: it registers
   play/pause/next/previous but **not** `seekto`, so seek must read as unavailable and
   the overlay's progress bar must not be clickable.
   **Note:** the fixture must be started with a real click. Chromium only publishes a
   media session to Windows after a genuine user gesture, so this check cannot be
   automated end to end.

### P8.3 Back-dated timing
1. With a source **playing**, check the overlay's timing readout.
2. **Pass:** `Source timing` reads `exact`, meaning `LastUpdatedTime` was used to
   back-date the observation. A long-paused source correctly reads `coarse`, because
   its last sample is genuinely old.

### P8.4 Artwork
**Pass:** album art appears for a track that has it, and the diagnostics panel shows a
bounded cache count that does not grow without limit over a long listening session.

### P8.5 Arbitration
1. Play in one application, pause another.
2. **Pass:** LIQUID follows the playing one. Pin the paused one in Settings →
   **Pass:** LIQUID follows the pinned source even though it is paused.

### P8.6 Transport controls
**Pass:** play/pause/next work where the source advertises them; a control the source
does not advertise is disabled and its tooltip explains why.

---

### Run 2026-10-29 (c) — Phase 2, media sessions

Observed against the machine's **live** media sessions, not a mock.

| Check | Result | Evidence |
|---|---|---|
| Session manager acquired | **PASS** | `media session manager acquired` |
| Real metadata from a real application | **PASS** | Brave, playing a YouTube mix: title `Chill Cool Vibes for Chill Cool People // Tyler, The Creator // Mac Miller // The Marías & More`, artist `Ammafari Music Jams`, duration `3924581 ms` (65 min), position `842275 ms`. |
| Optional fields stay optional | **PASS** | Same session reported `album=None` — rendered as absent, not as filler. |
| Source identified and named | **PASS** | `source=brave.exe` → displayed as "Brave". |
| **Capabilities read, not assumed** | **PASS** | `play=true pause=false next=true previous=true seek=true` on a *paused* session. A hard-coded "all true" could not produce `pause=false`; Chromium genuinely reports pause as unavailable while already paused. |
| Capability case for a source that **cannot seek** | **NOT VERIFIED** | The fixture exists but needs a real click (see P8.2). Not exercised. |
| Artwork extracted and decoded | **PASS** | `artwork fetched key=08698587d4e8354d bytes=21186 mime=image/png` — sniffed as PNG from the real thumbnail stream. |
| Artwork cache bounded | **PASS (unit)** | Eviction and size-limit tests in `media::artwork`. |
| Arbitration ran against live sessions | **PASS** | `source arbitration changed reason=Some(CurrentFallback)` — correct for a paused source that the OS reports as current. |
| `timing=coarse` for a long-paused source | **PASS** | Correct: that session's last sample was genuinely old, so back-dating is refused rather than fabricated. |
| **`timing=exact` back-dating on a playing source** | **NOT VERIFIED** | Needs a source that is actually playing. The window logic is unit-tested; the live path is not. |
| Transport controls end to end | **NOT VERIFIED** | Would start audio on the user's machine unprompted. |

**How to close the three gaps in about a minute:** press play on the already-loaded
Brave tab, then check Settings → Music sources and the overlay's timing readout
(P8.2/P8.3/P8.6). For the non-seekable case, open
`tests/fixtures/media-session.html` and click Play.

---

## P9 — Lyrics (Phase 3)

### P9.1 Lyrics appear for a track that has them
1. Play a reasonably well-known song in Spotify or a browser.
2. **Pass:** the lyrics panel fills, the active line is highlighted and advances in
   time with the music, and "Lyrics from LRCLIB" is credited in the footer.

### P9.2 A line-timed file is not dressed up as karaoke
**Pass:** the footer reads `line-timed (approximate within a line)` and no per-word
highlight sweeps across the active line. Word highlighting may only appear when the
footer reads `word-timed`.

### P9.3 Offsets
1. Nudge the offset with the ±10/50/100 ms buttons.
2. **Pass:** lyrics shift immediately, **positive moves them earlier**.
3. Restart LIQUID. **Pass:** the offset is still applied to that track.
4. Play the same song from a different app. **Pass:** the offset still applies — the
   key deliberately excludes the provider.

### P9.4 A track with no lyrics
**Pass:** "No lyrics found for this track", not an error and not a spinner forever.
Play it again — **Pass:** no second network request (the absence is cached).

### P9.5 Ambiguous matches are offered, not guessed
Play a track with several similarly-named recordings.
**Pass:** when nothing is confident enough, a short list of candidates with match
percentages appears instead of one being applied silently.

### P9.6 A live or remixed version does not inherit studio lyrics
Play a live recording whose title says so.
**Pass:** studio lyrics are refused — either nothing, or a candidate list to choose
from. This is the single worst failure mode in a lyrics app.

### P9.7 A source that does not report position
Some browser sources publish a frozen timeline.
**Pass:** the panel states that the source is not reporting playback position rather
than highlighting a wrong line.

### Run 2026-10-29 (d) — Phase 3

| Check | Result | Evidence |
|---|---|---|
| LRCLIB reachable, correct politeness headers | **PASS** | Live lookup succeeded with the required User-Agent. |
| `503` backoff with jitter | **PASS (live)** | `backing off status=503 attempt=1 delay_ms=1036`, `attempt=2 delay_ms=1160`, then `provider replied` — recovered rather than failing. |
| "No lyrics" path | **PASS (live)** | A video with no lyrics returned `count=0` and the panel showed the absent state. |
| Frozen-timeline detection | **PASS (live)** | Brave held `LastUpdatedTime` for 41 min with `Position` at 13 ms; reported as `timing=Unknown`. |
| Parse + match + clock pipeline | **PASS** | 17-test end-to-end suite against a real captured LRCLIB record. |
| `[offset:]` tag direction | **PASS** | Regression test added after a real sign bug was found. |
| **Lyrics rendered for a track that has them** | **NOT VERIFIED** | No music with lyrics was playing during testing; only a video. P9.1–P9.3, P9.5, P9.6 need a real song. |

---

## P10 — Audio and visualisers (Phase 4)

### P10.1 The visualiser tracks real audio
Play music and watch the spectrum.
**Pass:** bars move with the music, bass hits push the low bands, and the display is
clearly correlated with what you hear — not a loop playing regardless.

### P10.2 Silence is still
Pause the music.
**Pass:** the bars fall to nothing within a second and stay there. They must not
freeze at their last height, and must not animate on nothing.

### P10.3 Every visualiser mode works
Cycle bars, mirrored, circular, waveform and particles.
**Pass:** each renders and responds; none is a static image.

### P10.4 Device selection and recovery
1. Settings → Audio capture, switch endpoints. **Pass:** capture restarts on the new
   device and the status shows it.
2. Unplug or disable the active device while playing. **Pass:** state goes to
   `recovering` and capture resumes rather than the thread dying.

### P10.5 Overruns are not falsely reported
Leave LIQUID running for several minutes with audio playing.
**Pass:** `Dropped samples` stays at **none**. A steadily climbing count during
healthy capture is the regression fixed in this phase.

### P10.6 The system-mix limitation is stated
**Pass:** Settings → Audio capture says plainly that loopback captures everything
the machine plays and that per-process capture is unavailable on this build.

### Run 2026-10-29 (e) — Phase 4

| Check | Result | Evidence |
|---|---|---|
| Capture opens the default endpoint | **PASS (live)** | `device=Voicemeeter AUX Input (VB-Audio Voicemeeter VAIO) sample_rate=48000 channels=2 bits=32 is_float=true` |
| **Real audio produces correct measurements** | **PASS (live)** | A generated 1 kHz tone at amplitude 0.22: `peak=0.21984` (0.07% error), `rms=0.15498` vs theoretical 0.15556 (0.4% error). |
| Band placement on real audio | **PASS (live)** | `mid=0.0775` dominant, `bass≈0`, `bands_lit=5` of 64 — energy where 1 kHz belongs, no smearing. |
| Silence is still | **PASS (live)** | `silent=true`, all bands zero before and after the tone. |
| No false overrun reporting | **PASS (live)** | `dropped=0` throughout, after the `retain_latest` fix. Before it, a healthy capture reported ~5,000 dropped samples per second. |
| Band mapping, windowing, beat detection | **PASS** | 46 unit tests against synthetic signals. |
| Ring buffer under concurrency | **PASS** | Producer and consumer on separate threads, with accounting asserted. |
| **Visualiser rendering on screen** | **NOT VERIFIED** | The WebGL2 output needs a human looking at it. The data feeding it is verified; the pixels are not. |
| Device switching and recovery | **NOT VERIFIED** | Needs someone to change or unplug a device (P10.4). |

---

### Correction to the first Phase 1 run

The original Phase 1 evidence below was gathered by launching the **debug binary
directly**, with no Vite dev server running. Every window was therefore showing
WebView2's `ERR_CONNECTION_REFUSED` page the entire time. The window-style results
were still accurate — styles are a property of the native window, not its contents —
but the run proved nothing about whether the app *renders*, and the report did not
say so because the check did not exist.

That gap is closed: each webview now calls `report_frontend_ready` when React mounts,
and **no run counts as a pass unless the log shows one `frontend ready` line per
window.** A created window is not a working window.

### Run 2026-10-29 (b) — after the dev-server fix

Command: `npm run tauri:dev`. All results below are from the same run.

| Check | Result | Evidence |
|---|---|---|
| Vite binds where the webview looks | **AUTOMATED PASS** | `netstat`: `127.0.0.1:5183 LISTENING`. Both `http://127.0.0.1:5183/` and `http://localhost:5183/` return HTTP 200. |
| Settings window renders | **AUTOMATED PASS** | `frontend ready label=main route=settings href=http://127.0.0.1:5183/` |
| Overlay 1 renders | **AUTOMATED PASS** | `frontend ready label=overlay-0 route=overlay` |
| Overlay 2 renders | **AUTOMATED PASS** | `frontend ready label=overlay-1 route=overlay` |
| Each window loads its intended route | **AUTOMATED PASS** | Routes logged as `settings`, `overlay`, `overlay` — label-based routing resolves correctly. |
| No `ERR_CONNECTION_REFUSED` | **AUTOMATED PASS** | A webview that cannot load the document cannot execute JS, so three `frontend ready` lines are only possible if all three documents loaded. |
| No warnings or errors at startup | **AUTOMATED PASS** | `grep -E "WARN|ERROR"` over the log: no matches. |
| Overlay styles still correct under `tauri dev` | **AUTOMATED PASS** | External probe: `TOPMOST, TOOLWINDOW, NOACTIVATE` on both overlays. |

### Run 2026-10-29 (a) — Phase 1, superseded by the correction above

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
