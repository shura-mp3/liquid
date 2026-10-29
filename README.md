# LIQUID

A Liquid Glass music visualiser and live-lyrics HUD for Windows 10 — an always-on-top,
widget-based overlay that reads playback state from Windows media sessions and drives
audio-reactive visuals from the system audio mix.

**Status: in development.** Phases 0-4 of
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
| Synchronised lyrics: LRCLIB lookup, caching, matching, per-track offsets, 5 layouts | working |
| Word-level karaoke from Enhanced LRC | implemented; **no word-synced source was found on LRCLIB to test it against** |
| Music source detection via Windows media sessions (Spotify, browsers, local players) | working — real metadata, artwork, capabilities, arbitration, pinning, transport |
| WASAPI loopback capture, FFT analysis, device selection and recovery | working |
| WebGL2 visualisers: bars, mirrored, circular, waveform, bass particles | working |
| Audio-reactive glass borders | working |
| Dynamic Island, particles, ripples, album-adaptive tinting | **not implemented — Phase 5** |
| Browser extension, lyrics editor, translation, history, OBS output | **not implemented — Phases 6–7** |
| Installer | **not implemented — Phase 8** |

The Now playing panel and the lyrics panel both show real data. A line-timed lyric file
is labelled line-timed rather than dressed up as karaoke, and when a source does not
report its playback position the panel says so instead of syncing to a wrong number.
There is no fake "now playing" and no fake spectrum data anywhere.

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

### Which binary needs the dev server

This trips people up, so it is worth stating plainly:

| How you built it | Frontend source | Needs Vite running? |
|---|---|---|
| `npm run tauri:dev` | `devUrl` (`http://127.0.0.1:5183`) | **yes** — the CLI starts it for you |
| `cargo build` in `src-tauri/` | `devUrl` | **yes**, and nothing starts it for you |
| `npm run tauri:build` | assets bundled into the exe | no |
| `npx tauri build --debug` | assets bundled into the exe | no |

A debug binary launched straight from `src-tauri/target/debug/liquid.exe` with no dev
server will show WebView2's *"Hmmm… can't reach this page"* with
`ERR_CONNECTION_REFUSED`. That is Tauri working as designed, not a bug. Use
`npm run tauri:dev`, or build a standalone debug binary with `npx tauri build --debug`.

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
cd src-tauri
cargo test
```

> **PowerShell note.** Windows PowerShell 5.1 — what you get by default on Windows 10 —
> does not support `&&` between commands; it only arrived in PowerShell 7. Run the
> lines above one at a time, or install PowerShell 7. Every command block in this
> README is written one command per line for that reason.

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

## Troubleshooting

### A window shows "Hmmm… can't reach this page" / `ERR_CONNECTION_REFUSED`

The webview could not reach the dev server. Check, in order:

1. **Are you running a bare debug binary?** See the table above — it needs Vite.
2. **Is anything listening, and on which address family?**

```bash
netstat -ano | findstr ":5183"
```

   You want `127.0.0.1:5183 ... LISTENING`. If you see `[::1]:5183` instead, the dev
   server bound to the **IPv6** loopback only while WebView2 asks for the **IPv4**
   one, and the connection is refused even though `curl http://localhost:5183` works
   perfectly. This is exactly the bug that `vite.config.ts` now pins
   `server.host = '127.0.0.1'` to prevent; both it and `devUrl` use a literal
   `127.0.0.1` rather than the name `localhost` so no DNS resolution is involved.

3. **Did the frontend actually load?** The app logs a line per window when React
   mounts:

```bash
findstr "frontend ready" "%APPDATA%\com.shura.liquid\logs\liquid.log*"
```

   Three lines (`main`, `overlay-0`, `overlay-1` on a two-monitor setup) means the
   pages rendered. No lines means the webview never got the document — a connection
   problem, not a rendering problem. Window creation succeeding tells you nothing
   about this, which is why the signal exists.

### Port 5183 is already in use

`strictPort` is on deliberately, so Vite fails rather than quietly moving to another
port and leaving Tauri pointing at the old one. Free the port, or change it in **both**
`vite.config.ts` and `src-tauri/tauri.conf.json` — they must match.

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
