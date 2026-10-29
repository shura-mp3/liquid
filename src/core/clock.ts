/**
 * High-precision playback clock.
 *
 * The problem: GSMTC reports a position only when something changes, the value is
 * coarse, and it can be seconds stale. Animating lyrics directly from it is visibly
 * wrong. So we integrate a monotonic clock forward from the last trusted observation:
 *
 *   predicted = lastConfirmedPosition + (now - observedAt) * rate
 *
 * and classify each new observation so a seek resyncs instantly while ordinary
 * jitter is corrected smoothly. User offsets are applied by the *consumer*, never
 * folded into the stored position — otherwise changing an offset corrupts the basis.
 *
 * The clock also publishes an uncertainty bound. Pretending to know the position to
 * the millisecond after 30 s of silence from the source would be a lie, and the UI
 * uses this number to stop animating words confidently.
 */

import type {
  ClockReading,
  Discontinuity,
  PlaybackObservation,
  PlaybackStatus,
} from './types';

export interface ClockConfig {
  /** |observed - predicted| at or above this is a seek, not drift. */
  seekThresholdMs: number;
  /**
   * Add `granularity/2` to a truncated source's reported position.
   *
   * **Off by default, deliberately.** It is only correct when the truncation phase
   * is uniformly distributed. Measured against the synthetic timeline it improves
   * steady playback (mean 293 -> 208 ms) but roughly triples the error after a seek
   * to a round position, where the true phase is zero (117 -> 425 ms). A correction
   * that can be wrong by half a bucket is not worth applying blind; the detected
   * granularity is instead reported as honest *uncertainty*, which is always true.
   *
   * The real fix is phase estimation — polling faster than the bucket size to
   * observe exactly when the reported value increments. That is Phase 3 work.
   */
  compensateQuantisation: boolean;
  /** No observation for longer than this while playing marks the reading stale. */
  staleAfterMs: number;
  /** Maximum correction applied per second when slewing out small errors. */
  maxSlewMsPerSec: number;
  /** Uncertainty floor: even a fresh observation is only so good. */
  baseUncertaintyMs: number;
  /** Extra uncertainty accrued per second since the last observation. */
  uncertaintyGrowthMsPerSec: number;
  /** Uncertainty is clamped here; beyond it the reading is useless anyway. */
  maxUncertaintyMs: number;
}

export const DEFAULT_CLOCK_CONFIG: ClockConfig = {
  seekThresholdMs: 250,
  compensateQuantisation: false,
  staleAfterMs: 5000,
  maxSlewMsPerSec: 50,
  baseUncertaintyMs: 30,
  uncertaintyGrowthMsPerSec: 15,
  maxUncertaintyMs: 2000,
};

/**
 * Detects sources that report a *truncated* playback position.
 *
 * Several sources (notably browsers publishing whole-second positions) floor the
 * position to a coarse bucket. A clock that anchors to the reported value therefore
 * runs systematically late by half a bucket — about 500 ms for 1 s granularity,
 * which is a visibly wrong lyric line.
 *
 * The bias is not directly observable: our own prediction is anchored to those same
 * truncated values, so the measured error averages zero even while the whole
 * timeline is late. What *is* observable is the truncation itself — if every recent
 * reported position is a near-exact multiple of some granularity `g` while real
 * playback advanced smoothly across several buckets, the source is quantising. The
 * expected true position is then uniformly distributed across the bucket, so `g/2`
 * is the minimum-error estimate.
 *
 * Deliberately conservative: a source reporting precise positions fails the
 * multiple test and receives no correction at all. Guessing wrong here would make
 * accuracy worse, so detection requires several samples spanning several buckets.
 */
export class QuantisationDetector {
  /** Candidates from coarsest to finest; the coarsest consistent fit wins. */
  private static readonly CANDIDATES = [1000, 500, 250, 200, 100];
  private static readonly WINDOW = 8;
  private static readonly MIN_SAMPLES = 4;

  private samples: number[] = [];

  reset(): void {
    this.samples = [];
  }

  add(positionMs: number): void {
    this.samples.push(positionMs);
    if (this.samples.length > QuantisationDetector.WINDOW) this.samples.shift();
  }

  /** Detected granularity in ms, or 0 when the source is not quantising. */
  granularityMs(): number {
    const s = this.samples;
    if (s.length < QuantisationDetector.MIN_SAMPLES) return 0;

    for (const g of QuantisationDetector.CANDIDATES) {
      // Require the samples to span several buckets, otherwise a slow-moving
      // precise source trivially satisfies the multiple test.
      const span = Math.max(...s) - Math.min(...s);
      if (span < g * 2) continue;

      const tolerance = Math.min(50, g * 0.08);
      const allMultiples = s.every((p) => {
        const rem = ((p % g) + g) % g;
        return Math.min(rem, g - rem) <= tolerance;
      });
      if (allMultiples) return g;
    }
    return 0;
  }

  /** Correction to add to a reported position, in ms. */
  biasMs(): number {
    return this.granularityMs() / 2;
  }
}

interface ClockBasis {
  /** Position at the moment of the observation, after slew correction. */
  positionMs: number;
  /** Monotonic time of the observation. */
  atMs: number;
  rate: number;
  status: PlaybackStatus;
  trackKey: string;
}

export function trackKey(o: PlaybackObservation): string {
  const t = o.track;
  return [
    t.provider,
    t.providerTrackId ?? '',
    t.normalisedTitle,
    t.normalisedArtist,
    String(t.durationBucket),
  ].join('\u0001');
}

export class PlaybackClock {
  private basis: ClockBasis | null = null;
  /** Residual error we are slewing out, in ms. Signed. */
  private pendingCorrectionMs = 0;
  private lastClassification: Discontinuity = 'first-observation';
  private readonly detector = new QuantisationDetector();
  /** Bias currently folded into the basis, so a change can be applied as a delta. */
  private appliedBiasMs = 0;

  constructor(private readonly config: ClockConfig = DEFAULT_CLOCK_CONFIG) {}

  get classification(): Discontinuity {
    return this.lastClassification;
  }

  /**
   * Detected source quantisation in ms, 0 when none. Always measured, regardless of
   * whether compensation is enabled: the diagnostics panel uses it to tell the user
   * "this source only reports whole seconds", which is the honest explanation for
   * why sync cannot be tighter with that source.
   */
  get detectedGranularityMs(): number {
    return this.detector.granularityMs();
  }

  /** True once at least one observation has been accepted. */
  get hasBasis(): boolean {
    return this.basis !== null;
  }

  reset(): void {
    this.basis = null;
    this.pendingCorrectionMs = 0;
    this.lastClassification = 'first-observation';
    this.detector.reset();
    this.appliedBiasMs = 0;
  }

  /**
   * Feed a fresh observation. Returns how it was classified, which the caller uses
   * to decide whether to drop lyrics (track change) or re-render immediately (seek).
   */
  observe(o: PlaybackObservation): Discontinuity {
    const key = trackKey(o);
    const prev = this.basis;

    if (prev !== null && prev.trackKey !== key) {
      // Positions restart on a new track, so the quantisation evidence is void.
      this.detector.reset();
      this.appliedBiasMs = 0;
      this.setBasis(o, key, o.positionMs);
      this.lastClassification = 'track-change';
      return this.lastClassification;
    }

    // The detector must see the raw reported value, never a corrected one.
    this.detector.add(o.positionMs);
    const bias = this.config.compensateQuantisation ? this.detector.biasMs() : 0;
    const observedPosition = o.positionMs + bias;

    if (prev === null) {
      this.appliedBiasMs = bias;
      this.setBasis(o, key, observedPosition);
      this.lastClassification = 'first-observation';
      return this.lastClassification;
    }

    // Detection typically kicks in a few reports in. Shift the existing basis by
    // the delta rather than letting a bias change masquerade as a seek.
    if (bias !== this.appliedBiasMs) {
      prev.positionMs += bias - this.appliedBiasMs;
      this.appliedBiasMs = bias;
    }

    if (prev.status !== o.status) {
      this.setBasis(o, key, observedPosition);
      this.lastClassification = 'status-change';
      return this.lastClassification;
    }

    if (prev.rate !== o.rate) {
      this.setBasis(o, key, observedPosition);
      this.lastClassification = 'rate-change';
      return this.lastClassification;
    }

    const predicted = this.rawPredict(o.observedAtMs);
    const error = observedPosition - predicted;

    if (Math.abs(error) >= this.config.seekThresholdMs) {
      // A real discontinuity. Snap; do not slew a seek, that looks broken.
      this.setBasis(o, key, observedPosition);
      this.lastClassification = 'seek';
      return this.lastClassification;
    }

    // Small error: keep the existing basis and bleed the correction out gradually
    // so lyrics never visibly jump. Re-anchoring the basis time keeps uncertainty
    // low, since we genuinely did just hear from the source.
    this.basis = {
      positionMs: predicted,
      atMs: o.observedAtMs,
      rate: o.rate,
      status: o.status,
      trackKey: key,
    };
    this.pendingCorrectionMs = error;
    this.lastClassification = 'normal';
    return this.lastClassification;
  }

  /** Position with no slew applied — the pure integration of the basis. */
  private rawPredict(nowMs: number): number {
    const b = this.basis;
    if (b === null) return 0;
    if (b.status !== 'playing') return b.positionMs;
    const elapsed = Math.max(0, nowMs - b.atMs);
    return b.positionMs + elapsed * b.rate;
  }

  /**
   * Current reading. `nowMs` must come from the same monotonic source as
   * `observation.observedAtMs` — mixing wall-clock and monotonic time here is the
   * mistake that makes lyrics drift after a system sleep.
   */
  read(nowMs: number): ClockReading {
    const b = this.basis;
    if (b === null) {
      return {
        positionMs: 0,
        uncertaintyMs: this.config.maxUncertaintyMs,
        status: 'unknown',
        rate: 1,
        isStale: true,
      };
    }

    const ageMs = Math.max(0, nowMs - b.atMs);
    let position = this.rawPredict(nowMs);

    // Apply at most maxSlewMsPerSec of the pending correction, proportional to age.
    if (this.pendingCorrectionMs !== 0) {
      const budget = (this.config.maxSlewMsPerSec * ageMs) / 1000;
      const applied =
        Math.sign(this.pendingCorrectionMs) *
        Math.min(Math.abs(this.pendingCorrectionMs), budget);
      position += applied;
    }

    const isStale = b.status === 'playing' && ageMs > this.config.staleAfterMs;

    // Uncertainty grows only while playing: a paused position does not rot.
    const growth =
      b.status === 'playing' ? (this.config.uncertaintyGrowthMsPerSec * ageMs) / 1000 : 0;
    // A truncating source is uncertain by half a bucket whether or not we try to
    // compensate: correcting shifts the error, it does not remove it.
    const quantisationMs = this.detector.granularityMs() / 2;

    const uncertaintyMs = Math.min(
      this.config.maxUncertaintyMs,
      this.config.baseUncertaintyMs +
        growth +
        quantisationMs +
        Math.abs(this.pendingCorrectionMs),
    );

    return {
      positionMs: Math.max(0, position),
      uncertaintyMs,
      status: b.status,
      rate: b.rate,
      isStale,
    };
  }

  private setBasis(o: PlaybackObservation, key: string, positionMs: number): void {
    this.basis = {
      positionMs,
      atMs: o.observedAtMs,
      rate: o.rate,
      status: o.status,
      trackKey: key,
    };
    this.pendingCorrectionMs = 0;
  }
}

/**
 * Effective lyric time = clock position shifted by every offset that applies.
 * Kept separate from the clock so changing an offset never disturbs the basis,
 * and so the same reading can drive two layouts with different offsets.
 */
export function lyricTimeMs(
  reading: ClockReading,
  offsets: { perTrackMs?: number; globalMs?: number; fileMs?: number } = {},
): number {
  return (
    reading.positionMs +
    (offsets.perTrackMs ?? 0) +
    (offsets.globalMs ?? 0) +
    (offsets.fileMs ?? 0)
  );
}

/** Above this uncertainty, word-level highlighting is shown de-emphasised. */
export const WORD_SYNC_UNCERTAINTY_LIMIT_MS = 120;

export function shouldTrustWordSync(reading: ClockReading): boolean {
  return !reading.isStale && reading.uncertaintyMs <= WORD_SYNC_UNCERTAINTY_LIMIT_MS;
}
