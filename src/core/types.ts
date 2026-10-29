/**
 * Shared domain types. These mirror the Rust definitions in `src-tauri/src/`;
 * both sides are tested against the same fixtures so the IPC boundary stays honest.
 */

export type ProviderId =
  | 'gsmtc'
  | 'extension-youtube'
  | 'extension-soundcloud'
  | 'synthetic';

export type PlaybackStatus = 'playing' | 'paused' | 'stopped' | 'unknown';

/**
 * How much the reported position can be trusted at the moment it was observed.
 * This is about the *source*, not about our own prediction drift.
 */
export type TimingConfidence = 'exact' | 'interpolated' | 'coarse' | 'unknown';

export type Provenance = 'gsmtc' | 'extension' | 'synthetic' | 'user';

/** Controls the source has actually advertised. Never assumed. */
export interface ControlCapabilities {
  play: boolean;
  pause: boolean;
  next: boolean;
  previous: boolean;
  seek: boolean;
  volume: boolean;
}

export const NO_CONTROLS: ControlCapabilities = {
  play: false,
  pause: false,
  next: false,
  previous: false,
  seek: false,
  volume: false,
};

/**
 * Composite track identity. Deliberately NOT the title alone: per-track lyric
 * offsets must survive restarts without bleeding across different recordings
 * that happen to share a name.
 */
export interface TrackIdentity {
  provider: ProviderId;
  /** Provider-native id when the source exposes one. Most do not. */
  providerTrackId?: string;
  normalisedTitle: string;
  normalisedArtist: string;
  /** Duration rounded into a bucket so jitter in reported duration does not fork identity. */
  durationBucket: number;
}

export interface PlaybackObservation {
  track: TrackIdentity;
  title: string;
  artist?: string;
  album?: string;
  durationMs?: number;
  /** Position as reported by the source, in ms. */
  positionMs: number;
  status: PlaybackStatus;
  rate: number;
  /** Monotonic timestamp (ms) at which this observation was taken. */
  observedAtMs: number;
  timingConfidence: TimingConfidence;
  controls: ControlCapabilities;
  sourceApp?: string;
  provenance: Provenance;
}

/** What the clock publishes to the UI every tick. */
export interface ClockReading {
  positionMs: number;
  /** Growing error bound. Word-level highlighting is de-emphasised above a threshold. */
  uncertaintyMs: number;
  status: PlaybackStatus;
  rate: number;
  /** True when the last observation is too old to animate from confidently. */
  isStale: boolean;
}

export type Discontinuity =
  | 'normal'
  | 'seek'
  | 'track-change'
  | 'status-change'
  | 'rate-change'
  | 'first-observation';

// ---------------------------------------------------------------------------
// Lyrics
// ---------------------------------------------------------------------------

export interface LyricWord {
  /** Start time in ms, relative to track start. */
  timeMs: number;
  text: string;
}

export interface LyricLine {
  timeMs: number;
  text: string;
  /** Present only when the source genuinely carried word timings. */
  words?: LyricWord[];
}

export type LyricsTimingKind =
  /** Word-level timestamps present: real karaoke. */
  | 'word'
  /** Line timestamps only: highlighting is per-line, word progress is NOT faked. */
  | 'line'
  /** No timestamps at all. */
  | 'plain';

export interface ParsedLyrics {
  lines: LyricLine[];
  timingKind: LyricsTimingKind;
  /** `[ti:]`, `[ar:]`, `[al:]`, `[length:]`, `[offset:]` and friends. */
  tags: Record<string, string>;
  /** `[offset:]` from the file, in ms. Positive shifts lyrics later. */
  fileOffsetMs: number;
  /** Lines that could not be parsed, kept for the editor to surface. */
  warnings: string[];
}
