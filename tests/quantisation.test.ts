import { describe, expect, it } from 'vitest';
import {
  DEFAULT_CLOCK_CONFIG,
  PlaybackClock,
  QuantisationDetector,
  shouldTrustWordSync,
} from '../src/core/clock';
import { makeIdentity } from '../src/core/identity';
import { SyntheticTimeline } from '../src/core/synthetic';
import type { PlaybackObservation } from '../src/core/types';

function obs(positionMs: number, observedAtMs: number): PlaybackObservation {
  return {
    track: makeIdentity({ provider: 'synthetic', title: 'T', artist: 'A', durationMs: 200_000 }),
    title: 'T',
    artist: 'A',
    durationMs: 200_000,
    positionMs,
    status: 'playing',
    rate: 1,
    observedAtMs,
    timingConfidence: 'coarse',
    controls: { play: true, pause: true, next: true, previous: true, seek: true, volume: false },
    provenance: 'synthetic',
  };
}

describe('QuantisationDetector', () => {
  it('detects a source reporting whole seconds', () => {
    const d = new QuantisationDetector();
    for (const p of [10_000, 11_000, 12_000, 13_000, 14_000]) d.add(p);
    expect(d.granularityMs()).toBe(1000);
    expect(d.biasMs()).toBe(500);
  });

  it('tolerates small reporting noise around the bucket', () => {
    const d = new QuantisationDetector();
    for (const p of [10_040, 10_960, 12_030, 12_970, 14_010]) d.add(p);
    expect(d.granularityMs()).toBe(1000);
  });

  it('reports no quantisation for a precise source', () => {
    const d = new QuantisationDetector();
    for (const p of [10_123, 11_456, 12_789, 13_222, 14_655]) d.add(p);
    expect(d.granularityMs()).toBe(0);
    expect(d.biasMs()).toBe(0);
  });

  it('detects coarser 500 ms granularity', () => {
    const d = new QuantisationDetector();
    for (const p of [10_000, 10_500, 11_000, 11_500, 12_000]) d.add(p);
    // 1000 fails (10_500 is not a multiple), so 500 is the coarsest fit.
    expect(d.granularityMs()).toBe(500);
  });

  it('refuses to decide from too few samples', () => {
    const d = new QuantisationDetector();
    d.add(10_000);
    d.add(11_000);
    expect(d.granularityMs()).toBe(0);
  });

  it('refuses to decide when the samples barely move', () => {
    // A precise but slow-moving source would trivially satisfy a multiple test.
    const d = new QuantisationDetector();
    for (const p of [10_000, 10_000, 10_000, 10_000]) d.add(p);
    expect(d.granularityMs()).toBe(0);
  });

  it('forgets old evidence outside its window', () => {
    const d = new QuantisationDetector();
    for (const p of [10_000, 11_000, 12_000, 13_000]) d.add(p);
    expect(d.granularityMs()).toBe(1000);
    for (const p of [14_123, 15_456, 16_789, 17_222, 18_333, 19_444, 20_555, 21_666]) d.add(p);
    expect(d.granularityMs()).toBe(0);
  });

  it('resets', () => {
    const d = new QuantisationDetector();
    for (const p of [10_000, 11_000, 12_000, 13_000]) d.add(p);
    d.reset();
    expect(d.granularityMs()).toBe(0);
  });
});

describe('PlaybackClock — quantisation awareness', () => {
  it('surfaces the detected granularity for diagnostics', () => {
    const c = new PlaybackClock();
    const timeline = new SyntheticTimeline({ reportJitterMs: 0 });
    for (const t of [0, 1000, 2000, 3000, 4000, 5000]) c.observe(timeline.observe(t));
    expect(c.detectedGranularityMs).toBe(1000);
  });

  it('widens uncertainty for a coarse source instead of claiming precision', () => {
    const coarse = new PlaybackClock();
    const timeline = new SyntheticTimeline({ reportJitterMs: 0 });
    for (const t of [0, 1000, 2000, 3000, 4000, 5000]) coarse.observe(timeline.observe(t));
    const coarseReading = coarse.read(5000);

    const precise = new PlaybackClock();
    for (const [i, p] of [10_123, 11_456, 12_789, 13_222, 14_655].entries()) {
      precise.observe(obs(p, i * 1000));
    }
    const preciseReading = precise.read(4000);

    expect(coarseReading.uncertaintyMs).toBeGreaterThan(preciseReading.uncertaintyMs);
    // Half a bucket of genuine ambiguity.
    expect(coarseReading.uncertaintyMs).toBeGreaterThanOrEqual(500);
  });

  it('stops trusting word-level sync on a whole-second source', () => {
    const c = new PlaybackClock();
    const timeline = new SyntheticTimeline({ reportJitterMs: 0 });
    for (const t of [0, 1000, 2000, 3000, 4000, 5000]) c.observe(timeline.observe(t));
    // Word highlighting at +/-500 ms would be visibly wrong, so it is de-emphasised.
    expect(shouldTrustWordSync(c.read(5000))).toBe(false);
  });

  it('leaves compensation off by default', () => {
    expect(DEFAULT_CLOCK_CONFIG.compensateQuantisation).toBe(false);
  });

  it('applies the half-bucket shift only when explicitly enabled', () => {
    const cfg = { ...DEFAULT_CLOCK_CONFIG, compensateQuantisation: true };
    const on = new PlaybackClock(cfg);
    const off = new PlaybackClock();
    const timeline = new SyntheticTimeline({ reportJitterMs: 0 });
    for (const t of [0, 1000, 2000, 3000, 4000, 5000]) {
      on.observe(timeline.observe(t));
      off.observe(timeline.observe(t));
    }
    expect(on.read(5000).positionMs - off.read(5000).positionMs).toBeCloseTo(500, 0);
  });

  it('does not report a bias change as a seek', () => {
    // Detection kicks in partway through; the basis must shift silently.
    const cfg = { ...DEFAULT_CLOCK_CONFIG, compensateQuantisation: true };
    const c = new PlaybackClock(cfg);
    const timeline = new SyntheticTimeline({ reportJitterMs: 0 });
    const kinds: string[] = [];
    for (const t of [0, 1000, 2000, 3000, 4000, 5000, 6000]) {
      kinds.push(c.observe(timeline.observe(t)));
    }
    expect(kinds.filter((k) => k === 'seek')).toHaveLength(0);
  });

  it('discards quantisation evidence across a track change', () => {
    const c = new PlaybackClock();
    const timeline = new SyntheticTimeline({ reportJitterMs: 0 });
    for (const t of [0, 1000, 2000, 3000, 4000]) c.observe(timeline.observe(t));
    expect(c.detectedGranularityMs).toBe(1000);

    c.observe({
      ...obs(0, 5000),
      title: 'Another Track',
      track: makeIdentity({
        provider: 'synthetic',
        title: 'Another Track',
        artist: 'A',
        durationMs: 200_000,
      }),
    });
    expect(c.detectedGranularityMs).toBe(0);
  });
});
