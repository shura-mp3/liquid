# LIQUID — Compatibility & Policy Matrix

**Phase 0 deliverable.** Every claim in this file was verified against a primary source or a
command run on the target machine. Nothing here is aspirational; features that cannot be
verified are marked **UNAVAILABLE** or **CONDITIONAL**, not "planned".

Last verified: 2026-10-29

---

## 1. Verified development environment

Measured on the target machine, not assumed.

| Component | Result | How verified |
|---|---|---|
| OS | Windows 10 Home 22H2, build **10.0.19045** | `Win32_OperatingSystem` |
| CPU | AMD Ryzen 7 3700X (8C/16T) | `Win32_Processor` |
| GPU | NVIDIA GeForce RTX 4060 Ti | `Win32_VideoController` |
| RAM | 16 GB | `Win32_ComputerSystem` |
| Node.js | v24.19.0 | `node --version` |
| npm / pnpm | 12.0.2 / 11.21.0 | `npm -v`, `pnpm -v` |
| Git | 2.55.0.windows.3 | `git --version` |
| GitHub CLI | 2.97.0, authenticated as `shura-mp3` | `gh auth status` |
| **WebView2 Runtime** | **156.0.4314.50 (present, machine-wide)** | `HKLM\...\EdgeUpdate\Clients\{F3017226-...}` |
| Rust / cargo | **ABSENT at Phase 0 start** -> installed via `winget Rustlang.Rustup` | `rustc --version` |
| MSVC C++ toolchain | **ABSENT at Phase 0 start.** VS Build Tools 2026 was installed but with *no* C++ workload: no `cl.exe`, no `link.exe`, no Windows SDK under `Windows Kits`. Installed `Microsoft.VisualStudio.2022.BuildTools` + `Workload.VCTools` + `Windows11SDK.22621`. | `vswhere -requires`, `Get-Command link.exe` |
| Admin rights | Session is **non-elevated**. Toolchain install required an interactive UAC approval by the user. | `WindowsPrincipal.IsInRole` |

**Consequence:** Rust/Tauri was not buildable on this machine until the C++ workload was
added. This was a genuine blocker, resolved with the user's explicit consent — not a detail
to paper over.

---

## 2. Stack decision

| Choice | Decision | Rationale |
|---|---|---|
| Shell | **Tauri 2** | WebView2 runtime already present; small binaries; Rust gives direct `windows` crate access to Win32 + WinRT, which is required for GSMTC and WASAPI. |
| Renderer | **WebGL2 baseline** | Universally available through WebView2/Chromium on Win10. |
| WebGPU | **Optional, off by default** | WebGPU in WebView2 on Windows 10 is not dependably available across driver stacks. Never a hard requirement. See §5. |
| Native layer | `windows` crate | Official Microsoft bindings; covers Win32 windowing *and* WinRT `Windows.Media.Control`. |
| Storage | SQLite + versioned migrations | Per spec. |

### Known Tauri/WebView2 risks on Windows 10 (tracked, not hidden)

These are open upstream issues that directly affect a transparent always-on-top overlay:

- Transparent regions can turn **black on Windows 10** when cursor-event (click-through)
  state is toggled frequently — upstream `tauri-apps/tauri#15947`. LIQUID mitigates by
  debouncing click-through transitions and never toggling per-frame. **Must be re-tested
  on this machine in Phase 1.**
- WebView2 content can become unreadable/keyboard-unfocusable after the host window loses
  and regains activation — `tauri-apps/tauri#15624`. Relevant because the overlay uses
  `WS_EX_NOACTIVATE`.
- `setBackgroundColor` **cannot** express translucency on Windows — any alpha other than 0
  is forced to 255. Glass translucency must therefore come from CSS/shaders over a fully
  transparent window, never from the native background colour.

---

## 3. Music provider matrix

| Provider | Metadata | Position | Transport control | Mechanism | Status |
|---|---|---|---|---|---|
| **Spotify (desktop app)** | title, artist, album, artwork | yes, via `GetTimelineProperties` | play/pause/next/prev/seek as advertised by the session | Windows **GSMTC** (`Windows.Media.Control`) | **CONDITIONAL** — see §4 |
| **Spotify Web API** | — | — | — | — | **DELIBERATELY NOT IMPLEMENTED** — see §4 |
| **YouTube / YouTube Music** | title, artist (best-effort), artwork | yes | yes | GSMTC when the browser publishes a session; companion extension for higher-fidelity position | **CONDITIONAL** |
| **SoundCloud** | title, artist, artwork | yes | yes | GSMTC via browser media session; extension for better fidelity | **CONDITIONAL** |
| **Local players** (foobar2000, MusicBee, Winamp, VLC, Groove) | varies by player | varies | varies | GSMTC | **CONDITIONAL** on the player publishing SMTC |
| **Synthetic test provider** | full | full, deterministic | full | in-process fixture | **SUPPORTED** — the only provider used for automated timing tests |

GSMTC requires Windows 10 build **17763+**; build 19045 qualifies. Metadata richness is
decided by the *source application*, not by LIQUID. Providers report capabilities
explicitly; unavailable controls are rendered disabled with a reason, never as dead buttons.

---

## 4. Spotify policy review (mandatory gate)

Reviewed against the **primary source**, the Spotify Developer Policy at
`developer.spotify.com/policy`. Verbatim restrictions that bear on LIQUID:

| § | Restriction (as published) | Impact on LIQUID |
|---|---|---|
| III.6 | "Do not synchronize any sound recordings with any visual media, including any advertising, film…" | Directly implicates synchronised lyrics and audio-reactive visuals over Spotify content. |
| III.7 | "Do not permit any device or system to segue, mix, re-mix, or overlap any Spotify Content with any other audio content" | LIQUID never touches the audio path; loopback capture is read-only. No impact. |
| III.11 | Do not "mimic, or replicate or attempt to replace a core user experience" of Spotify | Transport controls are a convenience over the OS session, not a Spotify client replacement. |
| III.13 | **"Do not analyze the Spotify Content or the Spotify Service for any purpose"** | Directly implicates FFT/spectral analysis of Spotify audio. |
| II.5 | Metadata and cover art "may only be used in connection with the underlying content" | Album-art colour extraction and track display are in connection with the playing track — compliant. |
| IV.1 | Streaming only to Premium subscribers | N/A; LIQUID never streams. |

### Decision

These obligations attach to a **Spotify Developer Application** (SDA) — software that uses
the Spotify Platform (Web API / SDKs) under the Developer Terms. The architectural decision
that follows is therefore:

> **LIQUID does not integrate the Spotify Web API, Spotify SDKs, or any Spotify endpoint.
> There is no Spotify OAuth flow, no client ID, and no API key anywhere in this codebase.**

Spotify support is limited to **Windows GSMTC**, a first-party operating-system facility
that reports media metadata uniformly for *every* media application and is not part of the
Spotify Platform. By never becoming an SDA, LIQUID does not accept the Developer Terms.

**This is not treated as a loophole granting permission.** Per the project's own rule — OS
metadata availability is not permission for otherwise-prohibited uses — the following hold:

1. **No undocumented or private Spotify endpoints.** Ever. No scraping of the Spotify
   client's memory, local web helper, or protocol handlers.
2. **Audio analysis is of the Windows system mix, not of Spotify Content.** WASAPI loopback
   captures the mixed output of the default render endpoint. LIQUID cannot and does not
   isolate Spotify's stream. This limitation is stated in the UI, not hidden.
3. **A `spotify.audioReactiveVisuals` setting**, with a first-run disclosure naming III.6
   and III.13 and explaining the distinction above, so the user makes an informed choice
   rather than discovering it in a changelog.
4. **Lyrics are never sourced from Spotify.** They come from local files, user-authored
   files, or LRCLIB (§6) — keyed on title/artist/album/duration, which are facts about a
   recording, not Spotify Content.

**Unresolved, and stated as such:** whether a non-SDA desktop overlay analysing the system
audio mix is within the *spirit* of III.6/III.13 is a legal question, not an engineering
one. LIQUID's position is documented, user-visible and user-disableable. It is not presented
as legal clearance.

---

## 5. Graphics capability matrix

| Capability | Windows 10 19045 status | LIQUID posture |
|---|---|---|
| WebGL2 | **SUPPORTED** | Baseline renderer. All visualisers must work here. |
| WebGPU in WebView2 | **CONDITIONAL** | Opt-in only; probed at runtime, silently falls back to WebGL2. Never required. |
| `DwmEnableBlurBehindWindow` | SUPPORTED but legacy/limited on Win10 | Not used as the primary glass mechanism. |
| **Acrylic via `SetWindowCompositionAttribute`** | Undocumented private API | **NOT USED.** Private API, no stability guarantee. |
| `DwmSetWindowAttribute` + `DWMWA_SYSTEMBACKDROP_TYPE` (Mica/Acrylic) | **Windows 11 only (22621+)** | **UNAVAILABLE on this target.** Probed and skipped. |
| `DWMWA_WINDOW_CORNER_PREFERENCE` | **Windows 11 only** | **UNAVAILABLE.** Corner radius done in CSS/shader. |
| Per-monitor DPI v2 | SUPPORTED (1703+) | Used. |
| Genuine backdrop refraction (sampling pixels of windows behind) | **Not available to a transparent overlay** | See below. |

### The glass honesty note

A shader in a transparent overlay **cannot** sample the pixels of arbitrary windows behind
it. No API hands an overlay its own backdrop. The only ways to obtain those pixels are
desktop/window duplication capture — which costs real GPU bandwidth, trips protected-content
exclusions, and is an unacceptable thing to run continuously for a cosmetic effect.

**LIQUID therefore implements an approximation and labels it as one:** multi-layer
translucency, a procedurally generated frosted-noise substrate, simulated edge refraction at
panel borders, specular edge highlights, and album-art-driven tinting. It looks like glass.
It is not sampling your desktop, and the settings UI says so. Continuous screen capture for
cosmetic glass is explicitly rejected.

---

## 6. Lyrics provider matrix

| Source | Mechanism | Word-level timing | Status |
|---|---|---|---|
| Local `.lrc` | filesystem | if Enhanced LRC `<mm:ss.xx>` present | **SUPPORTED** |
| Enhanced LRC (word timestamps) | filesystem | yes | **SUPPORTED** |
| User-authored / editor output | built-in editor | yes | **SUPPORTED** |
| **LRCLIB** | `GET /api/get`, `GET /api/search` | `hasWordSync` flag; `lyricsfile` YAML carries word timings | **SUPPORTED, verified against `lrclib.net/docs.txt`** |

LRCLIB integration details, taken from its published docs:

- Clients are **required** to identify themselves via `User-Agent` with app name, version
  and a contact/homepage. LIQUID sends
  `LIQUID/<version> (+https://github.com/shura-mp3/liquid)`.
- Rate limits are approximately 45 req/min per IP generic, ~90 identified. LIQUID throttles
  to **one request per 400 ms**, serialised, and honours `429` + `Retry-After` with jitter.
- Match validation requires `duration` within **±2 s** of the record; LIQUID additionally
  scores title/artist/album and refuses to auto-apply a candidate below threshold — this is
  what prevents a remix/live/sped-up version's lyrics being silently applied.
- `instrumental: true` records are honoured as "no lyrics", not treated as a failure.
- The docs page states no explicit licence or attribution text; LIQUID therefore attributes
  LRCLIB visibly on every fetched lyric, and caches rather than re-requesting.

**Translation/romanization:** machine output is always labelled as machine-generated, and
nothing is sent to a third-party service without explicit per-provider opt-in.

---

## 7. Always-on-top: what works and what cannot

**Works reliably:** ordinary desktop windows, most borderless-windowed games and players,
File Explorer, browsers, Office.

**Does NOT work, and LIQUID says so rather than failing silently:**

| Surface | Why |
|---|---|
| Exclusive-fullscreen (DirectX FSE) applications | The app owns the swap chain; nothing composites above it. |
| Secure desktop / UAC prompts | Separate, isolated desktop by design. |
| Windows login / lock screen | Separate session. |
| Protected content (`SetWindowDisplayAffinity`, DRM video) | Deliberately excluded from composition/capture. |
| Some elevated windows | UIPI blocks a non-elevated process from interacting. |
| Anti-cheat-protected games | Overlays are frequently blocked by design. |

**Permanently out of scope:** no injection into other processes, no hooking of rendering
pipelines, no anti-cheat circumvention, no privilege escalation to force visibility. When
LIQUID detects an exclusive-fullscreen or protected surface it shows a compatibility notice
recommending borderless-windowed mode. That is the entire remedy.

---

## 8. Audio capture matrix

| Capability | Status | Note |
|---|---|---|
| WASAPI loopback on default render endpoint | **SUPPORTED** | Primary path. |
| Manual device selection | **SUPPORTED** | Enumerated via `IMMDeviceEnumerator`. |
| Device-change / format-change recovery | **SUPPORTED** | `IMMNotificationClient` + re-init. |
| **Per-process loopback** (`VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK` via `ActivateAudioInterfaceAsync`) | **CONDITIONAL — Windows 10 build 20348+ / Win11** | **Not available on 19045.** Probed; falls back to full-mix capture with a UI explanation. |
| Isolating one app's audio for analysis | **UNAVAILABLE on this target** | Direct consequence of the row above. The analysed signal is the *system mix* and may include Discord, notifications, game audio. Stated in the UI. |
| Microphone capture | **NOT IMPLEMENTED** | Not needed; excluded by design for privacy. |

---

## 9. Feature status summary

**SUPPORTED** (verified implementable on this target): Win32 transparent always-on-top
overlay, click-through with recovery, global hotkeys, tray, multi-monitor enumeration with
per-monitor DPI, GSMTC playback state, high-precision playback clock (QPC), LRC and
Enhanced-LRC parsing, lyrics matching, timing offsets, WASAPI loopback, FFT/DSP, WebGL2
visualisers, CSS/shader glass approximation, SQLite persistence, synthetic test provider.

**CONDITIONAL** (depends on source app, browser, or user action): per-provider metadata
richness, transport-control availability, browser-extension position fidelity, WebGPU,
translation/romanization providers, OBS alpha preservation.

**UNAVAILABLE on Windows 10 19045**: Mica/Acrylic system backdrop, DWM rounded corners,
per-process audio loopback, true backdrop refraction sampling, overlay above
exclusive-fullscreen / protected / secure surfaces.

**DELIBERATELY NOT IMPLEMENTED**: Spotify Web API/OAuth, private Spotify endpoints, DRM
circumvention, stream downloading, ad bypass, process injection, anti-cheat evasion,
continuous screen capture for cosmetic glass, telemetry.
