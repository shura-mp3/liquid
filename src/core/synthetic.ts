/**
 * Synthetic playback provider.
 *
 * The backbone of the timing test suite. It produces a deterministic, scriptable
 * playback timeline — including the nasty parts: seeks, pauses, rate changes, coarse
 * position reporting, and long gaps with no observation at all — so lyric sync
 * accuracy can be measured against a known ground truth.
 *
 * Streaming services are never a test dependency: they are not reproducible, and
 * testing against them would make the suite a network liveness check.
 */

import { makeIdentity } from './identity';
import type { PlaybackObservation, PlaybackStatus } from './types';

export interface SyntheticEvent {
  /** Monotonic time at which this happens, ms since timeline start. */
  atMs: number;
  kind: 'seek' | 'pause' | 'resume' | 'rate' | 'track';
  /** Target position for a seek, new rate for a rate change. */
  value?: number;
  title?: string;
  durationMs?: number;
}

export interface SyntheticConfig {
  title: string;
  artist: string;
  durationMs: number;
  /**
   * Position at timeline start. Deliberately NOT a whole number of seconds: with
   * `startPositionMs: 0` and 1 s reports at rate 1.0, every report lands exactly on
   * a second boundary and `positionGranularityMs` quantisation becomes a no-op,
   * which flatters the measured accuracy. Real playback never starts aligned.
   */
  startPositionMs: number;
  /** How often the source reports a position, ms. GSMTC is typically coarse. */
  reportIntervalMs: number;
  /** Reported positions are rounded to this granularity. GSMTC often gives ~1 s. */
  positionGranularityMs: number;
  /** Deterministic error added to each report, ms. Signed, applied alternately. */
  reportJitterMs: number;
  events: SyntheticEvent[];
}

export const DEFAULT_SYNTHETIC: SyntheticConfig = {
  title: 'Deterministic Test Track',
  artist: 'LIQUID Test Suite',
  durationMs: 240_000,
  startPositionMs: 7_321,
  reportIntervalMs: 1000,
  positionGranularityMs: 1000,
  reportJitterMs: 40,
  events: [],
};

/** Ground truth: the true playback position at a given monotonic time. */
export class SyntheticTimeline {
  private readonly cfg: SyntheticConfig;
  private reportCount = 0;

  constructor(config: Partial<SyntheticConfig> = {}) {
    this.cfg = { ...DEFAULT_SYNTHETIC, ...config };
    this.cfg.events = [...this.cfg.events].sort((a, b) => a.atMs - b.atMs);
  }

  get config(): Readonly<SyntheticConfig> {
    return this.cfg;
  }

  /**
   * The exact position the track is really at, by integrating the event list.
   * This is what a perfect clock would report.
   */
  truePositionMs(nowMs: number): number {
    let position = this.cfg.startPositionMs;
    let lastAt = 0;
    let rate = 1;
    let status: PlaybackStatus = 'playing';

    for (const ev of this.cfg.events) {
      if (ev.atMs > nowMs) break;
      if (status === 'playing') position += (ev.atMs - lastAt) * rate;
      lastAt = ev.atMs;

      switch (ev.kind) {
        case 'seek':
          position = ev.value ?? 0;
          break;
        case 'pause':
          status = 'paused';
          break;
        case 'resume':
          status = 'playing';
          break;
        case 'rate':
          rate = ev.value ?? 1;
          break;
        case 'track':
          position = 0;
          break;
      }
    }

    if (status === 'playing') position += (nowMs - lastAt) * rate;
    return Math.max(0, position);
  }

  statusAt(nowMs: number): PlaybackStatus {
    let status: PlaybackStatus = 'playing';
    for (const ev of this.cfg.events) {
      if (ev.atMs > nowMs) break;
      if (ev.kind === 'pause') status = 'paused';
      if (ev.kind === 'resume') status = 'playing';
    }
    return status;
  }

  rateAt(nowMs: number): number {
    let rate = 1;
    for (const ev of this.cfg.events) {
      if (ev.atMs > nowMs) break;
      if (ev.kind === 'rate') rate = ev.value ?? 1;
    }
    return rate;
  }

  titleAt(nowMs: number): string {
    let title = this.cfg.title;
    for (const ev of this.cfg.events) {
      if (ev.atMs > nowMs) break;
      if (ev.kind === 'track' && ev.title) title = ev.title;
    }
    return title;
  }

  /**
   * What the *source* would report at this instant: quantised, jittered and
   * therefore wrong in exactly the way real sources are wrong.
   */
  observe(nowMs: number): PlaybackObservation {
    const truth = this.truePositionMs(nowMs);
    const g = this.cfg.positionGranularityMs;
    const quantised = g > 0 ? Math.floor(truth / g) * g : truth;
    // Alternating sign keeps the error deterministic and zero-mean.
    const jitter = this.cfg.reportJitterMs * (this.reportCount % 2 === 0 ? 1 : -1);
    this.reportCount += 1;

    const title = this.titleAt(nowMs);
    return {
      track: makeIdentity({
        provider: 'synthetic',
        title,
        artist: this.cfg.artist,
        durationMs: this.cfg.durationMs,
      }),
      title,
      artist: this.cfg.artist,
      album: 'Fixtures',
      durationMs: this.cfg.durationMs,
      positionMs: Math.max(0, quantised + jitter),
      status: this.statusAt(nowMs),
      rate: this.rateAt(nowMs),
      observedAtMs: nowMs,
      timingConfidence: 'coarse',
      controls: {
        play: true,
        pause: true,
        next: true,
        previous: true,
        seek: true,
        volume: false,
      },
      sourceApp: 'liquid.tests.synthetic',
      provenance: 'synthetic',
    };
  }

  /** Monotonic times at which the source emits a report, including event instants. */
  reportTimes(untilMs: number): number[] {
    const times: number[] = [];
    for (let t = 0; t <= untilMs; t += this.cfg.reportIntervalMs) times.push(t);
    // Real sources push an immediate update when the user seeks or pauses.
    for (const ev of this.cfg.events) {
      if (ev.atMs <= untilMs) times.push(ev.atMs, ev.atMs + 50);
    }
    return [...new Set(times)].sort((a, b) => a - b);
  }
}
