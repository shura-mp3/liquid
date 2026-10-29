import { describe, expect, it } from 'vitest';
import {
  DEFAULT_CLOCK_CONFIG,
  PlaybackClock,
  lyricTimeMs,
  shouldTrustWordSync,
} from '../src/core/clock';
import { makeIdentity } from '../src/core/identity';
import type { PlaybackObservation } from '../src/core/types';

function obs(over: Partial<PlaybackObservation> = {}): PlaybackObservation {
  return {
    track: makeIdentity({
      provider: 'synthetic',
      title: 'Track',
      artist: 'Artist',
      durationMs: 200_000,
    }),
    title: 'Track',
    artist: 'Artist',
    durationMs: 200_000,
    positionMs: 0,
    status: 'playing',
    rate: 1,
    observedAtMs: 0,
    timingConfidence: 'coarse',
    controls: {
      play: true,
      pause: true,
      next: true,
      previous: true,
      seek: true,
      volume: false,
    },
    provenance: 'synthetic',
    ...over,
  };
}

describe('PlaybackClock — interpolation', () => {
  it('reports zero with maximum uncertainty before any observation', () => {
    const c = new PlaybackClock();
    const r = c.read(1_000);
    expect(r.positionMs).toBe(0);
    expect(r.uncertaintyMs).toBe(DEFAULT_CLOCK_CONFIG.maxUncertaintyMs);
    expect(r.isStale).toBe(true);
    expect(c.hasBasis).toBe(false);
  });

  it('integrates forward from the basis at rate 1', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 10_000, observedAtMs: 1_000 }));
    expect(c.read(1_000).positionMs).toBeCloseTo(10_000, 3);
    expect(c.read(2_000).positionMs).toBeCloseTo(11_000, 3);
    expect(c.read(3_500).positionMs).toBeCloseTo(12_500, 3);
  });

  it('scales interpolation by playback rate', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0, rate: 1.5 }));
    expect(c.read(2_000).positionMs).toBeCloseTo(3_000, 3);
  });

  it('does not advance while paused', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 42_000, observedAtMs: 0, status: 'paused' }));
    expect(c.read(10_000).positionMs).toBeCloseTo(42_000, 3);
    expect(c.read(60_000).positionMs).toBeCloseTo(42_000, 3);
  });

  it('never returns a negative position', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 5_000 }));
    expect(c.read(0).positionMs).toBeGreaterThanOrEqual(0);
  });
});

describe('PlaybackClock — discontinuity classification', () => {
  it('classifies the first observation', () => {
    const c = new PlaybackClock();
    expect(c.observe(obs())).toBe('first-observation');
  });

  it('classifies small error as normal drift', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    // 1 s later the source says 1.1 s: 100 ms of ordinary jitter.
    expect(c.observe(obs({ positionMs: 1_100, observedAtMs: 1_000 }))).toBe('normal');
  });

  it('classifies a large jump as a seek', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    expect(c.observe(obs({ positionMs: 90_000, observedAtMs: 1_000 }))).toBe('seek');
  });

  it('detects a backwards seek', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 100_000, observedAtMs: 0 }));
    expect(c.observe(obs({ positionMs: 5_000, observedAtMs: 1_000 }))).toBe('seek');
  });

  it('snaps immediately on a seek rather than slewing', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    c.observe(obs({ positionMs: 90_000, observedAtMs: 1_000 }));
    expect(c.read(1_000).positionMs).toBeCloseTo(90_000, 3);
  });

  it('classifies a track change by identity, not by position', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 100_000, observedAtMs: 0 }));
    const next = obs({
      title: 'Different Song',
      positionMs: 100_100,
      observedAtMs: 1_000,
      track: makeIdentity({
        provider: 'synthetic',
        title: 'Different Song',
        artist: 'Artist',
        durationMs: 200_000,
      }),
    });
    expect(c.observe(next)).toBe('track-change');
  });

  it('classifies a pause and a resume as status changes', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    expect(c.observe(obs({ positionMs: 1_000, observedAtMs: 1_000, status: 'paused' }))).toBe(
      'status-change',
    );
    expect(c.observe(obs({ positionMs: 1_000, observedAtMs: 9_000 }))).toBe('status-change');
  });

  it('classifies a rate change', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    expect(c.observe(obs({ positionMs: 1_000, observedAtMs: 1_000, rate: 2 }))).toBe(
      'rate-change',
    );
  });

  it('resumes from the paused position, not from wall-clock elapsed time', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 30_000, observedAtMs: 0, status: 'paused' }));
    // Paused for a minute, then resumed.
    c.observe(obs({ positionMs: 30_000, observedAtMs: 60_000, status: 'playing' }));
    expect(c.read(61_000).positionMs).toBeCloseTo(31_000, 3);
  });
});

describe('PlaybackClock — drift correction', () => {
  it('slews out a small error gradually instead of jumping', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    // Source reports 200 ms ahead of prediction: under the seek threshold.
    c.observe(obs({ positionMs: 1_200, observedAtMs: 1_000 }));

    const immediately = c.read(1_000).positionMs;
    // Not snapped to 1200 — correction has not been applied yet.
    expect(immediately).toBeCloseTo(1_000, 1);

    // After 1 s, at most maxSlewMsPerSec of the 200 ms has been applied.
    const afterOneSecond = c.read(2_000).positionMs;
    const corrected = afterOneSecond - 2_000;
    expect(corrected).toBeGreaterThan(0);
    expect(corrected).toBeLessThanOrEqual(DEFAULT_CLOCK_CONFIG.maxSlewMsPerSec + 0.001);
  });

  it('does not over-correct beyond the measured error', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    c.observe(obs({ positionMs: 1_050, observedAtMs: 1_000 }));
    // 50 ms error, 10 s of slew budget: must stop at 50 ms, not keep going.
    const corrected = c.read(11_000).positionMs - 11_000;
    expect(corrected).toBeCloseTo(50, 1);
  });
});

describe('PlaybackClock — uncertainty and staleness', () => {
  it('grows uncertainty with the age of the observation', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    const fresh = c.read(0).uncertaintyMs;
    const old = c.read(10_000).uncertaintyMs;
    expect(fresh).toBe(DEFAULT_CLOCK_CONFIG.baseUncertaintyMs);
    expect(old).toBeGreaterThan(fresh);
  });

  it('does not grow uncertainty while paused — a paused position does not rot', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 5_000, observedAtMs: 0, status: 'paused' }));
    expect(c.read(600_000).uncertaintyMs).toBe(DEFAULT_CLOCK_CONFIG.baseUncertaintyMs);
  });

  it('clamps uncertainty to the configured maximum', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    expect(c.read(10_000_000).uncertaintyMs).toBe(DEFAULT_CLOCK_CONFIG.maxUncertaintyMs);
  });

  it('marks a reading stale when the source has gone quiet while playing', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    expect(c.read(1_000).isStale).toBe(false);
    expect(c.read(DEFAULT_CLOCK_CONFIG.staleAfterMs + 1).isStale).toBe(true);
  });

  it('does not mark a paused reading stale', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0, status: 'paused' }));
    expect(c.read(600_000).isStale).toBe(false);
  });

  it('refuses to trust word sync once uncertainty is high', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 0, observedAtMs: 0 }));
    expect(shouldTrustWordSync(c.read(0))).toBe(true);
    expect(shouldTrustWordSync(c.read(30_000))).toBe(false);
  });
});

describe('lyricTimeMs', () => {
  const reading = { positionMs: 10_000, uncertaintyMs: 30, status: 'playing' as const, rate: 1, isStale: false };

  it('returns the raw position with no offsets', () => {
    expect(lyricTimeMs(reading)).toBe(10_000);
  });

  it('sums per-track, global and file offsets', () => {
    expect(lyricTimeMs(reading, { perTrackMs: 100, globalMs: -50, fileMs: 25 })).toBe(10_075);
  });

  it('leaves the clock basis untouched so offsets stay adjustable', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 10_000, observedAtMs: 0 }));
    const r = c.read(0);
    lyricTimeMs(r, { perTrackMs: 5_000 });
    expect(c.read(0).positionMs).toBeCloseTo(10_000, 3);
  });
});

describe('PlaybackClock — reset', () => {
  it('clears the basis', () => {
    const c = new PlaybackClock();
    c.observe(obs({ positionMs: 10_000, observedAtMs: 0 }));
    c.reset();
    expect(c.hasBasis).toBe(false);
    expect(c.read(5_000).positionMs).toBe(0);
  });
});
