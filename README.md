# LIQUID

A Liquid Glass music visualiser and live-lyrics HUD for Windows 10 — an always-on-top,
widget-based overlay that reads playback state from Windows media sessions and drives
audio-reactive visuals from the system audio mix.

**Status: in development.** Phase 0 and Phase 1 of
[`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) are done. The sections below say
exactly what works today and what does not, because a screenshot of a HUD with
placeholder text in it is not a feature.

---

## What works today

| Area | State |
|---|---|
| Transparent, borderless, always-on-top overlay (one window per monitor) | working |
| Click-through, with a hotkey **and** a tray item that always restore interactivity | working |
| Overlay never takes focus from the foreground app (`WS_EX_NOACTIVATE`) | working |
| Draggable, snapping, lockable widgets | working |
| Global hotkeys with conflict reporting | working |
| System tray icon and menu | working |
| Monitor enumeration with per-monitor DPI, negative coordinates, off-screen rescue | working |
| Liquid Glass token + shader-approximation styling, 7 themes | working |
| Settings in SQLite with schema validation and versioned migrations | working |
| Launch at login (per-user, no admin) | working |
| Diagnostics panel reading real OS state | working |
| LRC / Enhanced-LRC parsing, playback clock, matcher, offsets | **built and tested, not yet wired to a live provider** |
| Music source detection (Spotify / YouTube / SoundCloud / local via GSMTC) | **not implemented — Phase 2** |
| WASAPI audio capture and FFT visualisers | **not implemented — Phase 4** |
| Dynamic Island, particles, ripples, album-adaptive tinting | **not implemented — Phase 5** |
| Browser extension, lyrics editor, translation, history, OBS output | **not implemented — Phases 6–7** |
| Installer | **not implemented — Phase 8** |

The overlay currently shows placeholder lyric text that is explicitly labelled as
placeholder. There is no fake "now playing" and no fake spectrum data anywhere.

---

## Things worth knowing before you try it

- **The overlay cannot cover everything.** Exclusive-fullscreen games, UAC prompts, the
  lock screen, DRM-protected video and some elevated or anti-cheat-protected windows will
  sit above it. That is how Windows works. LIQUID does not inject into other processes or
  attempt to defeat any of it; for games, borderless windowed mode is the fix.
- **The "glass" is an approximation, and an honest one.** A transparent overlay cannot read
  the pixels of windows behind it — no API provides that, and continuously capturing the
  screen for a cosmetic effect would waste GPU bandwidth and trip protected-content rules.
  LIQUID layers real `backdrop-filter` blur with a procedural frost grain, a specular sweep
  and a rim light. See [`COMPATIBILITY.md` §5](COMPATIBILITY.md).
- **There is no Spotify API integration, deliberately.** No OAuth, no client ID, no key.
  Spotify's Developer Policy §III.6 prohibits synchronising recordings with visual media and
  §III.13 prohibits analysing Spotify Content, so LIQUID stays off the Spotify Platform
  entirely and reads only Windows media-session metadata, which the OS exposes uniformly for
  every media app. The reasoning, including what remains legally unresolved, is written out
  in [`COMPATIBILITY.md` §4](COMPATIBILITY.md).
- **Audio analysis is of the whole system mix.** Windows 10 build 19045 has no per-process
  loopback capture, so a game, a call or a notification is part of what the visualiser sees.
- **No telemetry.** Nothing is collected and nothing is uploaded. Logs and history stay on
  your machine.

---

## Requirements

- Windows 10 version 22H2 (build 19041+) or Windows 11, 64-bit
- [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) — already
  present on most up-to-date Windows 10 installs; the installer will fetch it if missing
- A GPU with WebGL2 support (anything from the last decade)

## Building from source

You need three things, and the C++ workload is the one people miss:

1. **Node.js 20+** — <https://nodejs.org>
2. **Rust, MSVC toolchain** — <https://rustup.rs>, then `rustup default stable-x86_64-pc-windows-msvc`
3. **Visual Studio Build Tools with the C++ workload and a Windows SDK.** Installing Build
   Tools *without* the C++ workload leaves you with no `link.exe` and no SDK, and Rust will
   fail to link with a confusing error:

```bash
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.Windows11SDK.22621"
```

Then:

```bash
git clone https://github.com/shura-mp3/liquid.git
cd liquid
npm install
npm run tauri:dev
```

### Useful commands

```bash
npm test                 # the full test suite
npm run typecheck        # TypeScript, no emit
npm run build            # production frontend bundle
npm run tauri:build      # release binary + NSIS installer
npx vite-node scripts/sync-report.ts   # print measured lyric sync accuracy
node scripts/make-icon.mjs             # regenerate the app icon from source
```

Rust tests:

```bash
cd src-tauri && cargo test
```

---

## Default hotkeys

| Combination | Action |
|---|---|
| `Ctrl+Alt+L` | Toggle lyrics |
| `Ctrl+Alt+M` | Toggle the music HUD |
| `Ctrl+Alt+G` | Toggle glass mode |
| `Ctrl+Alt+C` | Toggle click-through |
| `Ctrl+Alt+V` | Toggle the visualiser |
| `Ctrl+Alt+O` | Open settings |

All rebindable. A combination already owned by another application is reported in the log
rather than silently stolen or silently lost.

**If the overlay stops responding to the mouse,** click-through is on. Press `Ctrl+Alt+C`,
or right-click the tray icon and choose *Restore interactivity*. Both paths work
independently of the overlay window, so you cannot get locked out.

---

## Where your data lives

Everything is under `%APPDATA%\com.shura.liquid`:

- `liquid.db` — settings, widget layouts, lyric offsets (SQLite)
- `logs\liquid.log` — rotated daily

Delete that folder to reset LIQUID completely.

---

## Documentation

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module boundaries, threading, the playback clock
- [`COMPATIBILITY.md`](COMPATIBILITY.md) — verified capability matrix and the policy review
- [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — phases and exit criteria
- [`docs/MANUAL_TESTS.md`](docs/MANUAL_TESTS.md) — the Windows behaviour that cannot be automated

## Licence

MIT. LIQUID bundles no music, no lyrics and no third-party media.
