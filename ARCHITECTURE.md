# LIQUID — Architecture

A Windows 10 always-on-top music visualiser and live-lyrics HUD. Tauri 2 shell, Rust native
layer, React/TypeScript front end, WebGL2 rendering.

Read `COMPATIBILITY.md` first — it defines which of the capabilities below are actually
available on the target platform. This document describes structure; that one describes
reality.

---

## 1. Process and window topology

```
                    ┌──────────────────────────── liquid.exe (single process) ─┐
                    │                                                           │
   WinRT GSMTC ────►│  Rust core                                                │
   WASAPI loopback ►│  ┌─────────────┬──────────────┬────────────────────────┐  │
   Win32 windowing ►│  │ audio::     │ media::      │ timing::               │  │
   QPC ────────────►│  │ capture+dsp │ GSMTC bridge │ playback clock         │  │
                    │  ├─────────────┼──────────────┼────────────────────────┤  │
                    │  │ windows::   │ overlays::   │ lyrics::  storage::    │  │
                    │  │ Win32 glue  │ layout mgr   │ fetch+cache  SQLite    │  │
                    │  └─────────────┴──────────────┴────────────────────────┘  │
                    │         │ typed Tauri commands + events                   │
                    │  ┌──────▼──────────────────────────────────────────────┐  │
                    │  │ WebView2 webviews (one per window)                  │  │
                    │  │  main (settings)   overlay-*   island   editor      │  │
                    │  └─────────────────────────────────────────────────────┘  │
                    └───────────────────────────────────────────────────────────┘
```

**Windows created:**

| Window label | Taskbar | Styles | Purpose |
|---|---|---|---|
| `main` | yes | normal, decorated | Settings, onboarding, diagnostics, lyrics editor host |
| `overlay-<monitor-id>` | no | `WS_EX_TOOLWINDOW \| WS_EX_NOACTIVATE`, transparent, topmost | One widget canvas per monitor |
| `island` | no | same as overlay | Dynamic Island capsule |
| `obs` | no | transparent, *not* topmost | Fixed-size capture surface for OBS |

One overlay window per monitor rather than one giant spanning window: a spanning window
breaks per-monitor DPI, forces the largest monitor's backing store everywhere, and makes
click-through regions ambiguous.

---

## 2. Rust module boundaries

Each module exposes a narrow, documented interface. Platform code never leaks into
application logic; everything Win32/WinRT lives under `windows::`, `media::`, `audio::`.

```
src-tauri/src/
├── main.rs             Tauri builder, window creation, command registry
├── state.rs            AppState: the single owner of shared handles
├── windows/            PLATFORM-SPECIFIC. Nothing else may call Win32 directly.
│   ├── mod.rs          re-exports a platform-neutral facade
│   ├── overlay.rs      ex-style application, topmost enforcement, no_redirection_bitmap
│   ├── hittest.rs      WM_NCHITTEST subclass, click-through, drag regions
│   ├── monitors.rs     EnumDisplayMonitors, per-monitor DPI, hot-plug via WM_DISPLAYCHANGE
│   ├── hotkeys.rs      RegisterHotKey, conflict detection, rebinding
│   ├── tray.rs         shell notify icon + menu (incl. click-through rescue)
│   ├── foreground.rs   active-app detection via WinEvent hook (event-driven, not polled)
│   └── startup.rs      HKCU\...\Run registration (no admin, no scheduled task)
├── media/
│   ├── gsmtc.rs        Windows.Media.Control session manager + change subscriptions
│   ├── provider.rs     Provider trait + capability flags
│   ├── arbitration.rs  which source wins when several are playing
│   └── synthetic.rs    deterministic test provider (used by the timing test suite)
├── timing/
│   ├── clock.rs        QPC-based monotonic clock
│   ├── interpolator.rs position prediction, drift estimation, uncertainty
│   └── offsets.rs      per-track offsets, persisted
├── audio/
│   ├── wasapi.rs       loopback capture thread, device notifications
│   ├── ring.rs         lock-free SPSC ring buffer (capture thread -> dsp thread)
│   ├── dsp.rs          window fn, FFT, bands, RMS, flux, beat detection
│   └── features.rs     the AudioFeatures struct emitted to the UI
├── lyrics/
│   ├── lrc.rs          LRC + Enhanced-LRC parser (pure, heavily unit-tested)
│   ├── lyricsfile.rs   LRCLIB lyricsfile YAML (word-level timings)
│   ├── matcher.rs      candidate scoring; refuses low-confidence auto-apply
│   ├── lrclib.rs       HTTP client: UA, throttle, 429 + Retry-After honouring
│   └── cache.rs        on-disk + SQLite cache
├── overlays/
│   ├── layout.rs       widget model, snapping, monitor assignment
│   └── profiles.rs     per-application profile matching and activation
├── storage/
│   ├── db.rs           SQLite pool
│   ├── migrations/     versioned, forward-only, backed up before destructive steps
│   └── settings.rs     schema-validated settings with migration
└── diagnostics/
    ├── metrics.rs      FPS, frame time, audio latency, timing uncertainty
    └── log.rs          structured tracing to rotating files
```

### Threading model

| Thread | Owns | Must never |
|---|---|---|
| Main / UI | Tauri event loop, all window handles, Win32 message pump | block on audio or network |
| Audio capture | WASAPI client, writes to ring buffer | allocate, lock, or log in the hot path |
| DSP | reads ring buffer, FFT, emits `AudioFeatures` at a fixed rate | touch window handles |
| Lyrics/network | HTTP, parsing, cache writes | touch audio or windows |
| Renderer (per webview) | WebGL2 draw loop | do layout-thrashing reads mid-frame |

Window handles are main-thread-only. Everything else communicates by channels and typed
Tauri events.

---

## 3. The playback clock — the single most important component

Lyrics accuracy is bounded entirely by position accuracy. GSMTC gives a position only when
it changes, and it is coarse and occasionally stale, so a prediction layer is mandatory.

```
predicted = last_confirmed_position
          + (now_qpc - observed_at_qpc) * playback_rate
```

Applied separately, never folded into the stored position:
- `user_offset_ms` — per-track, persisted
- `global_offset_ms` — device/pipeline latency compensation

**Uncertainty is tracked, not ignored.** The clock publishes
`{ position_ms, uncertainty_ms, confidence, is_stale }`. `uncertainty_ms` grows with the age
of the last observation and with observed drift variance. The UI de-emphasises word-level
highlighting when uncertainty exceeds a threshold instead of animating confidently from a
position it no longer trusts.

**Discontinuity handling.** Each observation is classified:

| Classification | Test | Response |
|---|---|---|
| Normal | \|observed − predicted\| < 250 ms | slew-correct gradually (max ~50 ms/s) so lyrics never visibly snap |
| Seek | \|observed − predicted\| ≥ 250 ms, same track | hard reset, clear drift estimate, resync immediately |
| Track change | track identity differs | full reset; drop lyrics; re-resolve |
| Pause / resume | playback status transition | freeze/unfreeze the integrator |
| Rate change | rate differs | reset integrator with the new rate |
| Stale | no observation for > 5 s while playing | raise uncertainty, mark `is_stale` |

Track identity is `(provider, provider_track_id?, normalised_title, normalised_artist, duration_bucket)`
— never the title alone, so offsets survive and do not cross-contaminate.

---

## 4. Provider abstraction

```rust
pub struct PlaybackState {
    pub provider: ProviderId,
    pub track: TrackIdentity,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub artwork: Option<ArtworkRef>,
    pub duration_ms: Option<u64>,
    pub position_ms: u64,
    pub status: PlaybackStatus,       // Playing | Paused | Stopped | Unknown
    pub rate: f64,
    pub observed_at: QpcInstant,      // when, on OUR monotonic clock
    pub timing_confidence: Confidence,// Exact | Interpolated | Coarse | Unknown
    pub controls: ControlCapabilities,// bitflags, as advertised by the source
    pub source_app: Option<String>,   // AUMID / exe
    pub provenance: Provenance,       // Gsmtc | Extension | Synthetic
}
```

Every optional field is genuinely optional — half the sources in the wild omit album,
artwork, or duration. Nothing is invented to fill a gap; the UI renders absence.

`ControlCapabilities` is read from the session, so a disabled Seek button means *this source
said it cannot seek*, which is then shown as a tooltip.

### Source arbitration

Configurable, in priority order: manually pinned source -> reliably audible source -> active
playing session -> most recently interacted session. Stale sessions (no update, not playing)
are demoted. The loudest captured audio is **never** assumed to belong to the selected
metadata source — the capture is a system mix and cannot be attributed.

---

## 5. Audio pipeline

```
WASAPI loopback ─► SPSC ring (f32 interleaved) ─► downmix ─► Hann window
   ─► rFFT (2048) ─► magnitude ─► log-spaced bands (configurable 8..256)
   ─► per-band attack/release smoothing ─► AudioFeatures
                                            ├── bands[]
                                            ├── waveform[]
                                            ├── rms / peak
                                            ├── spectral_flux
                                            ├── bass_energy
                                            ├── beat (flux onset + adaptive threshold)
                                            └── silent (gate)
```

Band mapping is logarithmic because linear FFT bins put almost everything musical in the
bottom few pixels. Attack and release are separate so bars rise instantly and fall smoothly.

`AudioFeatures` is pushed to webviews at a fixed, settings-capped rate — decoupled from both
the FFT rate and the render rate, so the renderer never blocks on audio and audio never
blocks on the renderer.

---

## 6. Rendering and the glass system

- **WebGL2** is the baseline and must work alone. WebGPU is probed, opt-in, and falls back
  silently.
- One `<canvas>` per visualiser widget, each a small program in a modular shader registry
  with declared uniforms (audio features, time, resolution, theme colours).
- **Glass is CSS + shader approximation**, by necessity — see the honesty note in
  `COMPATIBILITY.md` §5. Layers: translucency, procedural frosted-noise substrate, simulated
  border refraction, specular edge highlight, album-art tint, soft shadow.
- All colour flows from **semantic tokens** (`--lq-surface`, `--lq-text-primary`,
  `--lq-accent`, …). No component hard-codes a colour, which is what makes eight themes plus
  album-adaptive tinting tractable.
- Album-art extraction produces dominant + accent + background + a **contrast-checked** text
  colour, so adaptive theming can never make lyrics unreadable.

### Untrusted presets

Imported themes, presets and shaders are data, never code. A preset manifest is
size-limited, schema-validated, and may contain only declared parameters. Shader source is
compiled in a sandboxed context; compile failure yields a described error and a fallback
program. **No preset can execute JavaScript, Rust, shell commands, or binaries.** A safe
mode boots with built-in defaults to recover from a bad import.

---

## 7. Front-end structure

```
src/
├── app/           shell, routing, window-role bootstrap
├── core/          pure, dependency-free logic — the unit-test target
│   ├── lrc.ts         parser (mirrors the Rust parser; both are tested)
│   ├── clock.ts       client-side prediction between backend ticks
│   ├── identity.ts    track identity normalisation
│   └── colour.ts      palette extraction + contrast
├── providers/     typed IPC clients for the Rust commands
├── widgets/       one directory per widget, each self-contained
├── visualizers/   WebGL2 programs + shader registry
├── lyrics/        layout components, all sharing one timing source
├── themes/        token definitions and theme JSON
├── settings/      schema-driven settings UI
├── profiles/      per-app profile editor
├── hooks/         useAudioFeatures, usePlaybackClock, useLyrics, …
└── styles/        token layer, glass primitives
```

State: Zustand, split into independent stores (playback, lyrics, audio, layout, theme,
settings) so a lyrics update does not re-render the visualiser.

The clock is interpolated **again** on the client between backend ticks, from the same
`observed_at` basis, so lyrics animate at display rate without the backend emitting at
display rate.

---

## 8. Data model (SQLite)

`tracks`, `providers`, `lyrics`, `lyrics_offsets`, `lyrics_versions`, `themes`, `presets`,
`widgets`, `layouts`, `monitor_profiles`, `application_profiles`, `playback_history`,
`user_settings`.

Notes that matter:
- `tracks` is keyed on composite identity (§3), not title.
- `lyrics` bodies are stored once and referenced; `lyrics_versions` holds editor history as
  diffs, not duplicated full copies.
- `playback_history` records a play only on an explicit rule (≥ 30 s listened or ≥ 50 % of
  duration, whichever is sooner, and only once per contiguous listen) — position updates are
  not plays.
- Migrations are forward-only and versioned; anything destructive backs up the DB file first.

---

## 9. Security posture

- Tokens and secrets: Windows Credential Manager via DPAPI. **Never** in config files or
  Git. (Currently there are no Spotify credentials at all — see `COMPATIBILITY.md` §4.)
- Browser extension <-> app: loopback-only, **paired with a token the user confirms**, schema
  validated, origin checked, and restricted to a small set of read/report operations. An
  arbitrary web page cannot drive the desktop app.
- Imported presets are parsed as data with size limits; no dynamic code execution.
- No telemetry. History is local and can be disabled entirely.
- Runs unelevated. Startup uses `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

---

## 10. Testing strategy

The **synthetic provider** is the backbone: it emits a deterministic, scriptable playback
timeline (including seeks, pauses, rate changes and stale gaps), so lyric-sync accuracy is
measured as a number against a known ground truth rather than judged by eye. Streaming
services are never a test dependency.

Pure logic (LRC parsing, interpolation, offsets, identity, scoring, DSP maths, migrations,
history aggregation, theme validation) is unit-tested on both sides of the IPC boundary.
Win32 behaviour — topmost, click-through, DPI, hot-plug, hotkeys, tray — is integration- and
manually tested on the real OS, with the procedure recorded in `docs/`.
