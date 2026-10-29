/**
 * Typed IPC client for the Rust commands.
 *
 * One module owns every `invoke` call so the command names and payload shapes exist
 * in exactly one place. These types mirror the serde definitions in
 * `src-tauri/src/commands.rs` and `src-tauri/src/storage/settings.rs`.
 */

import { invoke } from '@tauri-apps/api/core';

export interface MonitorInfo {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  work_x: number;
  work_y: number;
  work_width: number;
  work_height: number;
  is_primary: boolean;
  scale_factor: number;
}

export interface GlassSettings {
  transparency: number;
  blur: number;
  frost: number;
  tintStrength: number;
  saturation: number;
  edgeGlow: number;
  reflection: number;
  borderWidth: number;
  cornerRadius: number;
  rippleStrength: number;
  animationSpeed: number;
  shadowSoftness: number;
}

export interface Settings {
  theme: string;
  performancePreset: string;
  fpsCap: number;
  clickThrough: boolean;
  overlayVisible: boolean;
  launchAtLogin: boolean;
  startMinimised: boolean;
  globalOffsetMs: number;
  spotifyAudioReactiveVisuals: boolean;
  historyEnabled: boolean;
  glass: GlassSettings;
  hotkeys: Record<string, string>;
}

export type OverlayObstruction = 'none' | 'suspected-exclusive-fullscreen';

export interface OverlayStatus {
  label: string;
  topmost: boolean;
  clickThrough: boolean;
  visible: boolean;
  /**
   * Whether the required extended-style set is currently present. tao rewrites the
   * whole style word when it re-applies its own window flags, so this is surfaced
   * rather than assumed.
   */
  stylesOk: boolean;
  obstruction: OverlayObstruction;
}

// ---------------------------------------------------------------------------
// Media sessions
// ---------------------------------------------------------------------------

export type MediaPlaybackStatus = 'playing' | 'paused' | 'stopped' | 'unknown';
export type MediaTimingConfidence = 'exact' | 'interpolated' | 'coarse' | 'unknown';
export type MediaProvenance = 'gsmtc' | 'extension' | 'synthetic' | 'user';

export interface MediaControlCapabilities {
  play: boolean;
  pause: boolean;
  next: boolean;
  previous: boolean;
  seek: boolean;
  volume: boolean;
}

/**
 * Raw playback state from the native side. Carries no track identity on purpose —
 * see the note in `src-tauri/src/media/mod.rs`; identity is derived here with the
 * single tested implementation in `core/identity.ts`.
 */
export interface PlaybackSnapshot {
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  positionMs: number;
  status: MediaPlaybackStatus;
  rate: number;
  /** Native clock base; not comparable with `performance.now()`. Use `ageMs`. */
  observedAtMs: number;
  /** How long before the snapshot was produced the position was sampled. */
  ageMs: number;
  timingConfidence: MediaTimingConfidence;
  controls: MediaControlCapabilities;
  sourceApp: string | null;
  provenance: MediaProvenance;
  artworkKey: string | null;
}

export interface SessionSummary {
  sourceApp: string;
  displayName: string;
  title: string | null;
  artist: string | null;
  status: MediaPlaybackStatus;
  isCurrent: boolean;
  isPinned: boolean;
}

export interface MediaStatus {
  bridgeRunning: boolean;
  error: string | null;
  snapshot: PlaybackSnapshot | null;
  sessions: SessionSummary[];
  pinned: string | null;
}

export type TransportAction = 'play' | 'pause' | 'play-pause' | 'next' | 'previous';

/** Payload of the `liquid://playback` event. */
export interface PlaybackEvent {
  snapshot: PlaybackSnapshot | null;
  sessions: SessionSummary[];
}

// ---------------------------------------------------------------------------
// Lyrics
// ---------------------------------------------------------------------------

export type LyricsOrigin = 'cache' | 'lrclib' | 'local-file' | 'user-edited';

/** Mirrors `LyricsCandidate` in core/matcher.ts, so no translation layer is needed. */
export interface LyricsCandidate {
  id: number;
  trackName: string;
  artistName: string;
  albumName?: string;
  duration?: number;
  instrumental: boolean;
  hasWordSync: boolean;
  syncedLyrics?: string;
  plainLyrics?: string;
}

export interface ResolvedLyrics {
  /** Raw LRC, parsed here rather than natively — one parser, exhaustively tested. */
  text: string;
  origin: LyricsOrigin;
  instrumental: boolean;
  providerId?: number;
  attribution: string | null;
}

export interface LyricsLookupResult {
  resolved: ResolvedLyrics | null;
  /** Previously checked and genuinely none — not the same as "none found today". */
  knownAbsent: boolean;
  candidates: LyricsCandidate[];
  /** Provider unreachable. Reported as itself, never disguised as "no lyrics". */
  providerError: string | null;
}

export interface LyricsQuery {
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
}

export interface Diagnostics {
  appVersion: string;
  monotonicClockDegraded: boolean;
  monotonicNowMs: number;
  monitorCount: number;
  launchesAtLogin: boolean;
  audioCapture: string;
  mediaSession: string;
  artworkCached: boolean;
  artworkCacheEntries: number;
  lyricsCacheEntries: number;
}

export const ipc = {
  getMonitors: () => invoke<MonitorInfo[]>('get_monitors'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  resetSettings: () => invoke<Settings>('reset_settings'),
  setClickThrough: (enabled: boolean) => invoke<boolean>('set_click_through', { enabled }),
  toggleClickThrough: () => invoke<boolean>('toggle_click_through'),
  restoreInteractivity: () => invoke<void>('restore_interactivity'),
  setOverlayVisible: (visible: boolean) => invoke<void>('set_overlay_visible', { visible }),
  assertTopmost: () => invoke<void>('assert_topmost'),
  overlayStatus: () => invoke<OverlayStatus[]>('overlay_status'),
  getDiagnostics: () => invoke<Diagnostics>('get_diagnostics'),
  setLaunchAtLogin: (enabled: boolean) => invoke<void>('set_launch_at_login', { enabled }),
  showSettingsWindow: () => invoke<void>('show_settings_window'),
  /**
   * Tell the native side the page actually loaded. Window creation succeeding does
   * not mean the webview rendered anything, so this is what proves it.
   */
  reportFrontendReady: (route: string, href: string) =>
    invoke<void>('report_frontend_ready', { route, href }),

  getMediaStatus: () => invoke<MediaStatus>('get_media_status'),
  setPinnedSource: (sourceApp: string | null) =>
    invoke<void>('set_pinned_source', { sourceApp }),
  mediaTransport: (action: TransportAction) =>
    invoke<void>('media_transport', { action }),
  mediaSeek: (positionMs: number) => invoke<void>('media_seek', { positionMs }),
  getArtwork: (key: string) => invoke<string | null>('get_artwork', { key }),
  refreshMediaSessions: () => invoke<void>('refresh_media_sessions'),

  lyricsLookup: (query: LyricsQuery, offsetKey: string) =>
    invoke<LyricsLookupResult>('lyrics_lookup', { query, offsetKey }),
  lyricsAccept: (offsetKey: string, candidate: LyricsCandidate) =>
    invoke<ResolvedLyrics>('lyrics_accept', { offsetKey, candidate }),
  lyricsAcceptAbsent: (offsetKey: string) =>
    invoke<void>('lyrics_accept_absent', { offsetKey }),
  lyricsImportFile: (offsetKey: string, path: string) =>
    invoke<ResolvedLyrics>('lyrics_import_file', { offsetKey, path }),
  lyricsSaveEdited: (offsetKey: string, text: string) =>
    invoke<ResolvedLyrics>('lyrics_save_edited', { offsetKey, text }),
  lyricsForget: (offsetKey: string) => invoke<void>('lyrics_forget', { offsetKey }),
  getLyricOffset: (offsetKey: string) => invoke<number>('get_lyric_offset', { offsetKey }),
  setLyricOffset: (offsetKey: string, offsetMs: number) =>
    invoke<void>('set_lyric_offset', { offsetKey, offsetMs }),
};

/**
 * Push glass settings into CSS custom properties so slider changes apply instantly
 * without a reload. The CSS consumes these; no component reads them directly.
 */
export function applyGlassToDom(glass: GlassSettings, theme: string): void {
  const root = document.documentElement;
  root.dataset['theme'] = theme;
  root.style.setProperty('--lq-glass-alpha', String(glass.transparency));
  root.style.setProperty('--lq-glass-blur', `${(glass.blur * 34).toFixed(1)}px`);
  root.style.setProperty('--lq-glass-frost', String(glass.frost));
  root.style.setProperty('--lq-glass-saturation', String(glass.saturation));
  root.style.setProperty('--lq-glass-tint-strength', String(glass.tintStrength));
  root.style.setProperty('--lq-glass-edge-glow', String(glass.edgeGlow));
  root.style.setProperty('--lq-glass-reflection', String(glass.reflection));
  root.style.setProperty('--lq-glass-border-width', `${glass.borderWidth.toFixed(2)}px`);
  root.style.setProperty('--lq-glass-radius', `${glass.cornerRadius.toFixed(0)}px`);
  root.style.setProperty('--lq-glass-shadow-softness', String(glass.shadowSoftness));
  // Guard against zero: it would make every duration division infinite.
  root.style.setProperty('--lq-anim-speed', String(Math.max(0.1, glass.animationSpeed)));
}
