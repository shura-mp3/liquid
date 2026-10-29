# LIQUID — Implementation Plan

Incremental, phase-gated. A phase is not "done" because it compiles; it is done when its
exit criteria are demonstrated. Each phase ends with a report: Implemented / Tested /
Blocked / Limitations / Files changed / Next steps.

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done & verified

---

## Phase 0 — Environment, feasibility, policy  `[x]`

- [x] Inspect the development machine (Rust, Node, VS Build Tools, Windows SDK, WebView2, GPU)
- [x] Identify the toolchain blocker (no C++ workload, no Windows SDK) and resolve it with consent
- [x] Verify GSMTC availability for Windows 10 build 19045 (requires 17763+)
- [x] Review the **primary** Spotify Developer Policy; record III.6, III.7, III.11, III.13, II.5
- [x] Decide the Spotify posture (no Web API, GSMTC only) and document it
- [x] Verify LRCLIB endpoints, required User-Agent, rate limits, `hasWordSync`, ±2 s duration rule
- [x] Establish the architecture and module boundaries
- [x] Create the repository, initialise Git, point `origin` at `shura-mp3/liquid`
- [x] Write `ARCHITECTURE.md`, `IMPLEMENTATION_PLAN.md`, `COMPATIBILITY.md`

**Exit criteria:** a compatibility matrix separating SUPPORTED / CONDITIONAL / UNAVAILABLE /
DELIBERATELY-NOT-IMPLEMENTED, with primary sources. — met.

---

## Phase 1 — Functional desktop foundation  `[~]` built, awaiting visual verification

Goal: a real transparent always-on-top overlay on Windows 10, proven on this machine.

- [x] Tauri 2 + React + TS + Vite project, building and running
- [x] `main` settings window (decorated, taskbar present)
- [x] `overlay-*` window: transparent, borderless, topmost, `WS_EX_TOOLWINDOW`, `WS_EX_NOACTIVATE`
- [x] Topmost + style re-assertion on focus change and after every visibility change
- [x] Click-through via `WS_EX_TRANSPARENT` toggle, **debounced** (mitigates tauri#15947)
- [x] **Recovery guarantees:** tray menu item + global hotkey that always restore interactivity
- [x] Global hotkeys, with failures to register reported rather than swallowed
- [x] System tray icon and menu
- [x] Monitor enumeration, per-monitor DPI, off-screen widget rescue
- [x] Baseline Liquid Glass token layer and glass panel primitive, 7 themes
- [x] Settings persistence (SQLite + schema validation + migration v1)
- [x] External style-verification script (`scripts/verify-overlay-styles.ps1`)
- [ ] Hot-plug handling on `WM_DISPLAYCHANGE` (rescue logic exists and is tested; the event
      hook that invokes it is not wired yet)
- [ ] `WM_NCHITTEST` subclass — not needed yet: drag is handled in the webview, and the
      current widget model has no non-rectangular hit regions. Deferred to the widget system.
- [ ] Live hotkey re-registration on rebind (currently requires a restart)

**Exit criteria**
1. ~~Overlay visibly floats above File Explorer, a browser, and a borderless-windowed app.~~
   **NOT VERIFIED** — needs a human; styles confirmed, visual Z-order not observed.
2. Click-through on/off: **PARTIALLY VERIFIED** — `WS_EX_TRANSPARENT` confirmed to flip via
   the real global hotkey on both overlays, externally probed. That clicks land on the
   window beneath, and tray recovery, still need a human.
3. ~~Overlay does not take focus from the foreground app.~~ **PARTIALLY VERIFIED** —
   `WS_EX_NOACTIVATE` confirmed present; behaviour not observed.
4. ~~Overlay appears correctly on each monitor at its own DPI.~~ **PARTIALLY VERIFIED** —
   one overlay per monitor confirmed at correct coordinates; both displays are at 100 %, so
   mixed DPI is untested.
5. ~~No black transparency after 50 rapid toggles.~~ **NOT VERIFIED** — the failure mode is
   visual. Debounce is implemented at 120 ms.
6. Settings survive a restart: **VERIFIED** by the Rust test suite.

See [`docs/MANUAL_TESTS.md`](docs/MANUAL_TESTS.md) §P7 for the evidence table.

### Findings worth keeping from this phase

- **tao rewrites the entire extended-style word** when it re-applies its own window flags,
  which `show()` does. Styles set once at creation verified as written and were then
  silently reverted. They must be re-asserted after every window operation. This was only
  caught by probing the window from another process — the app's own report said it had
  succeeded, because `SetWindowLongPtrW`'s return value cannot distinguish success from a
  previous value of 0.
- **`WS_EX_LAYERED` must not be forced.** Tauri 2.12 creates transparent windows with
  `WS_EX_NOREDIRECTIONBITMAP`, so alpha comes from DirectComposition; the two flags are
  mutually exclusive. The original design doc had this wrong.
- **`skip_taskbar(true)` does not set `WS_EX_TOOLWINDOW`** on Windows — tao uses
  `ITaskbarList::DeleteTab` instead — so the flag has to be applied explicitly.

---

## Phase 2 — Music source detection  `[ ]`

- [ ] `media::gsmtc` session manager, `CurrentSessionChanged` + `MediaPropertiesChanged` subscriptions
- [ ] `PlaybackState` normalisation from GSMTC, with honest `Option` fields
- [ ] `ControlCapabilities` read from the session; disabled controls carry a reason
- [ ] Artwork extraction from the session thumbnail stream, cached by identity
- [ ] Source arbitration + manual pinning
- [ ] Synthetic provider (deterministic; the automated-test backbone)
- [ ] Provider diagnostics panel

**Exit criteria:** real metadata from at least one real app; capability flags proven correct
by a source that cannot seek; synthetic provider drives tests with zero network.

---

## Phase 3 — Lyrics engine  `[ ]`

This phase is gated: no advanced visuals until timing is measured and correct.

- [ ] LRC parser (tags, multi-timestamp lines, malformed input, BOM, CRLF)
- [ ] Enhanced-LRC word-timestamp parser
- [ ] LRCLIB `lyricsfile` YAML parser for word timings
- [ ] LRCLIB client: UA, 400 ms throttle, `429` + `Retry-After` + jitter
- [ ] Matcher: duration ±2 s, scored title/artist/album, confidence threshold, manual pick
- [ ] Cache (disk + SQLite)
- [ ] Playback clock: QPC, interpolation, drift slew, seek/pause/track/rate/stale classification
- [ ] Uncertainty publication and UI de-emphasis when untrusted
- [ ] Per-track offsets ±10/50/100 ms + custom, persisted on composite identity
- [ ] Word-level karaoke highlighting; **line-only lyrics are labelled approximate, never faked**
- [ ] Nine lyrics layouts, all on the one timing source

**Exit criteria:** against the synthetic timeline, measured mean sync error and p95 recorded
as numbers in the phase report; seek and pause handled; offsets survive restart.

---

## Phase 4 — Audio and visualisers  `[ ]`

- [ ] WASAPI loopback capture thread + SPSC ring buffer
- [ ] Device enumeration, selection, change/format recovery, health indicator
- [ ] Probe per-process loopback; expect UNAVAILABLE on 19045 and explain in UI
- [ ] Hann window, rFFT 2048, log band mapping, RMS, peak, flux, bass, beat, silence gate
- [ ] Spectrum (vertical/horizontal/circular/mirrored), waveform, particles, ripples, reactive borders
- [ ] Latency diagnostics

**Exit criteria:** visualisers demonstrably track real audio; silence produces stillness, not
idle animation; measured audio-to-visual latency recorded.

---

## Phase 5 — Advanced glass  `[ ]`

- [ ] Album-art palette extraction with contrast-safe text colour
- [ ] Glass parameter set (transparency, blur, frost, tint, saturation, edge glow, reflection,
      border, radius, ripple, speed, shadow)
- [ ] Spring animation system
- [ ] Dynamic Island collapsed/expanded with spring morph
- [ ] Theme system: 7 built-ins + Album Adaptive + Custom, editable and exportable
- [ ] Preset import/export with validation, preview, and safe mode

---

## Phase 6 — Additional providers  `[ ]`

- [ ] SoundCloud and YouTube via GSMTC browser sessions
- [ ] Companion browser extension (minimum permissions) for higher-fidelity position
- [ ] Loopback pairing channel: token the user confirms, origin + schema validation
- [ ] Per-provider capability and verification reports
- [ ] Reconnection logic

Core stability is not traded for provider breadth.

---

## Phase 7 — Advanced tools  `[ ]`

- [ ] Lyrics editor (timeline zoom, marker drag, keyboard nudge, undo/redo, validation,
      versioned backups; works **without** waveform access, waveform only for authorised local audio)
- [ ] Translation + romanization providers (opt-in, attributed, labelled machine-generated)
- [ ] Multi-monitor layouts, per-app profiles (event-driven foreground detection)
- [ ] Listening history with explicit play-count rules, stats, export, delete, disable
- [ ] OBS output mode (honest about what capture preserves alpha)

---

## Phase 8 — Production hardening  `[ ]`

- [ ] Regression suite in CI
- [ ] Performance profiling and the five performance presets
- [ ] Security review + dependency audit
- [ ] Accessibility (contrast, reduced motion, keyboard nav, screen-reader labels)
- [ ] Crash handling and recovery
- [ ] NSIS installer, WebView2 bootstrapper path, clean uninstall
- [ ] Docs: build, configure, troubleshoot, uninstall, licences
- [ ] Release validation

**Code signing is not configured. The installer will not be described as signed.**

---

## Standing rules

1. No fabricated API capabilities, no fake audio or playback data, no placeholder controls
   presented as working.
2. A provider is complete only with its own capability + verification report.
3. Tests are reported only if executed; output is quoted.
4. Windows-specific behaviour that cannot be automated gets a written manual procedure.
5. Dependencies are checked for maintenance and compatibility before adoption.
6. No secrets in Git. No admin requirement at runtime. No telemetry.
7. Commit at meaningful milestones.
